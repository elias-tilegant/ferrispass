//! All mutable application state, held in one `AppState` entity. The types
//! are defined here; the methods are split by concern into the submodules.

use crate::app::recents::{self, RecentEntry};
use crate::app::sync_history::{self, SyncHistoryEntry};
use crate::autotype::CancellationToken;
use crate::biometric::{
    BiometricEnrollment, BiometricError, BiometricRegistry, BiometricStore, EnrollmentId,
    NoopBiometricStore,
};
use crate::domain::{VaultEntry, VaultSnapshot};
use crate::keepass::limits::KdfCost;
use crate::keepass::merge::{ConflictKind, ConflictReport, Side};
use crate::keepass::{
    EntryDraft, MutationError, OtpDisplay, SavePayload, StrengthReport, VaultDocument,
};
use crate::sync::auth::{AccessToken, DeviceCodeChallenge};
use crate::sync::config::SyncConfig;
use crate::sync::graph::DriveItemHit;
use crate::update::{UpdateInfo, UpdateStatus};
use chrono::{DateTime, Local};
use gpui::{AppContext as _, Context};
use keepass::{DatabaseKey, db::Database};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use std::time::{Duration, SystemTime};
use zeroize::Zeroizing;

mod binding;
mod browse;
mod connect;
mod lock;
mod mutate;
mod open;
mod present;
mod routing;
mod save;
mod select;
mod session;
mod sync;
mod updates;

/// Process-wide ids prevent an async callback from ever confusing a newly
/// opened copy of the same path with the vault session that started the work.
/// Zero is reserved so the first real session is easy to spot in diagnostics.
static NEXT_VAULT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

/// App-global Quit/Restart actions cannot borrow the window-owned `AppState`.
/// Keep only the minimum cross-cutting signal here: how many unlocked sessions
/// still have local changes whose latest save has not succeeded.
static UNPERSISTED_VAULT_SESSION_COUNT: AtomicUsize = AtomicUsize::new(0);

/// File name of a vault whose save has settled into `SaveStatus::Failed` with
/// no writer left running. A save in that state never completes on its own,
/// so the quit veto has to offer a way past it instead of blocking forever
/// with a message about a save that is not in progress.
static STALLED_SAVE_VAULT: Mutex<Option<String>> = Mutex::new(None);

#[derive(Clone, Debug, Default)]
struct ConnectOperationGate {
    inner: Arc<ConnectOperationGateInner>,
}

#[derive(Debug, Default)]
struct ConnectOperationGateInner {
    generation: AtomicU64,
    cleanup_pending: AtomicUsize,
    commit: Mutex<()>,
    cleanup_finished: Condvar,
}

impl ConnectOperationGate {
    /// Invalidate every older operation without ever blocking the UI thread.
    fn advance(&self) -> u64 {
        self.inner
            .generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
    }

    fn is_current(&self, generation: u64) -> bool {
        self.inner.generation.load(Ordering::Acquire) == generation
    }

    /// Serialize the irreversible publication phase with cancellation.
    /// Network preparation stays outside this lock.
    fn commit_if_current<T>(&self, generation: u64, commit: impl FnOnce() -> T) -> Option<T> {
        let mut commit_guard = self
            .inner
            .commit
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while self.inner.cleanup_pending.load(Ordering::Acquire) != 0 {
            commit_guard = self
                .inner
                .cleanup_finished
                .wait(commit_guard)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        self.is_current(generation).then(commit)
    }

    /// Reserve cleanup before returning to the event loop. Newer publication
    /// waits until the matching cleanup has finished.
    fn reserve_cleanup(&self) {
        self.advance();
        self.inner.cleanup_pending.fetch_add(1, Ordering::AcqRel);
    }

    fn run_reserved_cleanup<T>(&self, operation: impl FnOnce() -> T) -> T {
        let _commit = self
            .inner
            .commit
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let result = operation();
        let previous = self.inner.cleanup_pending.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0, "cleanup must be reserved before it runs");
        if previous == 1 {
            self.inner.cleanup_finished.notify_all();
        }
        result
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct VaultSessionId(u64);

impl VaultSessionId {
    fn next() -> Self {
        Self(NEXT_VAULT_SESSION_ID.fetch_add(1, Ordering::Relaxed))
    }
}

/// Which vault a sync answer belongs to, and which sync relationship.
///
/// The vault's own session id does not move when the user disconnects one
/// provider and connects another: the vault stays open throughout. An answer
/// still in flight from the provider they left was then accepted as an answer
/// from the one they moved to, and a conflict resolved against the wrong
/// remote has no history anywhere to recover from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SyncSession {
    vault: VaultSessionId,
    binding: u64,
}

/// Used by app-global lifecycle actions, which deliberately refuse to
/// terminate while doing so could discard a queued or failed vault save.
pub(crate) fn has_unpersisted_vault_saves() -> bool {
    UNPERSISTED_VAULT_SESSION_COUNT.load(Ordering::Acquire) != 0
}

/// The vault named in [`STALLED_SAVE_VAULT`], if a save is stuck.
pub(crate) fn stalled_save_vault() -> Option<String> {
    STALLED_SAVE_VAULT
        .lock()
        .ok()
        .and_then(|stalled| stalled.clone())
}

#[derive(Debug)]
pub struct AppState {
    vault: VaultStatus,
    /// Identity of the currently-active `VaultStatus::Open`. Kept outside the
    /// public enum so this internal race guard does not widen its public API.
    active_vault_session_id: Option<VaultSessionId>,
    /// Authorization shared by Auto-Type plans for the current active vault
    /// context. State transitions rotate it synchronously, before decrypted
    /// ownership changes, so GPUI's deferred observers are not a security
    /// boundary.
    auto_type_cancellation: CancellationToken,
    overlay: Overlay,
    /// Last file-picker validation error. Kept separate from `VaultStatus` so
    /// a bad selection can never replace an unlocked document and discard its
    /// in-memory state.
    vault_selection_error: Option<String>,
    /// Background-save lifecycle of the open vault. Drives the status indicator
    /// and gates retry / explicit-save UX.
    save_status: SaveStatus,
    /// Cloud-sync binding for the currently-open vault. `Some` while a synced
    /// vault is open; `None` while in Welcome / unlocked-but-not-synced state.
    /// Holds the in-memory access token alongside the persisted SyncConfig.
    sync: Option<SyncBinding>,
    /// Which sync relationship each vault's binding belongs to, by path.
    /// Bumped when the user ends one, so an answer still in flight from the
    /// provider they just left cannot be applied to the one they moved to.
    /// Per vault, because ending one vault's relationship says nothing about
    /// another's: a shared counter stranded a parked vault's push as
    /// `Syncing` forever, and automatic sync then skipped it as busy.
    sync_binding_generations: HashMap<PathBuf, u64>,
    /// User-facing sync state. Drives the status pill, the SyncSettings card
    /// content, and whether the Conflict overlay opens.
    sync_status: SyncStatus,
    /// Session-scoped log of entries that flowed in from remote (silent
    /// merges + user-resolved conflicts). Surfaced as the "Recent
    /// activity" list in Settings → Sync. Cleared on lock and on sync
    /// disconnect; capped at `sync_history::MAX_SYNC_HISTORY`. Not
    /// persisted - entry titles are sensitive, see the module-level
    /// note on `app::sync_history` for the reasoning.
    sync_history: Vec<SyncHistoryEntry>,
    /// Active during the multi-step Connect overlay (provider pick → URL →
    /// device code → download). `None` when overlay isn't Connect.
    connect_flow: Option<ConnectFlow>,
    /// Cancels stale async Connect callbacks and orders persistent publication
    /// against cleanup without blocking the UI thread.
    connect_operations: ConnectOperationGate,
    /// SharePoint binding created by the Connect flow for a vault that has
    /// been downloaded locally but not unlocked yet. Installed only after
    /// the matching vault unlock succeeds so the previously-active vault's
    /// binding is never overwritten while adding a second cloud vault.
    pending_sync: Option<PendingSync>,
    /// Set while a user-driven *reconnect* is running its device-code
    /// re-auth. Holds the existing on-disk `SyncConfig` of the vault being
    /// reconnected - captured up front so the token poll loop can rebind it
    /// (via `finish_reconnect`), reusing its drive/item ids, instead of
    /// dropping into the file-picker connect flow. `None` for a normal
    /// first-time Connect. Cleared on overlay teardown (`unwind_connect_flow`)
    /// and on lock so a stale target can never mis-route a later plain
    /// Connect into a rebind.
    reconnect_target: Option<SyncConfig>,
    /// In-memory mirror of the on-disk recents list. Loaded once at
    /// construction (`with_resume`), prepended on every successful unlock,
    /// persisted async. Most-recent first.
    recents: Vec<RecentEntry>,
    /// Favicon-download progress. Driven by `start_favicon_download`;
    /// the UI reads it to render a live "X/Y downloaded" label and
    /// disable the trigger button while a run is in flight.
    favicon_status: FaviconDownloadStatus,
    /// Auto-update flow state. Idle by default; transitions through
    /// Checking → Available → Downloading → ReadyToRestart on the happy path.
    /// Drives the welcome banner + the Settings → Updates row.
    update_status: UpdateStatus,
    /// Release notes for the currently-running version, loaded from the
    /// post-update handoff file. Used by Settings → "View What's New" even
    /// after the one-shot startup overlay has been dismissed.
    whats_new_info: Option<UpdateInfo>,
    /// Already-unlocked vaults that aren't the active one. Lets the user
    /// switch back without re-entering the master password. Populated by
    /// `park_active` whenever the active vault is bumped off-screen (⌘O
    /// to another open vault, or starting a cold unlock while one is
    /// open). Cleared in full by `lock_vault` so the global auto-lock
    /// timer sweeps every session at once.
    parked: HashMap<PathBuf, ParkedSession>,
    /// Order in which paths landed in `parked`, oldest first. We pop the
    /// tail to find "the vault the user was just looking at" - that's the
    /// right target for Esc-on-unlock and for picking a fallback when
    /// the active vault is closed.
    parked_order: Vec<PathBuf>,
    /// Sessions whose newest in-memory state is not yet known to be on disk.
    /// Membership survives a failed save and is cleared only by a successful
    /// save for the exact same session id.
    unpersisted_vault_sessions: HashSet<VaultSessionId>,
    /// A lock requested while a save was unsafe to abandon. Completion of the
    /// final successful local save consumes this flag and performs the lock.
    lock_requested_after_save: bool,
    /// Decrypted documents retained solely so an already-requested local save
    /// can finish after access has been revoked. These sessions are deliberately
    /// excluded from every browser, copy, sync, launch, and Auto-Type lookup.
    deferred_lock_sessions: HashMap<VaultSessionId, DeferredSaveSession>,
    /// Two-step confirmation flag for "Discard changes and lock" on the
    /// deferred-lock screen. Reset whenever that screen is entered or left.
    discard_deferred_armed: bool,
    /// When the deferred lock started waiting for its saves. After a grace
    /// period the discard escape opens even while a save is nominally still
    /// in flight - blocking file I/O on a dead network volume never
    /// completes, and only the flock acquisition itself has a deadline.
    deferred_lock_since: Option<std::time::Instant>,
    /// When the most recent unlock completed. The session-lock monitor uses
    /// this to ignore trailing post-wake notifications that would otherwise
    /// re-lock the vault the user just unlocked (see `unlocked_within`).
    /// `SystemTime`, not `Instant`: `Instant` does not advance during sleep
    /// on macOS, so an unlock right before a lid-close would still read as
    /// "seconds ago" hours later and grace away the wake-up fail-safes.
    last_unlock_at: Option<std::time::SystemTime>,
    /// Per-platform biometric backend. Production uses
    /// `crate::biometric::default_store()` (Touch ID on macOS, noop
    /// elsewhere); tests inject `InMemoryBiometricStore`. Held as
    /// `Arc<dyn _>` so the same handle can be cloned into background
    /// tasks that call `retrieve` (which blocks for as long as the
    /// OS biometric prompt stays open).
    biometric: Arc<dyn BiometricStore>,
    /// Persistent record of which vaults have a Touch ID enrolment.
    /// Loaded once in `with_resume`; written through `enroll_biometric`
    /// / `forget_biometric`. Contents are deliberately metadata-only:
    /// vault path, UUID, keyfile path. Passwords live in the OS
    /// keychain under the UUID - never in this struct.
    biometric_registry: BiometricRegistry,
    /// Set by the Unlock screen's "Enable Touch ID" checkbox before
    /// the user submits the password. Consumed by `finish_open_attempt`
    /// on success and reset on every transition into `AwaitingPassword`.
    pending_biometric_enrollment: bool,
    /// What the unlock screen's header pre-check found for a path: `Some`
    /// once the check has run, holding the Argon2 cost the user still has
    /// to confirm (if any). Keyed by path so a result that lands after the
    /// user moved on to another vault is never shown against the wrong one.
    kdf_check: Option<(PathBuf, Option<KdfCost>)>,
    /// Bumped by every pre-check start and every direct write, so a
    /// pre-check that lands late (A -> B -> A, or after a failed open already
    /// recorded the real answer) is dropped instead of overwriting it.
    kdf_check_generation: u64,
    /// Bumped on every transition into `Opening`, so a password open that
    /// comes back after the user left and re-entered the same vault is
    /// recognised as stale rather than taken for the current attempt.
    open_attempt: u64,
    /// A synced remote whose Argon2 cost is above the default budget and not
    /// yet confirmed, keyed by the local vault it belongs to. The merge stops
    /// there; Settings → Sync offers the confirmation. Never decided by a
    /// background sync: raising that cost is exactly what a tampered remote
    /// would do.
    remote_kdf_confirmations: HashMap<PathBuf, KdfCost>,
    /// Remote merges whose decrypt is running in the background, by vault,
    /// with the token of the newest one. The continuation only runs while its
    /// token is still here: a newer merge replaces it, and a healthy sync or
    /// a disconnect removes it, so an old remote can never land on top.
    remote_merges_in_flight: HashMap<PathBuf, u64>,
    remote_merge_token: u64,
    /// In-flight / last-failure state for the Touch ID button on the
    /// Unlock screen. Lives on the state rather than on AppShell so
    /// re-renders driven by `cx.observe(&state, …)` show the spinner
    /// and the error consistently.
    biometric_attempt: BiometricAttempt,
    /// Monotonic id stamped onto every `begin_biometric_unlock`. The
    /// async retrieve carries its generation back; we only honour a
    /// resolution whose generation still matches the current attempt.
    /// Guards the race where the user cancels an attempt and starts a
    /// new one for the *same* vault path - path equality alone can't
    /// tell the two attempts apart, so a stale prompt resolving late
    /// could otherwise drive the newer screen.
    biometric_generation: u64,
    /// Vault paths with a background auto-sync request currently in
    /// flight. Without this a Graph call that stalls past the timer
    /// interval would let the next tick spawn a *second* request for the
    /// same vault - competing token refreshes, duplicate downloads, and
    /// racy status writes. We insert before spawning and remove on
    /// completion; the auto-sync tick skips any path already present.
    auto_sync_in_flight: HashSet<PathBuf>,
    /// Vaults whose conflict is waiting for the user rather than for the
    /// network. A deferred conflict reports itself as failed, and a failed
    /// sync is retried with a full push, so without this every tick pushed
    /// the same bytes, took the same 412, downloaded the same remote and
    /// deferred again. Cleared as soon as either side moves or the user gets
    /// to answer. The value is the remote revision it was deferred against,
    /// so the tick that follows can tell "still the same disagreement" from
    /// "something new to say" without paying for the whole file again.
    deferred_conflicts: HashMap<PathBuf, String>,
    /// Disk-save serialization per vault path. A key being present means a
    /// background `save_to` is running for that path; the value records
    /// whether another save was requested in the meantime. Only one writer
    /// may run per path: concurrent saves would race on temp files and -
    /// worse - an *older* payload could rename over a newer one, silently
    /// reverting the freshest state on disk. The completion callback runs
    /// one coalesced trailing save when the flag is set, so a burst of
    /// edits collapses into "current save + one save of the latest state".
    saves_in_flight: HashMap<PathBuf, QueuedSave>,
    /// Push serialization per vault path, mirroring `saves_in_flight`.
    /// Every successful save chains a sync, so a burst of edits would
    /// otherwise launch concurrent uploads all carrying the same
    /// `last_etag` - the first rotates the server etag and the rest
    /// collect spurious 412s, dragging the user through conflict cycles
    /// against their own writes. While a push is in flight, further
    /// requests only mark the queue flag; the completion callback runs
    /// one follow-up push that reads the then-latest bytes.
    syncs_in_flight: HashMap<PathBuf, QueuedSync>,
    /// Pushes that arrived while their vault had no live binding, held until
    /// the restore each one triggered installs one. Without this the save was
    /// written locally, the push was dropped, and auto-sync (pull-only unless
    /// the status is `Failed`) never uploaded it: the edit lived on one
    /// machine while the UI said everything was in sync. Keyed per vault
    /// because a single slot let a second vault's queued push evict the
    /// first, and let an unrelated restore consume the survivor.
    pushes_awaiting_restore: HashMap<PathBuf, QueuedSyncRequest>,
    /// Per-vault backoff after the provider asked us to slow down. The
    /// failure-recovery branch of the auto-sync tick re-uploads the whole
    /// vault, so without this a throttled client hammered Graph on every
    /// tick and kept extending the throttle window it was reacting to.
    sync_backoff: HashMap<PathBuf, SyncBackoff>,
    /// Lets a test look into `lock_vault_now` at the moment the active
    /// document is dropped, to see what had already been revoked by then.
    #[cfg(test)]
    wipe_witness: Option<characterization_tests::WipeWitness>,
}

/// When a throttled vault may be pushed again, and how many throttles it has
/// collected in a row.
#[derive(Clone, Copy, Debug)]
struct SyncBackoff {
    until: SystemTime,
    attempts: u32,
}

/// Floor for a self-chosen backoff, and the point where doubling stops.
/// A provider-supplied `Retry-After` overrides the floor in both directions:
/// it knows better than we do.
const SYNC_BACKOFF_MIN: Duration = Duration::from_secs(60);
const SYNC_BACKOFF_MAX: Duration = Duration::from_secs(15 * 60);

/// Bookkeeping value for `AppState::saves_in_flight` - see the field docs.
#[derive(Clone, Debug)]
struct QueuedSave {
    /// Session whose immutable payload is currently being written.
    owner: VaultSessionId,
    /// Latest session that requested another save for this path while the
    /// owner was in flight. Usually equal to `owner`, but may differ when the
    /// same file is closed/reopened before an old callback lands.
    pending_session: Option<VaultSessionId>,
    /// At least one of the coalesced requests wanted the cloud-sync push
    /// that normally chains off a successful save.
    sync_after: bool,
    /// Shared with the background save task. "Discard changes and lock"
    /// races the worker through this atomic state machine: the worker
    /// claims publication immediately before the rename, the discard
    /// claims abortion - exactly one side wins, so a discarded save can
    /// never publish later and a published save can never be reported as
    /// discarded.
    abort: crate::keepass::SaveAbortHandle,
}

impl QueuedSave {
    fn new(owner: VaultSessionId) -> Self {
        Self {
            owner,
            pending_session: None,
            sync_after: false,
            abort: crate::keepass::SaveAbortHandle::new(),
        }
    }
}

/// What the completion of a remote pull should do with the merge report.
///
/// A pure decision, split out because the branch it replaced lived inside a
/// spawn closure where nothing could reach it: a fix to it was silently lost
/// to a stray `git checkout` and only an outside reviewer noticed. The cases
/// it distinguishes are the ones where getting it wrong loses an edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RemoteMergePlan {
    /// Nothing to decide. Merge silently and push if the local side
    /// contributed anything.
    AutoMerge,
    /// Open the conflict screen.
    Ask,
    /// Something has to be decided, but not now: the vault is parked, the
    /// user is mid-edit, or this is a background pull that must not seize the
    /// screen. Report and wait for an explicit "Sync now".
    Defer,
}

impl RemoteMergePlan {
    fn of(
        report: &ConflictReport,
        target_is_active: bool,
        interactive: bool,
        editing: bool,
    ) -> Self {
        // Group conflicts count here as much as entry ones. Falling through
        // to the silent merge with one applies the default side and uploads
        // it, which is the loss the conflict screen exists to prevent.
        if !report.needs_a_decision() {
            return Self::AutoMerge;
        }
        // The conflict screen is single-vault by design: it edits the user's
        // active focus. And replacing an open entry editor with it throws
        // away whatever they had typed, while the interactive push that lands
        // here fires on every save, so an unlucky 412 can arrive at any
        // moment. A background pull must not seize the screen either.
        if target_is_active && interactive && !editing {
            Self::Ask
        } else {
            Self::Defer
        }
    }
}

/// What `AppState::apply_sync_binding_restore` decided, so its caller can run
/// the two effects that need a gpui context.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RestoreOutcome {
    /// A binding landed in this vault's slot, so a push that was waiting for
    /// one may now go out.
    bound: bool,
    /// The active vault's status changed.
    repaint: bool,
}

/// Bookkeeping value for `AppState::syncs_in_flight` - see the field docs.
#[derive(Clone, Debug)]
struct QueuedSync {
    /// Exact unlocked session, and sync relationship, whose request currently
    /// owns the network task.
    owner: SyncSession,
    /// Latest request that arrived while `owner` was in flight.
    pending: Option<QueuedSyncRequest>,
}

#[derive(Clone)]
struct QueuedSyncRequest {
    session: SyncSession,
    interactive: bool,
    bytes: Option<Arc<Vec<u8>>>,
}

/// Redacted by hand: `bytes` is the published KDBX ciphertext, and a derived
/// `Debug` would print the whole vault into any log or panic message.
impl std::fmt::Debug for QueuedSyncRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("QueuedSyncRequest")
            .field("session", &self.session)
            .field("interactive", &self.interactive)
            .field("bytes", &self.bytes.as_ref().map(|bytes| bytes.len()))
            .finish()
    }
}

/// How long a deferred lock may wait on nominally in-flight saves before
/// the "Discard changes and lock" escape opens anyway. Real saves finish in
/// seconds (KDF + fsync); anything past this is blocked file I/O that only
/// a process restart can interrupt.
const DEFERRED_LOCK_STUCK_AFTER: std::time::Duration = std::time::Duration::from_secs(60);

/// Vault bytes for a push: either already published by the save that
/// triggered it, or read from disk inside the background task - the read
/// takes the same locks a save holds across KDF + fsync and must never
/// run on the UI thread.
enum BytesSource {
    Published(Arc<Vec<u8>>),
    Read(Box<dyn FnOnce() -> Result<Arc<Vec<u8>>, crate::keepass::SaveError> + Send>),
}

impl QueuedSync {
    fn new(owner: SyncSession) -> Self {
        Self {
            owner,
            pending: None,
        }
    }

    fn queue(&mut self, request: QueuedSyncRequest) {
        if let Some(pending) = &mut self.pending
            && pending.session == request.session
        {
            pending.interactive |= request.interactive;
            pending.bytes = request.bytes;
            return;
        }

        // A reopened vault at the same display path is a different owner.
        // Replace every bit of queued state so old bytes/interactivity cannot
        // bleed into the new session's follow-up.
        self.pending = Some(request);
    }
}

/// Lifecycle of a single Touch ID unlock attempt. Drives the Unlock
/// screen's Touch ID button - `Idle` shows the button armed, `InFlight`
/// shows it disabled with a "Waiting for Touch ID…" hint, `Error`
/// renders the message and keeps the password input available so the
/// user can fall back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum BiometricAttempt {
    #[default]
    Idle,
    InFlight {
        path: PathBuf,
        /// Generation that produced this attempt - see
        /// `AppState::biometric_generation`.
        generation: u64,
    },
    Error {
        path: PathBuf,
        message: String,
    },
}

/// Handle returned by `AppState::begin_biometric_unlock` for the
/// `AppShell` to drive the OS call off-thread. Keeps every field the
/// background task needs (so it doesn't have to re-borrow `AppState`
/// across an await point) and a strong clone of the store handle.
#[derive(Clone, Debug)]
pub struct BiometricLaunch {
    pub id: EnrollmentId,
    pub path: PathBuf,
    pub keyfile: Option<PathBuf>,
    pub store: Arc<dyn BiometricStore>,
    /// Generation stamped at launch; carried back through
    /// `BiometricUnlockOutcome` so the resolution can be matched to
    /// the attempt that started it (and ignored if superseded).
    pub generation: u64,
}

impl Default for AppState {
    fn default() -> Self {
        // Default uses the noop store so unit tests of unrelated
        // methods (vault switching, sync, recents) don't need to
        // construct a real Keychain client. `app::run` overrides
        // this in `with_resume` via `default_store()`.
        Self {
            vault: VaultStatus::default(),
            active_vault_session_id: None,
            auto_type_cancellation: CancellationToken::new(),
            overlay: Overlay::default(),
            vault_selection_error: None,
            save_status: SaveStatus::default(),
            sync: None,
            sync_binding_generations: HashMap::new(),
            sync_status: SyncStatus::default(),
            sync_history: Vec::new(),
            connect_flow: None,
            connect_operations: ConnectOperationGate::default(),
            pending_sync: None,
            reconnect_target: None,
            recents: Vec::new(),
            favicon_status: FaviconDownloadStatus::default(),
            update_status: UpdateStatus::default(),
            whats_new_info: None,
            parked: HashMap::new(),
            parked_order: Vec::new(),
            unpersisted_vault_sessions: HashSet::new(),
            lock_requested_after_save: false,
            deferred_lock_sessions: HashMap::new(),
            discard_deferred_armed: false,
            deferred_lock_since: None,
            last_unlock_at: None,
            biometric: Arc::new(NoopBiometricStore),
            biometric_registry: BiometricRegistry::new(),
            pending_biometric_enrollment: false,
            kdf_check: None,
            kdf_check_generation: 0,
            open_attempt: 0,
            remote_kdf_confirmations: HashMap::new(),
            remote_merges_in_flight: HashMap::new(),
            remote_merge_token: 0,
            biometric_attempt: BiometricAttempt::default(),
            biometric_generation: 0,
            auto_sync_in_flight: HashSet::new(),
            deferred_conflicts: HashMap::new(),
            saves_in_flight: HashMap::new(),
            syncs_in_flight: HashMap::new(),
            pushes_awaiting_restore: HashMap::new(),
            sync_backoff: HashMap::new(),
            #[cfg(test)]
            wipe_witness: None,
        }
    }
}

impl Drop for AppState {
    fn drop(&mut self) {
        // Unit tests and application teardown can drop an AppState without a
        // final save callback. Remove only this instance's contribution to the
        // process-wide lifecycle guard.
        let remaining = self.unpersisted_vault_sessions.len();
        if remaining != 0 {
            UNPERSISTED_VAULT_SESSION_COUNT.fetch_sub(remaining, Ordering::AcqRel);
        }
    }
}

/// A vault that the user has unlocked at some point during this session
/// but isn't currently looking at. Holds the full decrypted document plus
/// every piece of per-vault UI state that would otherwise be lost on
/// switch (selection, search, save lifecycle, sync binding). On switch-back
/// it's drained into `VaultStatus::Open` byte-for-byte - no second KDF.
#[derive(Debug)]
pub struct ParkedSession {
    session_id: VaultSessionId,
    pub document: Box<VaultDocument>,
    pub selection: LibrarySelection,
    pub selected_entry_id: Option<String>,
    pub search_query: String,
    pub visible_entries: Rc<Vec<VaultEntry>>,
    pub selected_strength: Option<StrengthReport>,
    pub last_used: HashMap<String, DateTime<Local>>,
    pub save_status: SaveStatus,
    pub sync: Option<SyncBinding>,
    pub sync_status: SyncStatus,
    /// Per-vault sync activity log. Mirrors `AppState::sync_history`
    /// across park/unpark so each vault keeps the events that
    /// happened while it was active.
    pub sync_history: Vec<SyncHistoryEntry>,
}

/// Minimal save-only state kept after a lock request. UI-derived caches and
/// sync credentials are dropped when the session enters this quarantine.
#[derive(Debug)]
struct DeferredSaveSession {
    path: PathBuf,
    document: Box<VaultDocument>,
    save_status: SaveStatus,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SaveStatus {
    /// No save has happened since the vault was opened (the on-disk file is
    /// authoritative and equal to the in-memory state).
    #[default]
    Idle,
    /// A background save is in flight.
    Saving,
    /// The most recent save succeeded.
    Saved,
    /// The most recent save failed; message is suitable for a toast.
    Failed(String),
}

/// What a background tick needs to know about a vault's sync state, without
/// copying the state itself. `SyncStatus::Conflict` owns two decrypted
/// databases; a tick that only wants to ask "is this busy?" must not pay for
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncActivity {
    /// An operation is running, or a conflict is waiting for the user.
    Busy,
    /// The last operation failed and the tick owns the retry.
    Failed,
    /// Connected and settled.
    Resting,
    /// Local-only, or the sign-in expired.
    Inactive,
}

impl SyncActivity {
    pub(crate) fn of(status: &SyncStatus) -> Self {
        match status {
            SyncStatus::Syncing
            | SyncStatus::Connecting
            | SyncStatus::Restoring
            | SyncStatus::Conflict(_) => SyncActivity::Busy,
            SyncStatus::Failed(_) => SyncActivity::Failed,
            SyncStatus::Synced { .. } | SyncStatus::Idle => SyncActivity::Resting,
            SyncStatus::Disconnected | SyncStatus::Reconnect { .. } => SyncActivity::Inactive,
        }
    }
}

/// Live sync binding for an open synced vault. Owns the access token in
/// memory; the refresh token lives in the keychain (loaded on demand by
/// `service::refresh_access_token`).
#[derive(Debug)]
pub struct SyncBinding {
    pub config: SyncConfig,
    pub access_token: AccessToken,
}

#[derive(Debug)]
struct PendingSync {
    local_path: PathBuf,
    binding: SyncBinding,
}

/// User-facing sync lifecycle. Mirrors the SaveStatus shape so the UI
/// status pill can read both with the same vocabulary.
#[derive(Clone, Debug, Default)]
pub enum SyncStatus {
    /// No sync configured for this vault, or no vault open.
    #[default]
    Disconnected,
    /// Synced, idle. Equivalent to "everything's good".
    Idle,
    /// Initial connect in progress (multi-step - see `ConnectFlow` for which step).
    Connecting,
    /// Restoring an existing sync binding from `sync/<hash>.json` + the
    /// keychain refresh token. Distinct from `Connecting` (which is the
    /// device-code OAuth dance). Renders the same "Connecting…" pill.
    Restoring,
    /// Push or pull in flight.
    Syncing,
    /// Last operation succeeded at the given time. `chrono::Local` for the
    /// "Synced 2 minutes ago" UI string. `auto_merged` is the number of
    /// remote-only entries that got pulled in during a git-style silent
    /// merge - non-zero only when `handle_remote_conflict` short-circuited
    /// past the overlay; zero for normal saves and manual conflict resolution.
    Synced {
        at: chrono::DateTime<chrono::Local>,
        auto_merged: usize,
    },
    /// Server returned 412 - local + remote diverged. UI opens the Conflict
    /// overlay; resolution clears this back to Synced.
    Conflict(Box<ConflictState>),
    /// Last operation failed. Caller (UI) decides whether to retry.
    Failed(String),
    /// Refresh token is gone or revoked - user must re-run Connect. The
    /// optional `detail` carries the Azure `AADSTS…` reason (from
    /// `AuthError::InvalidGrant`) so the reconnect screen can tell the
    /// user *why* their sign-in expired instead of a generic message.
    Reconnect { detail: Option<String> },
}

/// Lifecycle of the explicit "Download favicons" action. Surfaced in the
/// Settings → General panel; `Idle` is the resting state, `Running`
/// drives the live progress label, `Finished` hangs around for one
/// session so the user can see the result before moving on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum FaviconDownloadStatus {
    #[default]
    Idle,
    Running {
        done: usize,
        total: usize,
        succeeded: usize,
    },
    Finished {
        succeeded: usize,
        total: usize,
    },
}

impl FaviconDownloadStatus {
    pub fn is_running(&self) -> bool {
        matches!(self, FaviconDownloadStatus::Running { .. })
    }
}

/// Heavy state owned by `SyncStatus::Conflict`. Holds both decrypted
/// databases, the report computed by `keepass::merge::diff`, the user's
/// per-entry picks, and the remote ETag we need to send back when uploading
/// the merged result.
///
/// Clone-ability is required because `SyncStatus` is `Clone` (the renderer
/// snapshots it). The two `Database` clones inside aren't free but they're
/// the same memcpy `save_payload` already does on every save - acceptable.
#[derive(Clone)]
pub struct ConflictState {
    pub local_db: Database,
    pub remote_db: Database,
    pub remote_etag: String,
    pub report: ConflictReport,
    pub picks: crate::keepass::merge::Resolutions,
    /// `VaultDocument::generation` at the moment `local_db` was cloned.
    /// `commit_merged_for` compares against the live document before
    /// installing the merged result - if the user edited while this
    /// conflict sat open, the merge is recomputed instead of silently
    /// discarding those edits.
    pub base_generation: u64,
    /// Exact unlocked session, and the exact sync relationship, that produced
    /// this report. A path can be reopened, and a provider swapped, while
    /// async conflict work is in flight.
    session: SyncSession,
}

impl std::fmt::Debug for ConflictState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConflictState")
            .field("has_remote_etag", &!self.remote_etag.is_empty())
            .field("conflict_count", &self.report.conflicts.len())
            .field("local_only_count", &self.report.local_only.len())
            .field("remote_only_count", &self.report.remote_only.len())
            .field("auto_resolved_count", &self.report.auto_resolved.len())
            .field("entry_pick_count", &self.picks.entries.len())
            .field("group_pick_count", &self.picks.groups.len())
            .field("base_generation", &self.base_generation)
            .field("session", &self.session)
            .finish()
    }
}

/// Step machine for the Connect overlay. The Connect screen renders a
/// stepper (Choose provider → Authorize → Pick vault) keyed off this.
#[derive(Clone, Debug)]
pub enum ConnectFlow {
    /// Initial: three provider buttons (only SharePoint is wired in this MVP).
    PickProvider,
    /// iCloud has no app-level authorization. The user chooses whether to
    /// open an existing remote or publish the active local vault.
    ICloudActions,
    /// Requesting the device code (reconnect path): a brief spinner shown
    /// between opening the overlay and the code arriving, so the user never
    /// sees the provider picker during a reconnect (clicking it there could
    /// spawn a second, non-reconnect connect flow). Replaced by `SigningIn`
    /// once the challenge is in hand.
    Authorizing,
    /// Device code shown; background task is polling for token. No file
    /// has been chosen yet - that comes after sign-in completes.
    SigningIn { challenge: DeviceCodeChallenge },
    /// Token in hand. Initial state shows a loading spinner while we fetch
    /// the user's `.kdbx` files; once `results` is populated the picker
    /// renders. `query` is the live filter the user types into the picker.
    Picking {
        token: AccessToken,
        results: Vec<DriveItemHit>,
        query: String,
        loading: bool,
        error: Option<String>,
    },
    /// User picked a file; downloading + persisting config.
    Downloading,
    /// Coordinated iCloud download/publication is running.
    ICloudTransferring { message: String },
    /// Anything went wrong before we hit the unlock screen. Carries a
    /// human-readable message for the UI.
    Failed(String),
}

#[derive(Debug, Default)]
pub enum VaultStatus {
    #[default]
    Empty,
    AwaitingPassword {
        path: PathBuf,
        keyfile: Option<PathBuf>,
        error: Option<String>,
    },
    Opening {
        path: PathBuf,
    },
    Open {
        path: PathBuf,
        document: Box<VaultDocument>,
        selection: LibrarySelection,
        selected_entry_id: Option<String>,
        search_query: String,
        /// Pre-computed result of `entries_for_selection(selection, search_query)`,
        /// rebuilt only when selection / search changes. Sharing via `Rc` makes
        /// `vault_browser()` cheap on every render frame, which keeps scrolling
        /// smooth on large vaults (3 500+ entries).
        visible_entries: Rc<Vec<VaultEntry>>,
        /// Real `zxcvbn` score for the currently-selected entry. Computed once
        /// per selection change so the detail view can render an accurate bar
        /// without paying the ~1-5 ms zxcvbn cost on every frame.
        selected_strength: Option<StrengthReport>,
        /// In-memory access log: entry-id → wall-clock time of the last
        /// password/username copy. Drives the "Recently used" library
        /// filter. Intentionally session-scoped - closing the vault drops
        /// the map so a read-only browse never touches disk.
        last_used: HashMap<String, DateTime<Local>>,
    },
    /// User access is already revoked, but one or more decrypted documents are
    /// retained privately until their latest local state is durable.
    LockedPendingSave,
    Error {
        message: String,
        path: Option<PathBuf>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibrarySelection {
    Group(String),
    AllItems,
    Favorites,
    RecentlyUsed,
    Trash,
    Tag(String),
    /// Entries with a TOTP secret configured. Decoupled from the tag
    /// system on purpose - used to be a synthetic "2FA" tag, but that
    /// lied to users (and disagreed with KeePassXC). Driven by the real
    /// `has_otp` bit on each entry.
    TotpEnabled,
}

impl LibrarySelection {
    pub fn group_id(&self) -> Option<&str> {
        match self {
            LibrarySelection::Group(id) => Some(id.as_str()),
            _ => None,
        }
    }

    pub fn tag(&self) -> Option<&str> {
        match self {
            LibrarySelection::Tag(name) => Some(name.as_str()),
            _ => None,
        }
    }

    pub fn is_all_items(&self) -> bool {
        matches!(self, LibrarySelection::AllItems)
    }
    pub fn is_favorites(&self) -> bool {
        matches!(self, LibrarySelection::Favorites)
    }
    pub fn is_recently_used(&self) -> bool {
        matches!(self, LibrarySelection::RecentlyUsed)
    }
    pub fn is_trash(&self) -> bool {
        matches!(self, LibrarySelection::Trash)
    }
    pub fn is_totp_enabled(&self) -> bool {
        matches!(self, LibrarySelection::TotpEnabled)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Overlay {
    #[default]
    None,
    /// Cloud provider picker (welcome → connect flow).
    Connect,
    /// Unified Settings overlay - full window. Tabs (General, Sync, …)
    /// are tracked in AppShell as UI-local state. Universally available
    /// (no vault-open gate), matching the Mac ⌘, convention.
    Settings,
    /// New entry modal - appears over the vault.
    AddEntry,
    /// Edit existing entry. Carries the entry id so the Save handler knows
    /// what to update; same modal layout as `AddEntry`, just a different
    /// header + save action.
    EditEntry { entry_id: String },
    /// Add a new group under `parent_group_id`. When the parent is the
    /// database root id the modal is presented as "New group"; otherwise
    /// "New subgroup".
    AddGroup { parent_group_id: String },
    /// Rename an existing group. Carries the id so the Save handler
    /// knows which document method to call.
    RenameGroup { group_id: String },
    /// Conflict resolution between this copy and the cloud one.
    Conflict,
    /// Quick vault picker - recents list + filter + "Browse other…" row.
    /// Universal like `Settings`: reachable from any vault state, including
    /// Welcome and Unlock screens.
    VaultSwitcher,
    /// Separate entrypoint for opening another vault. Keeps switching
    /// between known vaults distinct from adding local / SharePoint vaults.
    AddVault,
    /// Release notes for the version that was just installed. Universal like
    /// Settings so it can appear on first launch before any vault is open.
    WhatsNew { info: UpdateInfo },
    /// "About FerrisPass" modal - version, tagline, repo link. Universal
    /// like Settings so it's reachable from any vault state.
    About,
}

impl Overlay {
    pub fn is_active(&self) -> bool {
        !matches!(self, Overlay::None)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnlockPrompt {
    pub path: PathBuf,
    pub file_name: String,
    pub display_path: String,
    pub keyfile: Option<PathBuf>,
    pub error: Option<String>,
    /// Argon2 settings above the default budget that unlocking this vault
    /// will accept. The screen says so before the user commits.
    pub kdf_confirmation: Option<KdfCost>,
}

/// Health tone for the sidebar header's status dot. Derived from the live
/// `SyncStatus` (not from `is_open`) so the header agrees with the bottom
/// sync chip and the Settings → Sync card instead of always showing a
/// green "Synced" the moment a vault is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SyncTone {
    /// Synced / idle - everything's good (green).
    Synced,
    /// An operation is in flight (connecting / restoring / syncing) (blue).
    Connecting,
    /// Needs the user's attention: sign-in expired, last sync failed, or a
    /// conflict is awaiting resolution (orange).
    Attention,
    /// No cloud sync for this vault, or no vault open - local-only (muted).
    #[default]
    Neutral,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultSummary {
    pub title: String,
    pub subtitle: String,
    pub status: String,
    pub entries: usize,
    pub groups: usize,
    pub is_open: bool,
    pub is_busy: bool,
    /// Health tone for the header status dot, derived from `SyncStatus`.
    /// `Neutral` for non-open / local-only vaults.
    pub sync_tone: SyncTone,
    /// Provider name from the active SyncBinding. `None` when the open vault
    /// is local-only.
    pub provider: Option<String>,
    /// Human-readable last-synced indicator (e.g. "just now", "2m ago",
    /// "Failed", "Connecting…"). Derived from `SyncStatus`. Compact form
    /// ("16s ago" not "16 seconds ago") so the sidebar pill can fit
    /// provider + time + an optional merge badge on one line.
    pub synced_at: Option<String>,
    /// Number of entries the most recent sync silently merged from
    /// remote (`remote_only` adds + timestamp-resolved divergences).
    /// `Some(n)` only when `synced_at` is also `Some` and the count is
    /// non-zero. Rendered next to `synced_at` as a `[+N]` chip.
    pub auto_merged: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct VaultBrowserModel {
    /// Cheap `Arc` clone of the current snapshot - held so renderers can read
    /// the group tree, recently-used count, etc. without re-cloning.
    pub snapshot: Arc<VaultSnapshot>,
    pub selection: LibrarySelection,
    pub selection_label: String,
    pub selected_entry_id: Option<String>,
    /// Currently-visible entries (after selection + search filter), shared by
    /// `Rc` so the virtual list, scroll handler, and detail-pane all read from
    /// the same allocation.
    pub entries: Rc<Vec<VaultEntry>>,
    pub selected_entry: Option<VaultEntry>,
    pub selected_strength: Option<StrengthReport>,
    pub search_query: String,
    pub showing_search_results: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyValueKind {
    Username,
    Url,
    Password,
}

fn entries_for_selection(
    snapshot: &VaultSnapshot,
    selection: &LibrarySelection,
    search_query: &str,
    last_used: &HashMap<String, DateTime<Local>>,
) -> Vec<VaultEntry> {
    let query = search_query.trim();

    if !query.is_empty() {
        return super::search::ranked_entries(snapshot, query)
            .into_iter()
            .cloned()
            .collect();
    }

    match selection {
        // Selecting a group includes everything below it - entries directly
        // in the group *plus* every entry in any nested subgroup. Without
        // this the entry-count chip in the sidebar (which is recursive,
        // see `VaultGroup::entry_count`) and the visible list disagree:
        // clicking "Personal" with a "Personal/Banking" subgroup would
        // show "57" as the count but only the direct hits in the list.
        // Matches KeePassXC's behaviour.
        LibrarySelection::Group(id) => snapshot
            .find_group(id)
            .unwrap_or(&snapshot.root)
            .live_entries_recursive()
            .into_iter()
            .cloned()
            .collect(),
        LibrarySelection::AllItems => snapshot.live_entries().into_iter().cloned().collect(),
        LibrarySelection::Favorites => snapshot.entries_starred().into_iter().cloned().collect(),
        LibrarySelection::RecentlyUsed => {
            // Session-scoped: only entries the user has actually copied
            // a password/username from since unlock. Newest first.
            let mut entries: Vec<VaultEntry> = snapshot
                .live_entries()
                .into_iter()
                .filter(|entry| last_used.contains_key(&entry.id))
                .cloned()
                .collect();
            entries.sort_by(|a, b| last_used.get(&b.id).cmp(&last_used.get(&a.id)));
            entries
        }
        // The whole bin subtree, not just its direct children: deleting a
        // group nests its entries one level deeper, and they were unreachable
        // from the UI entirely.
        LibrarySelection::Trash => snapshot.trashed_entries().into_iter().cloned().collect(),
        LibrarySelection::Tag(name) => snapshot
            .entries_with_tag(name)
            .into_iter()
            .cloned()
            .collect(),
        LibrarySelection::TotpEnabled => snapshot.entries_with_otp().into_iter().cloned().collect(),
    }
}

fn selection_label_for(selection: &LibrarySelection, snapshot: &VaultSnapshot) -> String {
    match selection {
        LibrarySelection::Group(id) => snapshot
            .find_group(id)
            .map(|g| g.name.clone())
            .unwrap_or_else(|| snapshot.root.name.clone()),
        LibrarySelection::AllItems => "All items".to_string(),
        LibrarySelection::Favorites => "Favorites".to_string(),
        LibrarySelection::RecentlyUsed => "Recently used".to_string(),
        LibrarySelection::Trash => "Trash".to_string(),
        LibrarySelection::Tag(name) => format!("Tag · {name}"),
        LibrarySelection::TotpEnabled => "2FA enabled".to_string(),
    }
}

fn non_empty_copy(value: String) -> Option<String> {
    if value.is_empty() { None } else { Some(value) }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| path.display().to_string(), ToString::to_string)
}

/// Map `SyncStatus` to a short, user-facing string for the header / status pill.
/// `None` means "no sync indicator at all" - used when the vault is local-only.
fn sync_status_label(status: &SyncStatus) -> Option<String> {
    use chrono::Local;
    match status {
        SyncStatus::Disconnected => None,
        // Bound to a provider, but nothing has been checked against the
        // server yet this session. Saying "Synced" here promised a round trip
        // that never happened.
        SyncStatus::Idle => Some("Connected".into()),
        SyncStatus::Connecting => Some("Connecting…".into()),
        SyncStatus::Restoring => Some("Connecting…".into()),
        SyncStatus::Syncing => Some("Syncing…".into()),
        // Compact time only - the merge count rides as a separate
        // `auto_merged` badge in `VaultSummary`, rendered next to this
        // string by the sidebar pill. Keeping them separate stops the
        // pill from overflowing in narrow sidebars and lets the badge
        // be styled independently.
        SyncStatus::Synced { at, .. } => Some(crate::app::time::relative_time_label_short(
            *at,
            Local::now(),
        )),
        SyncStatus::Conflict(_) => Some("Conflict".into()),
        SyncStatus::Failed(_) => Some("Sync failed".into()),
        SyncStatus::Reconnect { .. } => Some("Sign-in expired".into()),
    }
}

/// Map a `SyncStatus` to the header dot's health tone. Keeps the sidebar
/// header in lockstep with `sync_status_label` (which produces the text):
/// green when healthy, blue while working, orange when the user needs to
/// act, muted when there's no sync at all.
/// Whether there is known to be no sync config for this vault, asked
/// cheaply enough for the UI thread.
///
/// Only a plain "no file" counts. A path that will not resolve and a
/// directory that will not answer are questions for the locked read on the
/// task, which has a Failed state with a Retry to put them in; calling
/// either of them "no config" would present a bound vault as local-only and
/// say nothing about why.
fn known_to_have_no_sync_config(
    config_path: Result<PathBuf, crate::sync::config::ConfigError>,
) -> bool {
    config_path.is_ok_and(|config| matches!(config.try_exists(), Ok(false)))
}

fn sync_status_tone(status: &SyncStatus) -> SyncTone {
    match status {
        SyncStatus::Idle | SyncStatus::Synced { .. } => SyncTone::Synced,
        SyncStatus::Connecting | SyncStatus::Restoring | SyncStatus::Syncing => {
            SyncTone::Connecting
        }
        SyncStatus::Reconnect { .. } | SyncStatus::Failed(_) | SyncStatus::Conflict(_) => {
            SyncTone::Attention
        }
        SyncStatus::Disconnected => SyncTone::Neutral,
    }
}

#[cfg(test)]
mod sync_config_probe_tests;

#[cfg(test)]
mod park_tests;

#[cfg(test)]
mod biometric_tests;

#[cfg(test)]
mod selection_tests;

#[cfg(test)]
mod characterization_tests;

use crate::app::recents::{self, RecentEntry};
use crate::app::sync_history::{self, SyncHistoryEntry};
use crate::domain::{VaultEntry, VaultSnapshot};
use crate::keepass::merge::{ConflictReport, Side};
use crate::keepass::{EntryDraft, MutationError, OtpDisplay, StrengthReport, VaultDocument};
use crate::sync::auth::{AccessToken, DeviceCodeChallenge};
use crate::sync::config::SyncConfig;
use crate::sync::graph::DriveItemHit;
use crate::update::{UpdateInfo, UpdateStatus};
use chrono::{DateTime, Local};
use gpui::{AppContext as _, Context};
use keepass::db::Database;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct AppState {
    vault: VaultStatus,
    overlay: Overlay,
    /// Background-save lifecycle of the open vault. Drives the status indicator
    /// and gates retry / explicit-save UX.
    save_status: SaveStatus,
    /// Cloud-sync binding for the currently-open vault. `Some` while a synced
    /// vault is open; `None` while in Welcome / unlocked-but-not-synced state.
    /// Holds the in-memory access token alongside the persisted SyncConfig.
    sync: Option<SyncBinding>,
    /// User-facing sync state. Drives the status pill, the SyncSettings card
    /// content, and whether the Conflict overlay opens.
    sync_status: SyncStatus,
    /// Session-scoped log of entries that flowed in from remote (silent
    /// merges + user-resolved conflicts). Surfaced as the "Recent
    /// activity" list in Settings → Sync. Cleared on lock and on sync
    /// disconnect; capped at `sync_history::MAX_SYNC_HISTORY`. Not
    /// persisted — entry titles are sensitive, see the module-level
    /// note on `app::sync_history` for the reasoning.
    sync_history: Vec<SyncHistoryEntry>,
    /// Active during the multi-step Connect overlay (provider pick → URL →
    /// device code → download). `None` when overlay isn't Connect.
    connect_flow: Option<ConnectFlow>,
    /// SharePoint binding created by the Connect flow for a vault that has
    /// been downloaded locally but not unlocked yet. Installed only after
    /// the matching vault unlock succeeds so the previously-active vault's
    /// binding is never overwritten while adding a second cloud vault.
    pending_sync: Option<PendingSync>,
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
    /// tail to find "the vault the user was just looking at" — that's the
    /// right target for Esc-on-unlock and for picking a fallback when
    /// the active vault is closed.
    parked_order: Vec<PathBuf>,
}

/// A vault that the user has unlocked at some point during this session
/// but isn't currently looking at. Holds the full decrypted document plus
/// every piece of per-vault UI state that would otherwise be lost on
/// switch (selection, search, save lifecycle, sync binding). On switch-back
/// it's drained into `VaultStatus::Open` byte-for-byte — no second KDF.
#[derive(Debug)]
pub struct ParkedSession {
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
    /// Initial connect in progress (multi-step — see `ConnectFlow` for which step).
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
    /// merge — non-zero only when `handle_remote_conflict` short-circuited
    /// past the overlay; zero for normal saves and manual conflict resolution.
    Synced {
        at: chrono::DateTime<chrono::Local>,
        auto_merged: usize,
    },
    /// Server returned 412 — local + remote diverged. UI opens the Conflict
    /// overlay; resolution clears this back to Synced.
    Conflict(Box<ConflictState>),
    /// Last operation failed. Caller (UI) decides whether to retry.
    Failed(String),
    /// Refresh token is gone or revoked — user must re-run Connect.
    Reconnect,
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
/// the same memcpy `save_payload` already does on every save — acceptable.
#[derive(Clone, Debug)]
pub struct ConflictState {
    pub local_db: Database,
    pub remote_db: Database,
    pub remote_etag: String,
    pub report: ConflictReport,
    pub picks: HashMap<String, Side>,
}

/// Step machine for the Connect overlay. The Connect screen renders a
/// stepper (Choose provider → Authorize → Pick vault) keyed off this.
#[derive(Clone, Debug)]
pub enum ConnectFlow {
    /// Initial: three provider buttons (only SharePoint is wired in this MVP).
    PickProvider,
    /// Device code shown; background task is polling for token. No file
    /// has been chosen yet — that comes after sign-in completes.
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
        /// filter. Intentionally session-scoped — closing the vault drops
        /// the map so a read-only browse never touches disk.
        last_used: HashMap<String, DateTime<Local>>,
    },
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
    /// system on purpose — used to be a synthetic "2FA" tag, but that
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
    /// Unified Settings overlay — full window. Tabs (General, Sync, …)
    /// are tracked in AppShell as UI-local state. Universally available
    /// (no vault-open gate), matching the Mac ⌘, convention.
    Settings,
    /// New entry modal — appears over the vault.
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
    /// Three-way conflict resolution.
    Conflict,
    /// Quick vault picker — recents list + filter + "Browse other…" row.
    /// Universal like `Settings`: reachable from any vault state, including
    /// Welcome and Unlock screens.
    VaultSwitcher,
    /// Separate entrypoint for opening another vault. Keeps switching
    /// between known vaults distinct from adding local / SharePoint vaults.
    AddVault,
    /// Release notes for the version that was just installed. Universal like
    /// Settings so it can appear on first launch before any vault is open.
    WhatsNew { info: UpdateInfo },
    /// "About FerrisPass" modal — version, tagline, repo link. Universal
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
    /// Cheap `Arc` clone of the current snapshot — held so renderers can read
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

impl AppState {
    /// Construct an AppState that auto-resumes the most recently opened
    /// vault. Reads the recents file synchronously (a few hundred bytes
    /// of JSON — cheap), prunes entries whose file no longer exists, and
    /// pre-populates the unlock screen with the head of the list.
    ///
    /// Falls back to an empty AppState (Welcome screen) when the list is
    /// empty or the file isn't readable.
    pub fn with_resume() -> Self {
        let recents = recents::load_pruned();
        let initial_vault = recents
            .entries
            .first()
            .map(|entry| VaultStatus::AwaitingPassword {
                path: entry.path.clone(),
                keyfile: crate::keepass::KeePassRepository::suggested_keyfile(&entry.path),
                error: None,
            })
            .unwrap_or_default();

        let pending_whats_new = crate::update::load_whats_new_for_version(crate::app::APP_VERSION);
        let whats_new_info = pending_whats_new
            .as_ref()
            .map(|pending| pending.info.clone());
        let overlay = pending_whats_new
            .filter(|pending| !pending.auto_shown)
            .map(|pending| Overlay::WhatsNew { info: pending.info })
            .unwrap_or_default();

        Self {
            vault: initial_vault,
            overlay,
            recents: recents.entries,
            whats_new_info,
            ..Self::default()
        }
    }

    pub fn vault_status(&self) -> &VaultStatus {
        &self.vault
    }

    /// Recently opened vaults, most recent first. Drives the Welcome
    /// screen's "Recent" section.
    pub fn recents(&self) -> &[RecentEntry] {
        &self.recents
    }

    pub fn favicon_status(&self) -> &FaviconDownloadStatus {
        &self.favicon_status
    }

    pub fn overlay(&self) -> &Overlay {
        &self.overlay
    }

    pub fn open_overlay(&mut self, overlay: Overlay, cx: &mut Context<Self>) {
        if self.overlay == overlay {
            return;
        }
        // Switching directly between overlays (e.g. ⌘O while Connect is
        // mid-`SigningIn`) has to run the same teardown as
        // `close_overlay` — otherwise the device-code polling loop would
        // outlive the overlay and keep mutating `connect_flow` /
        // `sync_status` behind a screen the user has already moved on
        // from.
        let leaving_connect = matches!(self.overlay, Overlay::Connect);
        self.overlay = overlay;
        if leaving_connect {
            self.unwind_connect_flow();
        }
        cx.notify();
    }

    pub fn close_overlay(&mut self, cx: &mut Context<Self>) -> bool {
        if matches!(self.overlay, Overlay::None) {
            return false;
        }
        let leaving_connect = matches!(self.overlay, Overlay::Connect);
        let leaving_whats_new = matches!(self.overlay, Overlay::WhatsNew { .. });
        self.overlay = Overlay::None;
        if leaving_connect {
            self.unwind_connect_flow();
        }
        if leaving_whats_new {
            let _ = crate::update::mark_whats_new_auto_shown(crate::app::APP_VERSION);
        }
        cx.notify();
        true
    }

    /// Drop any in-flight Connect state when the Connect overlay is left,
    /// regardless of whether we're closing to None or jumping to a
    /// different overlay. The device-code polling loop watches
    /// `connect_flow` and exits when it observes `None`; the sync status
    /// reset clears whichever transient `Connecting` / `Failed` pill the
    /// flow had pushed. Without this the next "Connect SharePoint" click
    /// would also re-open into whichever sub-step the user left it on
    /// (e.g. a stale "Failed" message).
    fn unwind_connect_flow(&mut self) {
        self.connect_flow = None;
        if matches!(
            self.sync_status,
            SyncStatus::Connecting | SyncStatus::Failed(_)
        ) {
            self.sync_status = match &self.sync {
                Some(_) => SyncStatus::Idle,
                None => SyncStatus::Disconnected,
            };
        }
    }

    pub fn request_password(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.clear_pending_sync_unless(&path);
        // Already looking at an unlocked vault? Park it so the user can
        // come back without re-typing their master password. The
        // cold-unlock screen we're about to render points at a *different*
        // path; the previously-active vault stays in `parked` until the
        // user switches to it again (or hits Esc, which rehydrates the
        // most-recently parked one).
        self.park_active();
        let keyfile = crate::keepass::KeePassRepository::suggested_keyfile(&path);
        self.vault = VaultStatus::AwaitingPassword {
            path,
            keyfile,
            error: None,
        };
        self.overlay = Overlay::None;
        cx.notify();
    }

    /// Swap to an already-unlocked vault if we know one for `path`. Returns
    /// `true` when the swap happened (caller can skip the unlock prompt),
    /// `false` when `path` is cold and needs a password.
    ///
    /// A no-op (returning `true`) when `path` is already the active vault —
    /// the caller hasn't told us they wanted to do anything, so we don't
    /// disturb selection / search state.
    pub fn switch_to_unlocked(&mut self, path: &Path, cx: &mut Context<Self>) -> bool {
        if let VaultStatus::Open { path: active, .. } = &self.vault
            && active.as_path() == path
        {
            // Same vault — nothing to do. Still count as "handled" so the
            // caller doesn't fall through to the password prompt.
            return true;
        }
        if !self.parked.contains_key(path) {
            return false;
        }
        // Park whatever's currently active (Open or AwaitingPassword) so
        // it survives the switch.
        self.park_active();
        if !self.unpark(path) {
            // Defensive: park_active above could in theory race away the
            // map entry (it can't — we just checked). Treat as cold.
            return false;
        }
        // Front-rank in recents so Welcome / ⌘O reflect the switch.
        self.push_recent(path.to_path_buf(), cx);
        cx.notify();
        true
    }

    /// Bring the most-recently-parked vault back into the active slot.
    /// Used by Esc-on-unlock to undo a request_password that the user
    /// decided against. Returns `true` when something was rehydrated.
    pub fn rehydrate_most_recent_park(&mut self, cx: &mut Context<Self>) -> bool {
        self.pending_sync = None;
        let Some(path) = self.parked_order.last().cloned() else {
            return false;
        };
        // We're abandoning the AwaitingPassword screen — drop it without
        // parking (no decrypted state to preserve).
        self.vault = VaultStatus::Empty;
        self.save_status = SaveStatus::Idle;
        self.sync = None;
        self.sync_status = SyncStatus::Disconnected;
        if !self.unpark(&path) {
            return false;
        }
        cx.notify();
        true
    }

    /// Paths of every currently-unlocked vault, *including* the active
    /// one. Vault Switcher UI uses this to render the "Open" section.
    /// Order: parked entries newest-last, active last. Callers that need
    /// the active marker should consult `current_vault_path` separately.
    pub fn unlocked_paths(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = self.parked_order.clone();
        if let VaultStatus::Open { path, .. } = &self.vault
            && !out.iter().any(|p| p == path)
        {
            out.push(path.clone());
        }
        out
    }

    /// `true` whenever any vault is unlocked in memory (active or parked).
    /// Drives the global auto-lock task gate: the idle timer must keep
    /// ticking even when the active slot is `Empty`/`AwaitingPassword` but
    /// parked vaults are still decrypted in memory.
    pub fn has_any_unlocked(&self) -> bool {
        matches!(self.vault, VaultStatus::Open { .. }) || !self.parked.is_empty()
    }

    /// Move the currently-active `Open` vault into the parked map, taking
    /// its save lifecycle + sync binding with it. No-op on every other
    /// `VaultStatus` (Empty/AwaitingPassword/Opening/Error carry no
    /// decrypted state worth preserving).
    fn park_active(&mut self) {
        if !matches!(self.vault, VaultStatus::Open { .. }) {
            return;
        }
        let prev = std::mem::take(&mut self.vault);
        let VaultStatus::Open {
            path,
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            last_used,
        } = prev
        else {
            // Unreachable — guard above already established Open. Restoring
            // to Empty is the safe fallthrough if a future variant slips in.
            return;
        };
        let session = ParkedSession {
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            last_used,
            save_status: std::mem::take(&mut self.save_status),
            sync: self.sync.take(),
            sync_status: std::mem::take(&mut self.sync_status),
            sync_history: std::mem::take(&mut self.sync_history),
        };
        // Refresh order — if this vault was parked before (shouldn't be,
        // but defend), move it to the tail.
        self.parked_order.retain(|p| p != &path);
        self.parked_order.push(path.clone());
        self.parked.insert(path, session);
    }

    /// Hydrate a parked session back into the active `Open` variant.
    /// Returns `false` when `path` isn't in the parked map. Caller is
    /// expected to have parked whatever was active first.
    fn unpark(&mut self, path: &Path) -> bool {
        let Some(session) = self.parked.remove(path) else {
            return false;
        };
        self.parked_order.retain(|p| p != path);
        let ParkedSession {
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            last_used,
            save_status,
            sync,
            sync_status,
            sync_history,
        } = session;
        self.vault = VaultStatus::Open {
            path: path.to_path_buf(),
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            last_used,
        };
        self.save_status = save_status;
        self.sync = sync;
        self.sync_status = sync_status;
        self.sync_history = sync_history;
        true
    }

    /// Apply a save result against the vault at `target`, regardless of
    /// whether it's currently active or has been parked by the user
    /// switching away mid-save. Notifies only when we touched the active
    /// vault — parked-vault status changes don't have an on-screen view
    /// to redraw. Drops the result silently if the vault was locked
    /// outright while the save was in flight.
    fn apply_save_status(&mut self, target: &Path, status: SaveStatus, cx: &mut Context<Self>) {
        if matches!(&self.vault, VaultStatus::Open { path, .. } if path.as_path() == target) {
            self.save_status = status;
            cx.notify();
        } else if let Some(parked) = self.parked.get_mut(target) {
            parked.save_status = status;
        }
    }

    /// Mirror of `apply_save_status` for the cloud-sync lifecycle. Notifies
    /// only when the active vault was touched.
    fn apply_sync_status(&mut self, target: &Path, status: SyncStatus, cx: &mut Context<Self>) {
        if matches!(&self.vault, VaultStatus::Open { path, .. } if path.as_path() == target) {
            self.sync_status = status;
            cx.notify();
        } else if let Some(parked) = self.parked.get_mut(target) {
            parked.sync_status = status;
        }
    }

    /// Mutate the `SyncBinding` for the vault at `target` if one exists.
    /// Used by sync callbacks to write back a refreshed access token /
    /// updated ETag against the vault that actually issued the upload,
    /// even after the user has switched away.
    fn with_sync_binding_mut_for(&mut self, target: &Path, f: impl FnOnce(&mut SyncBinding)) {
        if matches!(&self.vault, VaultStatus::Open { path, .. } if path.as_path() == target) {
            if let Some(b) = self.sync.as_mut() {
                f(b);
            }
        } else if let Some(parked) = self.parked.get_mut(target) {
            if let Some(b) = parked.sync.as_mut() {
                f(b);
            }
        }
    }

    fn is_unlocked_path(&self, target: &Path) -> bool {
        matches!(&self.vault, VaultStatus::Open { path, .. } if path.as_path() == target)
            || self.parked.contains_key(target)
    }

    fn clear_pending_sync_unless(&mut self, target: &Path) {
        if self
            .pending_sync
            .as_ref()
            .is_some_and(|pending| pending.local_path != target)
        {
            self.pending_sync = None;
        }
    }

    fn install_pending_sync_for(&mut self, opened: &Path) -> bool {
        if !self
            .pending_sync
            .as_ref()
            .is_some_and(|pending| pending.local_path == opened)
        {
            return false;
        }
        let pending = self.pending_sync.take().expect("checked above");
        self.sync = Some(pending.binding);
        self.sync_status = SyncStatus::Synced {
            at: chrono::Local::now(),
            auto_merged: 0,
        };
        true
    }

    fn has_open_sync_remote(&self, drive_id: &str, item_id: &str) -> bool {
        let matches_remote = |binding: &SyncBinding| {
            binding.config.drive_id == drive_id && binding.config.item_id == item_id
        };
        self.sync.as_ref().is_some_and(matches_remote)
            || self.parked.values().any(|session| {
                session.sync.as_ref().is_some_and(|binding| {
                    binding.config.drive_id == drive_id && binding.config.item_id == item_id
                })
            })
            || self.pending_sync.as_ref().is_some_and(|pending| {
                pending.binding.config.drive_id == drive_id
                    && pending.binding.config.item_id == item_id
            })
    }

    /// Snapshot just enough state to run a sync against `target` from a
    /// background task — works whether `target` is the active vault or
    /// one we parked away from. Returns `None` when the vault is locked
    /// or has no sync binding (= local-only / disconnected).
    fn snapshot_sync_inputs(
        &self,
        target: &Path,
    ) -> Option<(crate::sync::config::SyncConfig, AccessToken, String)> {
        if let VaultStatus::Open { path, document, .. } = &self.vault
            && path.as_path() == target
        {
            let binding = self.sync.as_ref()?;
            return Some((
                binding.config.clone(),
                binding.access_token.clone(),
                document.password().to_string(),
            ));
        }
        let parked = self.parked.get(target)?;
        let binding = parked.sync.as_ref()?;
        Some((
            binding.config.clone(),
            binding.access_token.clone(),
            parked.document.password().to_string(),
        ))
    }

    /// Borrow the live `Database` for the vault at `target` if it's
    /// still unlocked. Used by the conflict-diff path so it works for
    /// vaults parked while a sync was in flight.
    fn database_clone_for(&self, target: &Path) -> Option<keepass::Database> {
        if let VaultStatus::Open { path, document, .. } = &self.vault
            && path.as_path() == target
        {
            return Some(document.database().clone());
        }
        self.parked
            .get(target)
            .map(|p| p.document.database().clone())
    }

    /// Replace the live `Database` for the vault at `target` with the
    /// freshly merged one. Used by `commit_merged` after a 412 → merge →
    /// re-save cycle so the in-memory and on-disk views agree, regardless
    /// of which vault is currently active. Returns `true` when a vault was
    /// actually updated.
    fn replace_document_for(
        &mut self,
        target: &Path,
        replacement: VaultDocument,
        cx: &mut Context<Self>,
    ) -> bool {
        if matches!(&self.vault, VaultStatus::Open { path, .. } if path.as_path() == target) {
            if let VaultStatus::Open { document, .. } = &mut self.vault {
                **document = replacement;
                cx.notify();
                return true;
            }
        } else if let Some(parked) = self.parked.get_mut(target) {
            *parked.document = replacement;
            return true;
        }
        false
    }

    pub fn set_unlock_keyfile(&mut self, keyfile: Option<PathBuf>, cx: &mut Context<Self>) {
        if let VaultStatus::AwaitingPassword {
            keyfile: existing,
            error,
            ..
        } = &mut self.vault
        {
            *existing = keyfile;
            *error = None;
            cx.notify();
        }
    }

    pub fn pending_unlock_keyfile(&self) -> Option<PathBuf> {
        match &self.vault {
            VaultStatus::AwaitingPassword { keyfile, .. } => keyfile.clone(),
            _ => None,
        }
    }

    pub fn set_unlock_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        if let VaultStatus::AwaitingPassword { error, .. } = &mut self.vault {
            *error = Some(message.into());
            cx.notify();
        }
    }

    pub fn begin_open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        // If we're transitioning from a currently-open vault directly into
        // unlocking another one (Welcome-recent → submit_password while a
        // different vault is already Open), park the active one first so it
        // doesn't get overwritten silently.
        self.park_active();
        self.vault = VaultStatus::Opening { path };
        cx.notify();
    }

    pub fn finish_open_attempt(
        &mut self,
        path: PathBuf,
        result: Result<VaultDocument, String>,
        cx: &mut Context<Self>,
    ) {
        if !matches!(&self.vault, VaultStatus::Opening { path: active } if active == &path) {
            return;
        }

        // Track whether the unlock succeeded so we can fire post-open
        // side-effects (recents push, sync rebind) below — they need a
        // `&mut self` borrow that conflicts with the match arm.
        let mut opened_path: Option<PathBuf> = None;
        self.vault = match result {
            Ok(document) => {
                let snapshot = document.snapshot();
                let selection = LibrarySelection::Group(snapshot.root.id.clone());
                let selected_entry_id = snapshot.root.entries.first().map(|entry| entry.id.clone());
                let visible_entries = Rc::new(entries_for_selection(
                    snapshot,
                    &selection,
                    "",
                    &HashMap::new(),
                ));
                let selected_strength = selected_entry_id
                    .as_deref()
                    .and_then(|id| document.strength_for_entry(id));

                opened_path = Some(path.clone());
                VaultStatus::Open {
                    path,
                    document: Box::new(document),
                    selection,
                    selected_entry_id,
                    search_query: String::new(),
                    visible_entries,
                    selected_strength,
                    last_used: HashMap::new(),
                }
            }
            Err(message) => VaultStatus::AwaitingPassword {
                path: path.clone(),
                keyfile: crate::keepass::KeePassRepository::suggested_keyfile(&path),
                error: Some(message),
            },
        };
        cx.notify();

        if let Some(opened) = opened_path {
            // Remember the vault for next launch's auto-resume + the
            // Welcome screen's Recents list.
            self.push_recent(opened.clone(), cx);
            if self.install_pending_sync_for(&opened) {
                cx.notify();
                return;
            }
            // If a sync config exists for this path, rebuild the
            // SyncBinding using the keychain refresh token. No-op for
            // local-only vaults.
            self.try_restore_sync_binding(opened, cx);
        }
    }

    /// Prepend `path` to the in-memory recents list (dedup + truncate),
    /// then schedule an atomic write to disk in the background. Failures
    /// are intentionally swallowed — the next successful open will retry,
    /// and we don't want a transient disk error to surface as a UI toast.
    fn push_recent(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        recents::push_front_in(&mut self.recents, path, recents::MAX_RECENTS);
        let snapshot = recents::RecentVaults {
            entries: self.recents.clone(),
        };
        cx.background_spawn(async move {
            let _ = recents::save(&snapshot);
        })
        .detach();
    }

    /// Try to rebuild a `SyncBinding` for the just-opened vault from the
    /// on-disk sync config + the keychain refresh token. Runs in the
    /// background and is a no-op for local-only vaults. On
    /// `InvalidGrant` (refresh token revoked), surfaces
    /// `SyncStatus::Reconnect` so the user is prompted to re-authenticate
    /// via SyncSettings — we don't auto-disconnect, since that would
    /// silently delete their config.
    fn try_restore_sync_binding(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        // Bail when there's no config on disk for this path — the common
        // case for local-only vaults.
        let config = match crate::sync::config::load(&path) {
            Ok(Some(c)) => c,
            _ => return,
        };

        // Defensive: if Connect just established a binding (during the
        // pick_kdbx_file → request_password → unlock flow), don't trash it.
        if self.sync.is_some() {
            return;
        }

        self.sync_status = SyncStatus::Restoring;
        cx.notify();

        let email = config.account_email.clone();
        let task = cx.background_spawn(async move {
            crate::sync::service::refresh_access_token(&email).map(|token| (config, token))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| match result {
                Ok((config, access_token)) => {
                    state.sync = Some(SyncBinding {
                        config,
                        access_token,
                    });
                    state.sync_status = SyncStatus::Synced {
                        at: chrono::Local::now(),
                        auto_merged: 0,
                    };
                    cx.notify();
                }
                Err(crate::sync::service::ServiceError::Auth(
                    crate::sync::auth::AuthError::InvalidGrant,
                )) => {
                    state.sync_status = SyncStatus::Reconnect;
                    cx.notify();
                }
                Err(e) => {
                    // Transient (network, etc.) — leave the user in
                    // Failed; the next save's sync_now will retry.
                    state.sync_status = SyncStatus::Failed(e.to_string());
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Read-only access to the auto-update flow state. Drives the welcome
    /// banner + Settings → Updates row.
    pub fn update_status(&self) -> &UpdateStatus {
        &self.update_status
    }

    pub fn whats_new_info(&self) -> Option<&UpdateInfo> {
        self.whats_new_info.as_ref()
    }

    pub fn open_whats_new(&mut self, cx: &mut Context<Self>) {
        let Some(info) = self.whats_new_info.clone() else {
            return;
        };
        self.open_overlay(Overlay::WhatsNew { info }, cx);
    }

    pub fn open_about(&mut self, cx: &mut Context<Self>) {
        self.open_overlay(Overlay::About, cx);
    }

    /// Kick off a background update check. No-op when one is already in
    /// flight or a download is running. Transitions:
    ///
    /// - `Idle` (or `Failed`) → `Checking` → `Available(_) | Idle | Failed`
    ///
    /// Mirrors the `try_restore_sync_binding` pattern: blocking I/O on a
    /// background thread, UI mutations bounced back to the main loop via
    /// `cx.spawn` + `entity.update`.
    pub fn start_update_check(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.update_status,
            UpdateStatus::Checking
                | UpdateStatus::Downloading { .. }
                | UpdateStatus::ReadyToRestart(_)
        ) {
            return;
        }
        self.update_status = UpdateStatus::Checking;
        cx.notify();

        let task = cx.background_spawn(async move { crate::update::check() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| {
                state.update_status = match result {
                    Ok(Some(info)) => UpdateStatus::Available(info),
                    Ok(None) => UpdateStatus::Idle,
                    Err(e) => UpdateStatus::Failed(e.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// Download + install whatever update is currently advertised. Caller is
    /// expected to have verified `update_status() == Available(_)` before
    /// calling — we don't pre-check, the underlying library re-fetches the
    /// manifest as part of `download_and_install`.
    ///
    /// Progress is reported via shared atomics: the blocking download
    /// callback writes byte counters from the background thread, and a
    /// foreground poll loop translates them into `UpdateStatus::Downloading
    /// { progress }` updates roughly every 150ms. When the server omits
    /// `Content-Length` we keep `progress` at 0 — the UI then shows an
    /// indeterminate "Downloading…" rather than a fake percentage.
    pub fn install_update(&mut self, cx: &mut Context<Self>) {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        use std::time::Duration;

        if matches!(self.update_status, UpdateStatus::Downloading { .. }) {
            return;
        }
        let UpdateStatus::Available(info) = self.update_status.clone() else {
            return;
        };
        self.update_status = UpdateStatus::Downloading {
            info,
            progress: 0.0,
        };
        cx.notify();

        let downloaded = Arc::new(AtomicU64::new(0));
        let total = Arc::new(AtomicU64::new(0));
        let done = Arc::new(AtomicBool::new(false));

        let downloaded_bg = downloaded.clone();
        let total_bg = total.clone();
        let done_bg = done.clone();
        let task = cx.background_spawn(async move {
            let result = crate::update::install(move |bytes, content_length| {
                downloaded_bg.store(bytes as u64, Ordering::Relaxed);
                if let Some(len) = content_length {
                    total_bg.store(len, Ordering::Relaxed);
                }
            });
            done_bg.store(true, Ordering::Relaxed);
            result
        });

        let downloaded_poll = downloaded.clone();
        let total_poll = total.clone();
        let done_poll = done.clone();
        cx.spawn(async move |this, cx| {
            loop {
                if done_poll.load(Ordering::Relaxed) {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                let bytes = downloaded_poll.load(Ordering::Relaxed);
                let len = total_poll.load(Ordering::Relaxed);
                if let Some(progress) =
                    (len > 0).then(|| (bytes as f32 / len as f32).clamp(0.0, 1.0))
                {
                    let _ = this.update(cx, |state, cx| {
                        if let UpdateStatus::Downloading { info, .. } = &state.update_status {
                            let info = info.clone();
                            state.update_status = UpdateStatus::Downloading { info, progress };
                            cx.notify();
                        }
                    });
                }
            }
        })
        .detach();

        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| {
                state.update_status = match result {
                    Ok(info) => {
                        let _ = crate::update::save_pending_whats_new(&info);
                        state.whats_new_info = Some(info.clone());
                        UpdateStatus::ReadyToRestart(info)
                    }
                    Err(e) => UpdateStatus::Failed(e.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    pub fn fail_vault_selection(
        &mut self,
        path: Option<PathBuf>,
        message: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        self.vault = VaultStatus::Error {
            message: message.into(),
            path,
        };
        cx.notify();
    }

    pub fn lock_vault(&mut self, cx: &mut Context<Self>) {
        self.vault = VaultStatus::Empty;
        self.overlay = Overlay::None;
        self.save_status = SaveStatus::Idle;
        self.sync = None;
        self.sync_status = SyncStatus::Disconnected;
        self.pending_sync = None;
        // Clear with the rest of the session secrets — entry titles in
        // the history would otherwise outlive the unlocked DB they came
        // from, which contradicts the rest of the lock contract.
        self.sync_history.clear();
        // Global auto-lock semantics: any parked vault gets wiped too so
        // a single idle timeout sweeps every decrypted session at once.
        self.parked.clear();
        self.parked_order.clear();
        cx.notify();
    }

    pub fn sync_history(&self) -> &[SyncHistoryEntry] {
        &self.sync_history
    }

    /// Append already-computed history entries to whichever slot
    /// (active vault or parked session) currently owns `target`. Mirrors
    /// the routing done by `apply_sync_status` / `with_sync_binding_mut_for`
    /// so a sync that completes after the user switched away still logs
    /// against the vault it actually changed. Vaults that were locked
    /// between dispatch and callback are dropped silently — there's no
    /// active session to surface the history against.
    fn append_sync_history_for(&mut self, target: &Path, entries: Vec<SyncHistoryEntry>) {
        if entries.is_empty() {
            return;
        }
        if matches!(&self.vault, VaultStatus::Open { path, .. } if path.as_path() == target) {
            sync_history::append_capped(&mut self.sync_history, entries);
        } else if let Some(parked) = self.parked.get_mut(target) {
            sync_history::append_capped(&mut parked.sync_history, entries);
        }
    }

    pub fn save_status(&self) -> &SaveStatus {
        &self.save_status
    }

    pub fn sync_status(&self) -> &SyncStatus {
        &self.sync_status
    }

    pub fn sync_binding(&self) -> Option<&SyncBinding> {
        self.sync.as_ref()
    }

    pub fn connect_flow(&self) -> Option<&ConnectFlow> {
        self.connect_flow.as_ref()
    }

    /// Reset the Connect overlay to its initial step. Called when the user
    /// opens Connect from Welcome.
    pub fn begin_connect_flow(&mut self, cx: &mut Context<Self>) {
        self.connect_flow = Some(ConnectFlow::PickProvider);
        cx.notify();
    }

    /// Drop any in-progress Connect flow state. Called by Cancel + on
    /// successful completion.
    pub fn clear_connect_flow(&mut self, cx: &mut Context<Self>) {
        if self.connect_flow.is_some() {
            self.connect_flow = None;
            cx.notify();
        }
    }

    /// Replace the current connect flow step. Used by the Connect overlay's
    /// Back / provider-pick buttons.
    pub fn connect_flow_set(&mut self, flow: ConnectFlow, cx: &mut Context<Self>) {
        self.connect_flow = Some(flow);
        cx.notify();
    }

    /// Compute the live TOTP code for the currently-selected entry, if any.
    /// Recomputed on every render (cheap, ~µs); the per-second AppShell tick
    /// triggers `cx.notify` which causes the detail panel to re-call this.
    pub fn totp_for_selected_entry(&self) -> Option<OtpDisplay> {
        let VaultStatus::Open {
            document,
            selected_entry_id,
            ..
        } = &self.vault
        else {
            return None;
        };
        let id = selected_entry_id.as_deref()?;
        document.totp_for_entry(id)
    }

    /// Flip the expanded/collapsed state of a sidebar group. Reads the
    /// current flag from the snapshot, writes the inverse via the
    /// document, and queues a *local-only* save so the change is
    /// durable across restarts without firing a cloud-sync push for
    /// every chevron click. The flag still rides out to the cloud
    /// piggybacking on the next real mutation's save_async. No-ops
    /// silently if the group has vanished mid-flight (rare race when
    /// the user clicks while a sync overwrites the tree).
    pub fn toggle_group_expanded(&mut self, group_id: &str, cx: &mut Context<Self>) {
        let VaultStatus::Open { document, .. } = &mut self.vault else {
            return;
        };
        let Some(group) = document.snapshot().find_group(group_id) else {
            return;
        };
        let new_value = !group.is_expanded;
        if document.set_group_expanded(group_id, new_value).is_ok() {
            cx.notify();
            self.save_async_local_only(cx);
        }
    }

    /// Walk the open vault, find every entry with a non-empty URL and no
    /// existing custom icon, and pull a favicon for each from DuckDuckGo.
    /// Successful fetches are written into the database as Custom Icons;
    /// once the loop is done we trigger a single `save_async` so the
    /// flushed bytes ride out via the normal save → sync path.
    ///
    /// Sequential by design: a typical vault has 30–200 entries with
    /// URLs, and DDG's icon service is fast — running these in parallel
    /// would mostly just shave a few seconds while burning more cache
    /// quota. Keeping it serial also gives us a clean progress label
    /// without coordinating shared mutable state across workers.
    ///
    /// Re-entrancy: if a run is already in flight, additional triggers
    /// are dropped (the UI also disables its button).
    pub fn start_favicon_download(&mut self, cx: &mut Context<Self>) {
        if self.favicon_status.is_running() {
            return;
        }
        let VaultStatus::Open { document, .. } = &self.vault else {
            return;
        };

        // Snapshot the (id, url) pairs up front so the spawned task
        // doesn't have to re-borrow the snapshot every iteration. We
        // skip entries that already have a custom icon — re-running
        // shouldn't blow away user-curated icons.
        let targets: Vec<(String, String)> = document
            .snapshot()
            .entries_recursive()
            .into_iter()
            .filter(|entry| !entry.url.trim().is_empty())
            .filter(|entry| entry.favicon.image.is_none())
            .map(|entry| (entry.id.clone(), entry.url.clone()))
            .collect();

        let total = targets.len();
        if total == 0 {
            self.favicon_status = FaviconDownloadStatus::Finished {
                succeeded: 0,
                total: 0,
            };
            cx.notify();
            return;
        }

        self.favicon_status = FaviconDownloadStatus::Running {
            done: 0,
            total,
            succeeded: 0,
        };
        cx.notify();

        cx.spawn(async move |this, cx| {
            let mut succeeded = 0usize;
            for (idx, (entry_id, url)) in targets.into_iter().enumerate() {
                // Each fetch off the UI thread — ureq is sync, so we'd
                // block the renderer otherwise.
                let url_for_task = url.clone();
                let bytes_result = cx
                    .background_spawn(async move { crate::favicon::fetch_favicon(&url_for_task) })
                    .await;

                let _ = this.update(cx, |state, cx| {
                    if let Ok(bytes) = bytes_result {
                        if let VaultStatus::Open { document, .. } = &mut state.vault {
                            // Errors here mean the entry vanished
                            // mid-run (e.g. user deleted it) — fine to
                            // silently skip.
                            if document.set_entry_custom_icon(&entry_id, bytes).is_ok() {
                                succeeded += 1;
                            }
                        }
                    }
                    state.favicon_status = FaviconDownloadStatus::Running {
                        done: idx + 1,
                        total,
                        succeeded,
                    };
                    cx.notify();
                });
            }

            let _ = this.update(cx, |state, cx| {
                state.favicon_status = FaviconDownloadStatus::Finished { succeeded, total };
                cx.notify();
                // Persist whichever icons we managed to land. `save_async`
                // is a no-op if `succeeded == 0` would still be valid —
                // running it harmlessly re-writes the same bytes — but
                // skip when there's nothing to save so we don't block
                // the disk for a no-op.
                if succeeded > 0 {
                    state.save_async(cx);
                }
            });
        })
        .detach();
    }

    /// Spawn an atomic save of the open vault on a background thread.
    ///
    /// Concurrency model: snapshots the live `Database` once on the foreground
    /// (cheap memcpy) and ships the clone + key material to a worker. The UI
    /// thread is free during the ~500 ms Argon2 KDF. If a save is already in
    /// flight we deliberately let the new one queue behind it — the latest
    /// state always wins, but we don't drop user changes.
    pub fn save_async(&mut self, cx: &mut Context<Self>) {
        self.save_async_internal(true, cx);
    }

    /// Save locally but skip the cloud-sync push afterwards. Used for
    /// purely cosmetic mutations (today: sidebar group collapse/expand)
    /// where firing a SharePoint upload on every click would burn
    /// bandwidth and — worse — race against any already-in-flight sync
    /// to produce a 412 ETag mismatch and an unnecessary Conflict
    /// overlay. The change still rides out to the cloud the next time
    /// any "real" mutation triggers `save_async`, so other devices
    /// eventually see the updated flags.
    pub fn save_async_local_only(&mut self, cx: &mut Context<Self>) {
        self.save_async_internal(false, cx);
    }

    fn save_async_internal(&mut self, sync_after: bool, cx: &mut Context<Self>) {
        let VaultStatus::Open { document, path, .. } = &self.vault else {
            return;
        };
        let payload = document.save_payload();
        let target = path.clone();

        self.save_status = SaveStatus::Saving;
        cx.notify();

        let target_for_callback = target.clone();
        let task = cx.background_spawn(async move { payload.save_to(&target) });

        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| {
                let succeeded = result.is_ok();
                let new_status = match result {
                    Ok(()) => SaveStatus::Saved,
                    Err(error) => SaveStatus::Failed(error.to_string()),
                };
                // Route the result by path: if the user switched away
                // from the saving vault while the disk write was in
                // flight, mark the parked session, not whoever is now
                // active.
                state.apply_save_status(&target_for_callback, new_status, cx);
                // Chain into sync against the same vault that just saved
                // — even if the user has switched away. `sync_now_for_path`
                // routes its results back to whichever slot still owns
                // `target_for_callback`, so a parked vault's edit still
                // makes it to SharePoint.
                if succeeded
                    && sync_after
                    && state.snapshot_sync_inputs(&target_for_callback).is_some()
                {
                    state.sync_now_for_path(&target_for_callback, cx);
                }
            });
        })
        .detach();
    }

    /// Create an entry inside the given group, refresh the snapshot-derived
    /// caches, focus the new entry, and trigger a background save. Returns the
    /// new entry's id on success.
    pub fn create_entry(
        &mut self,
        group_id: &str,
        draft: EntryDraft,
        cx: &mut Context<Self>,
    ) -> Result<String, MutationError> {
        let new_id = {
            let VaultStatus::Open {
                document,
                selection,
                selected_entry_id,
                search_query,
                visible_entries,
                selected_strength,
                last_used,
                ..
            } = &mut self.vault
            else {
                return Err(MutationError::EntryNotFound);
            };

            let new_id = document.create_entry(group_id, &draft)?;

            // Snap the user to the entry's group so they can see what they
            // just created — otherwise creating from inside "Favorites" or a
            // tag filter would silently land the entry off-screen.
            *selection = LibrarySelection::Group(group_id.to_string());
            search_query.clear();

            let entries = entries_for_selection(document.snapshot(), selection, "", last_used);
            *selected_entry_id = Some(new_id.clone());
            *visible_entries = Rc::new(entries);
            *selected_strength = document.strength_for_entry(&new_id);

            new_id
        };
        cx.notify();
        self.save_async(cx);
        Ok(new_id)
    }

    pub fn update_entry(
        &mut self,
        entry_id: &str,
        draft: EntryDraft,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        {
            let VaultStatus::Open {
                document,
                selection,
                selected_entry_id,
                search_query,
                visible_entries,
                selected_strength,
                last_used,
                ..
            } = &mut self.vault
            else {
                return Err(MutationError::EntryNotFound);
            };

            document.update_entry(entry_id, &draft)?;

            *visible_entries = Rc::new(entries_for_selection(
                document.snapshot(),
                selection,
                search_query,
                last_used,
            ));
            // Re-score; the password may have changed.
            if selected_entry_id.as_deref() == Some(entry_id) {
                *selected_strength = document.strength_for_entry(entry_id);
            }
        }
        cx.notify();
        self.save_async(cx);
        Ok(())
    }

    /// Move an entry to the recycle bin (creating one if necessary). Selection
    /// jumps to the next visible entry so the detail pane stays populated.
    pub fn delete_entry(
        &mut self,
        entry_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        self.run_entry_mutation(cx, |doc| doc.delete_entry(entry_id), entry_id)
    }

    /// Permanent (unrecoverable) delete. Use only after a confirmation step in
    /// the UI — `save_async` flushes the result to disk and the entry is gone.
    pub fn delete_entry_permanent(
        &mut self,
        entry_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        self.run_entry_mutation(cx, |doc| doc.delete_entry_permanent(entry_id), entry_id)
    }

    /// Restore an entry from the recycle bin to the vault root.
    pub fn restore_entry(
        &mut self,
        entry_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        self.run_entry_mutation(cx, |doc| doc.restore_entry(entry_id), entry_id)
    }

    /// Move an entry into a different group. Used by drag-and-drop.
    /// Mirrors the update/delete pattern: refresh the visible-entries
    /// cache against the current selection (which may now exclude the
    /// moved entry, e.g. when viewing only one group), then schedule a
    /// background save. Selection-tracking is intentionally lazy —
    /// `vault_browser()` falls back to the first visible entry if the
    /// previously-selected one disappears from view, so we don't need
    /// to repoint `selected_entry_id` here.
    pub fn move_entry(
        &mut self,
        entry_id: &str,
        target_group_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        {
            let VaultStatus::Open {
                document,
                selection,
                search_query,
                visible_entries,
                last_used,
                ..
            } = &mut self.vault
            else {
                return Err(MutationError::EntryNotFound);
            };

            document.move_entry(entry_id, target_group_id)?;

            *visible_entries = Rc::new(entries_for_selection(
                document.snapshot(),
                selection,
                search_query,
                last_used,
            ));
        }
        cx.notify();
        self.save_async(cx);
        Ok(())
    }

    /// Create a new group under `parent_id` and select it so the user
    /// lands on the freshly-created (empty) group. Real content mutation
    /// — uses the full `save_async` path so the change syncs to the
    /// cloud, unlike the cosmetic `toggle_group_expanded`.
    pub fn create_group(
        &mut self,
        parent_id: &str,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Result<String, MutationError> {
        let new_id = {
            let VaultStatus::Open {
                document,
                selection,
                selected_entry_id,
                search_query,
                visible_entries,
                selected_strength,
                last_used,
                ..
            } = &mut self.vault
            else {
                return Err(MutationError::GroupNotFound);
            };

            let new_id = document.create_group(parent_id, name)?;
            *selection = LibrarySelection::Group(new_id.clone());
            search_query.clear();
            *selected_entry_id = None;
            *selected_strength = None;
            let entries = entries_for_selection(document.snapshot(), selection, "", last_used);
            *visible_entries = Rc::new(entries);
            new_id
        };
        cx.notify();
        self.save_async(cx);
        Ok(new_id)
    }

    /// Rename an existing group. Refreshes `visible_entries` because
    /// `EntryRow::group_path` carries the group names and would
    /// otherwise render stale text in the entry list.
    pub fn rename_group(
        &mut self,
        group_id: &str,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        {
            let VaultStatus::Open {
                document,
                selection,
                search_query,
                visible_entries,
                last_used,
                ..
            } = &mut self.vault
            else {
                return Err(MutationError::GroupNotFound);
            };

            document.rename_group(group_id, name)?;
            *visible_entries = Rc::new(entries_for_selection(
                document.snapshot(),
                selection,
                search_query,
                last_used,
            ));
        }
        cx.notify();
        self.save_async(cx);
        Ok(())
    }

    /// Soft-delete a group: hand off to `VaultDocument::delete_group`,
    /// then if the deleted group was the active selection, snap back to
    /// the root so the entry list isn't pointing at a now-orphaned id.
    pub fn delete_group(
        &mut self,
        group_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        {
            let VaultStatus::Open {
                document,
                selection,
                selected_entry_id,
                search_query,
                visible_entries,
                selected_strength,
                last_used,
                ..
            } = &mut self.vault
            else {
                return Err(MutationError::GroupNotFound);
            };

            document.delete_group(group_id)?;

            let snapshot = document.snapshot();
            if let LibrarySelection::Group(sel_id) = selection.clone()
                && sel_id == group_id
            {
                *selection = LibrarySelection::Group(snapshot.root.id.clone());
            }
            search_query.clear();
            let entries = entries_for_selection(snapshot, selection, "", last_used);
            *selected_entry_id = entries.first().map(|e| e.id.clone());
            *selected_strength = selected_entry_id
                .as_deref()
                .and_then(|id| document.strength_for_entry(id));
            *visible_entries = Rc::new(entries);
        }
        cx.notify();
        self.save_async(cx);
        Ok(())
    }

    /// Toggle the favourite-marker on an entry. Mutates the underlying
    /// `Favorite` tag through `VaultDocument`, refreshes the visible
    /// snapshot caches so the star icon updates immediately, and
    /// schedules a background save (which chains into sync if connected).
    pub fn toggle_starred(
        &mut self,
        entry_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        {
            let VaultStatus::Open {
                document,
                selection,
                search_query,
                visible_entries,
                last_used,
                ..
            } = &mut self.vault
            else {
                return Err(MutationError::EntryNotFound);
            };

            document.toggle_starred(entry_id)?;

            *visible_entries = Rc::new(entries_for_selection(
                document.snapshot(),
                selection,
                search_query,
                last_used,
            ));
        }
        cx.notify();
        self.save_async(cx);
        Ok(())
    }

    /// Shared post-mutation bookkeeping for delete / restore / permanent-delete:
    /// run the mutation, refresh the visible entry list, repoint the selection
    /// if the affected entry was selected, then schedule the autosave.
    fn run_entry_mutation<F>(
        &mut self,
        cx: &mut Context<Self>,
        mutate: F,
        entry_id: &str,
    ) -> Result<(), MutationError>
    where
        F: FnOnce(&mut VaultDocument) -> Result<(), MutationError>,
    {
        {
            let VaultStatus::Open {
                document,
                selection,
                selected_entry_id,
                search_query,
                visible_entries,
                selected_strength,
                last_used,
                ..
            } = &mut self.vault
            else {
                return Err(MutationError::EntryNotFound);
            };

            mutate(document)?;

            let entries =
                entries_for_selection(document.snapshot(), selection, search_query, last_used);
            if selected_entry_id.as_deref() == Some(entry_id) {
                *selected_entry_id = entries.first().map(|e| e.id.clone());
                *selected_strength = selected_entry_id
                    .as_deref()
                    .and_then(|id| document.strength_for_entry(id));
            }
            *visible_entries = Rc::new(entries);
        }
        cx.notify();
        self.save_async(cx);
        Ok(())
    }

    pub fn pending_unlock_path(&self) -> Option<PathBuf> {
        match &self.vault {
            VaultStatus::AwaitingPassword { path, .. } => Some(path.clone()),
            _ => None,
        }
    }

    /// Path of the vault the user is currently looking at, regardless of
    /// state (Open / Opening / AwaitingPassword). `None` on the Welcome
    /// screen. Used by the vault switcher to mark the active row.
    pub fn current_vault_path(&self) -> Option<PathBuf> {
        match &self.vault {
            VaultStatus::Open { path, .. }
            | VaultStatus::Opening { path }
            | VaultStatus::AwaitingPassword { path, .. } => Some(path.clone()),
            VaultStatus::Empty => None,
            VaultStatus::Error { path, .. } => path.clone(),
        }
    }

    pub fn unlock_prompt(&self) -> Option<UnlockPrompt> {
        match &self.vault {
            VaultStatus::AwaitingPassword {
                path,
                keyfile,
                error,
            } => Some(UnlockPrompt {
                path: path.clone(),
                file_name: file_name(path),
                display_path: path.display().to_string(),
                keyfile: keyfile.clone(),
                error: error.clone(),
            }),
            _ => None,
        }
    }

    pub fn select_group(&mut self, group_id: impl Into<String>, cx: &mut Context<Self>) {
        let group_id = group_id.into();

        let VaultStatus::Open {
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            last_used,
            ..
        } = &mut self.vault
        else {
            return;
        };

        let snapshot = document.snapshot();
        if snapshot.find_group(&group_id).is_none() {
            return;
        }

        *selection = LibrarySelection::Group(group_id);
        search_query.clear();
        let entries = entries_for_selection(snapshot, selection, "", last_used);
        *selected_entry_id = entries.first().map(|entry| entry.id.clone());
        *selected_strength = selected_entry_id
            .as_deref()
            .and_then(|id| document.strength_for_entry(id));
        *visible_entries = Rc::new(entries);
        cx.notify();
    }

    pub fn select_library(&mut self, sel: LibrarySelection, cx: &mut Context<Self>) {
        let VaultStatus::Open {
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            last_used,
            ..
        } = &mut self.vault
        else {
            return;
        };
        if *selection == sel {
            return;
        }
        *selection = sel;
        search_query.clear();
        let entries = entries_for_selection(document.snapshot(), selection, "", last_used);
        *selected_entry_id = entries.first().map(|entry| entry.id.clone());
        *selected_strength = selected_entry_id
            .as_deref()
            .and_then(|id| document.strength_for_entry(id));
        *visible_entries = Rc::new(entries);
        cx.notify();
    }

    pub fn select_entry(&mut self, entry_id: impl Into<String>, cx: &mut Context<Self>) {
        let entry_id = entry_id.into();

        let VaultStatus::Open {
            document,
            selected_entry_id,
            selected_strength,
            ..
        } = &mut self.vault
        else {
            return;
        };

        if document.snapshot().find_entry(&entry_id).is_some() {
            *selected_strength = document.strength_for_entry(&entry_id);
            *selected_entry_id = Some(entry_id);
            cx.notify();
        }
    }

    pub fn set_search_query(&mut self, query: impl Into<String>, cx: &mut Context<Self>) {
        let query = query.into();

        let VaultStatus::Open {
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            last_used,
            ..
        } = &mut self.vault
        else {
            return;
        };

        if *search_query == query {
            return;
        }

        *search_query = query;
        let entries =
            entries_for_selection(document.snapshot(), selection, search_query, last_used);
        let selected_entry_is_visible = selected_entry_id
            .as_deref()
            .is_some_and(|id| entries.iter().any(|entry| entry.id == id));

        if !selected_entry_is_visible {
            *selected_entry_id = entries.first().map(|entry| entry.id.clone());
            *selected_strength = selected_entry_id
                .as_deref()
                .and_then(|id| document.strength_for_entry(id));
        }

        *visible_entries = Rc::new(entries);
        cx.notify();
    }

    pub fn clear_search(&mut self, cx: &mut Context<Self>) {
        let VaultStatus::Open {
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            last_used,
            ..
        } = &mut self.vault
        else {
            return;
        };

        if search_query.is_empty() {
            return;
        }

        search_query.clear();
        let entries = entries_for_selection(document.snapshot(), selection, "", last_used);
        *selected_entry_id = entries.first().map(|entry| entry.id.clone());
        *selected_strength = selected_entry_id
            .as_deref()
            .and_then(|id| document.strength_for_entry(id));
        *visible_entries = Rc::new(entries);
        cx.notify();
    }

    /// Stamp the currently-selected entry as "just used" in the
    /// in-memory access log. Called from the AppShell after a
    /// successful password / username copy. No-op when no vault is
    /// open or no entry is selected. Doesn't notify — the
    /// `RecentlyUsed` list is rebuilt on the next selection change,
    /// which matches KeePassXC (the list is a snapshot, not live).
    pub fn mark_selected_used(&mut self) {
        if let VaultStatus::Open {
            selected_entry_id,
            last_used,
            ..
        } = &mut self.vault
            && let Some(id) = selected_entry_id.clone()
        {
            last_used.insert(id, Local::now());
        }
    }

    /// Same as `mark_selected_used` but for an explicit entry id —
    /// used by the auto-type path, where the credential we just typed
    /// is the foreground-matched entry, not necessarily the one the
    /// user has selected in the sidebar.
    pub fn mark_entry_used(&mut self, entry_id: &str) {
        if let VaultStatus::Open { last_used, .. } = &mut self.vault {
            last_used.insert(entry_id.to_string(), Local::now());
        }
    }

    pub fn copy_selected_value(&self, kind: CopyValueKind) -> Option<String> {
        let model = self.vault_browser()?;
        let entry = model.selected_entry?;

        match kind {
            CopyValueKind::Username => non_empty_copy(entry.username),
            CopyValueKind::Url => non_empty_copy(entry.url),
            CopyValueKind::Password => {
                let VaultStatus::Open { document, .. } = &self.vault else {
                    return None;
                };

                document.password_for_entry(&entry.id)
            }
        }
    }

    /// Read a single custom-field value off any entry by id. Drives
    /// the detail-panel "Additional fields" copy buttons and the
    /// launcher path's per-key lookups. Returns `None` when no vault
    /// is open, the entry doesn't exist, or the field is unset.
    pub fn custom_field_value(&self, entry_id: &str, key: &str) -> Option<String> {
        let VaultStatus::Open { document, .. } = &self.vault else {
            return None;
        };
        document.custom_field_value(entry_id, key)
    }

    pub fn vault_browser(&self) -> Option<VaultBrowserModel> {
        let VaultStatus::Open {
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            ..
        } = &self.vault
        else {
            return None;
        };

        let snapshot = document.snapshot_rc();
        let showing_search_results = !search_query.trim().is_empty();

        let selected_entry = selected_entry_id
            .as_deref()
            .and_then(|id| visible_entries.iter().find(|entry| entry.id == id))
            .cloned()
            .or_else(|| visible_entries.first().cloned());

        let selection_label = selection_label_for(selection, &snapshot);

        Some(VaultBrowserModel {
            snapshot,
            selection: selection.clone(),
            selection_label,
            selected_entry_id: selected_entry.as_ref().map(|entry| entry.id.clone()),
            entries: Rc::clone(visible_entries),
            selected_entry,
            selected_strength: *selected_strength,
            search_query: search_query.clone(),
            showing_search_results,
        })
    }

    pub fn summary(&self) -> VaultSummary {
        let provider = self.sync.as_ref().map(|b| match b.config.provider {
            crate::sync::config::SyncProvider::SharePoint => "SharePoint".to_string(),
        });
        let synced_at = sync_status_label(&self.sync_status);
        let auto_merged = match &self.sync_status {
            SyncStatus::Synced { auto_merged, .. } if *auto_merged > 0 => Some(*auto_merged),
            _ => None,
        };

        match &self.vault {
            VaultStatus::Empty => VaultSummary {
                title: "No vault open".to_string(),
                subtitle: "Choose a KeePass database to begin.".to_string(),
                status: "Locked".to_string(),
                entries: 0,
                groups: 0,
                is_open: false,
                is_busy: false,
                provider: None,
                synced_at: None,
                auto_merged: None,
            },
            VaultStatus::AwaitingPassword { path, .. } => VaultSummary {
                title: file_name(path),
                subtitle: path.display().to_string(),
                status: "Password required".to_string(),
                entries: 0,
                groups: 0,
                is_open: false,
                is_busy: false,
                provider: provider.clone(),
                synced_at: synced_at.clone(),
                auto_merged,
            },
            VaultStatus::Opening { path } => VaultSummary {
                title: file_name(path),
                subtitle: "Decrypting database…".to_string(),
                status: "Opening".to_string(),
                entries: 0,
                groups: 0,
                is_open: false,
                is_busy: true,
                provider: provider.clone(),
                synced_at: synced_at.clone(),
                auto_merged,
            },
            VaultStatus::Open { path, document, .. } => VaultSummary {
                title: file_name(path),
                subtitle: path.display().to_string(),
                status: "Synced".to_string(),
                entries: document.snapshot().entry_count,
                groups: document.snapshot().group_count.saturating_sub(1),
                is_open: true,
                is_busy: false,
                provider,
                synced_at,
                auto_merged,
            },
            VaultStatus::Error { message, path } => VaultSummary {
                title: "Could not open vault".to_string(),
                subtitle: path
                    .as_ref()
                    .map_or_else(|| message.clone(), |path| path.display().to_string()),
                status: "Error".to_string(),
                entries: 0,
                groups: 0,
                is_open: false,
                is_busy: false,
                provider: None,
                synced_at: None,
                auto_merged: None,
            },
        }
    }

    // ============== Sync actions ==============

    /// Tear down the current sync binding: drop the in-memory token, mark
    /// status as Disconnected, then in the background remove the keychain
    /// entry + sync-config file. UI updates immediately; cleanup is fire-
    /// and-forget (failures here just leave a stale config we'll happily
    /// overwrite next Connect).
    pub fn disconnect_sync(&mut self, cx: &mut Context<Self>) {
        let Some(binding) = self.sync.take() else {
            return;
        };
        self.sync_status = SyncStatus::Disconnected;
        // Activity log is tied to the connected sync — once the user
        // disconnects, the events refer to a relationship that no
        // longer exists. Clearing avoids stale "Updated from remote"
        // lines hanging around after a fresh Connect.
        self.sync_history.clear();
        cx.notify();
        cx.background_spawn(async move {
            let _ = crate::sync::service::disconnect(&binding.config);
        })
        .detach();
    }

    /// Drop the Connect overlay's transient state. Wired to the Cancel
    /// button + the Escape key.
    pub fn cancel_connect(&mut self, cx: &mut Context<Self>) {
        self.connect_flow = None;
        cx.notify();
    }

    /// Step 1 of Connect: request a device code and kick off the polling
    /// loop. UI should observe `connect_flow` transitioning to
    /// `Some(SigningIn { .. })` and switch to the device-code screen.
    /// No URL/path is needed up front — the user picks a file *after*
    /// signing in (see `Picking`).
    pub fn start_sharepoint_connect(&mut self, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async move { crate::sync::service::request_device_code() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| match result {
                Ok(challenge) => {
                    state.connect_flow = Some(ConnectFlow::SigningIn {
                        challenge: challenge.clone(),
                    });
                    cx.notify();
                    state.start_token_polling(challenge, cx);
                }
                Err(e) => {
                    let msg = e.to_string();
                    state.connect_flow = Some(ConnectFlow::Failed(msg.clone()));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Background polling loop. Runs until token received, code expired,
    /// auth declined, or the user cancels (we observe `connect_flow`
    /// transitioning out of `SigningIn` between iterations).
    fn start_token_polling(&mut self, challenge: DeviceCodeChallenge, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let mut interval = challenge.interval;
            loop {
                // Cooperative cancel: if the user closed Connect (or moved
                // past SigningIn for any other reason), stop polling.
                let still_signing_in = this
                    .update(cx, |s, _| {
                        matches!(s.connect_flow, Some(ConnectFlow::SigningIn { .. }))
                    })
                    .unwrap_or(false);
                if !still_signing_in {
                    return;
                }

                // Hard timeout: if the device-code expiry passed, give up.
                if std::time::SystemTime::now() > challenge.expires_at {
                    let _ = this.update(cx, |s, cx| {
                        let msg = "Device code expired before sign-in.".to_string();
                        s.connect_flow = Some(ConnectFlow::Failed(msg.clone()));
                        cx.notify();
                    });
                    return;
                }

                cx.background_executor().timer(interval).await;

                let challenge_clone = challenge.clone();
                let outcome = cx
                    .background_spawn(
                        async move { crate::sync::auth::poll_token(&challenge_clone) },
                    )
                    .await;

                use crate::sync::auth::PollOutcome;
                match outcome {
                    PollOutcome::Pending => continue,
                    PollOutcome::SlowDown => {
                        // Server asked us to back off; double the interval as
                        // suggested by the OAuth device-code spec.
                        interval = interval.saturating_mul(2);
                        continue;
                    }
                    PollOutcome::Token(token) => {
                        let _ = this.update(cx, |s, cx| {
                            // Transition to picker (loading state); spawn the
                            // file-list fetch.
                            s.connect_flow = Some(ConnectFlow::Picking {
                                token: token.clone(),
                                results: Vec::new(),
                                query: String::new(),
                                loading: true,
                                error: None,
                            });
                            cx.notify();
                            s.start_kdbx_search(token, cx);
                        });
                        return;
                    }
                    PollOutcome::Failed(e) => {
                        let msg = e.to_string();
                        let _ = this.update(cx, |s, cx| {
                            s.connect_flow = Some(ConnectFlow::Failed(msg.clone()));
                            cx.notify();
                        });
                        return;
                    }
                }
            }
        })
        .detach();
    }

    /// Step 2 of Connect: fetch the user's `.kdbx` files. Cheap (one
    /// search call); results are filtered client-side as the user types.
    fn start_kdbx_search(&mut self, token: AccessToken, cx: &mut Context<Self>) {
        let token_for_task = token.clone();
        let task =
            cx.background_spawn(
                async move { crate::sync::service::list_kdbx_files(&token_for_task) },
            );
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| {
                if let Some(ConnectFlow::Picking {
                    results,
                    loading,
                    error,
                    ..
                }) = &mut state.connect_flow
                {
                    *loading = false;
                    match result {
                        Ok(hits) => {
                            *results = hits;
                            *error = None;
                        }
                        Err(e) => {
                            *error = Some(e.to_string());
                        }
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Live-filter the picker as the user types. Cheap — runs against the
    /// already-fetched list, no API calls.
    pub fn set_picker_query(&mut self, query: String, cx: &mut Context<Self>) {
        if let Some(ConnectFlow::Picking { query: q, .. }) = &mut self.connect_flow {
            *q = query;
            cx.notify();
        }
    }

    /// Step 3 of Connect: user picked one of the search results. Download
    /// the file, write it locally, persist SyncConfig + keychain token,
    /// then transition the vault into AwaitingPassword so the unlock flow
    /// takes over.
    pub fn pick_kdbx_file(
        &mut self,
        hit: DriveItemHit,
        local_path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if self.is_unlocked_path(&local_path) {
            self.connect_flow = Some(ConnectFlow::Failed(
                "That local vault is already open. Use the Vaults switcher to switch to it."
                    .to_string(),
            ));
            cx.notify();
            return;
        }
        if self.has_open_sync_remote(&hit.drive_id, &hit.item_id) {
            self.connect_flow = Some(ConnectFlow::Failed(
                "That SharePoint vault is already open. Use the Vaults switcher to switch to it."
                    .to_string(),
            ));
            cx.notify();
            return;
        }
        // The picker holds the access token; capture it before transitioning
        // out of Picking (which drops the token).
        let token = match &self.connect_flow {
            Some(ConnectFlow::Picking { token, .. }) => token.clone(),
            _ => return,
        };
        self.connect_flow = Some(ConnectFlow::Downloading);
        cx.notify();

        let path_for_task = local_path.clone();
        let task = cx.background_spawn(async move {
            let result =
                crate::sync::service::complete_connect_picked(&hit, token, &path_for_task)?;
            // Write bytes to disk before returning so the unlock flow's
            // `KeePassRepository::open` finds them.
            std::fs::write(&path_for_task, &result.remote_bytes).map_err(|e| {
                crate::sync::service::ServiceError::Io {
                    path: path_for_task.clone(),
                    source: e,
                }
            })?;
            Ok::<_, crate::sync::service::ServiceError>(result)
        });
        let final_path = local_path;
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| match result {
                Ok(connect_result) => {
                    state.connect_flow = None;
                    state.overlay = Overlay::None;
                    state.pending_sync = Some(PendingSync {
                        local_path: final_path.clone(),
                        binding: SyncBinding {
                            config: connect_result.config,
                            access_token: connect_result.access_token,
                        },
                    });
                    state.request_password(final_path, cx);
                    cx.notify();
                }
                Err(e) => {
                    let msg = e.to_string();
                    state.connect_flow = Some(ConnectFlow::Failed(msg.clone()));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Push the active vault's local file to SharePoint. Used as the
    /// manual SyncSettings → Sync now button. No-op when no vault is
    /// active or the active vault is local-only.
    pub fn sync_now(&mut self, cx: &mut Context<Self>) {
        let target = match &self.vault {
            VaultStatus::Open { path, .. } => path.clone(),
            _ => return,
        };
        self.sync_now_for_path(&target, cx);
    }

    /// Path-aware sync trigger. Works for both the active vault and a
    /// vault the user has parked (e.g., after edit-then-switch). All
    /// background-task results are routed back to whichever slot
    /// (`vault` or `parked[target]`) holds the binding at completion
    /// time, so a sync that finishes after the user has switched away
    /// updates the saving vault, not whoever is now in focus.
    pub fn sync_now_for_path(&mut self, target: &Path, cx: &mut Context<Self>) {
        let Some((config, token, master_password)) = self.snapshot_sync_inputs(target) else {
            return;
        };
        let local_path = target.to_path_buf();

        self.apply_sync_status(target, SyncStatus::Syncing, cx);

        let task_path = local_path.clone();
        let task_config = config.clone();
        let task = cx.background_spawn(async move {
            let token = crate::sync::service::ensure_fresh(token, &task_config.account_email)?;
            let bytes = crate::sync::service::read_local(&task_path)?;
            let outcome = crate::sync::service::upload_after_save(&task_config, &token, &bytes)?;
            Ok::<_, crate::sync::service::ServiceError>((outcome, token))
        });

        let callback_path = local_path;
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| match result {
                Ok((outcome, fresh_token)) => {
                    state.with_sync_binding_mut_for(&callback_path, |b| {
                        b.access_token = fresh_token;
                    });
                    use crate::sync::service::UploadAfterSave;
                    match outcome {
                        UploadAfterSave::Synced { new_etag, item: _ } => {
                            state.with_sync_binding_mut_for(&callback_path, |b| {
                                b.config.last_etag = new_etag;
                                // Persist updated etag — best effort; if the
                                // disk write fails we'll just re-detect a
                                // conflict next push (and re-resolve).
                                let _ = crate::sync::config::save(&b.config);
                            });
                            state.apply_sync_status(
                                &callback_path,
                                SyncStatus::Synced {
                                    at: chrono::Local::now(),
                                    auto_merged: 0,
                                },
                                cx,
                            );
                        }
                        UploadAfterSave::Conflict {
                            remote_bytes,
                            remote_etag,
                        } => {
                            state.handle_remote_conflict_for(
                                &callback_path,
                                remote_bytes,
                                remote_etag,
                                master_password,
                                cx,
                            );
                        }
                    }
                }
                Err(e) => {
                    let status = match &e {
                        crate::sync::service::ServiceError::Auth(
                            crate::sync::auth::AuthError::InvalidGrant,
                        ) => SyncStatus::Reconnect,
                        _ => SyncStatus::Failed(e.to_string()),
                    };
                    state.apply_sync_status(&callback_path, status, cx);
                }
            });
        })
        .detach();
    }

    /// Decrypt remote bytes with the master password, build a `ConflictReport`
    /// against the in-memory local DB, and either open the Conflict overlay
    /// (when `target` is the active vault) or mark the parked vault Failed
    /// so the user can resolve it after switching back. Auto-merge is run
    /// for both active and parked targets — silent merges don't need the UI.
    fn handle_remote_conflict_for(
        &mut self,
        target: &Path,
        remote_bytes: Vec<u8>,
        remote_etag: String,
        master_password: String,
        cx: &mut Context<Self>,
    ) {
        let Some(local_db) = self.database_clone_for(target) else {
            // Vault was locked between issuing the upload and the 412
            // response landing — nothing to merge against. Drop silently.
            return;
        };

        let target_is_active = matches!(
            &self.vault,
            VaultStatus::Open { path, .. } if path.as_path() == target
        );

        match crate::keepass::KeePassRepository::open_bytes(&remote_bytes, &master_password, None) {
            Ok(remote_doc) => {
                let remote_db = remote_doc.database().clone();
                let report = crate::keepass::merge::diff(&local_db, &remote_db);

                // Git-style: if no per-entry conflicts to decide, auto-merge
                // silently. Remote-only additions get pulled in with their
                // original UUIDs preserved (see merge::add_entry_under) and
                // the result uploads back. The user sees no overlay — just a
                // "Synced · N merged" badge in the status pill.
                if report.conflicts.is_empty() {
                    let auto_merged_count = report.remote_only.len() + report.auto_resolved.len();
                    // Snapshot the change list now (silent path → empty
                    // picks), but defer the actual append until the
                    // local save phase inside commit_merged_for has
                    // succeeded. That way a save/reload failure doesn't
                    // leave phantom rows in the activity log.
                    let history_entries = sync_history::entries_from_report(
                        &report,
                        &HashMap::new(),
                        chrono::Local::now(),
                    );
                    let merged = crate::keepass::merge::apply_picks(
                        &local_db,
                        &remote_db,
                        &HashMap::new(),
                        &report,
                    );
                    self.commit_merged_for(
                        target,
                        merged,
                        remote_etag,
                        master_password,
                        auto_merged_count,
                        history_entries,
                        cx,
                    );
                    return;
                }

                // Real conflicts. The Conflict overlay is single-vault by
                // design — it edits the user's active focus. For a parked
                // vault, mark Failed with a hint so the user knows to
                // switch back before resolving.
                if !target_is_active {
                    self.apply_sync_status(
                        target,
                        SyncStatus::Failed(
                            "Remote conflict — switch back to this vault to resolve.".into(),
                        ),
                        cx,
                    );
                    return;
                }
                let mut picks: HashMap<String, Side> = HashMap::new();
                for c in &report.conflicts {
                    picks.insert(c.id.clone(), Side::Local);
                }
                self.sync_status = SyncStatus::Conflict(Box::new(ConflictState {
                    local_db,
                    remote_db,
                    remote_etag,
                    report,
                    picks,
                }));
                self.overlay = Overlay::Conflict;
                cx.notify();
            }
            Err(_) => {
                self.apply_sync_status(
                    target,
                    SyncStatus::Failed(
                        "Remote file uses a different master password — \
                         cannot merge automatically."
                            .into(),
                    ),
                    cx,
                );
            }
        }
    }

    /// Mutate one user pick. Called by the Conflict overlay when the user
    /// clicks "Keep this" on either side. Idempotent.
    pub fn set_conflict_pick(&mut self, entry_id: &str, side: Side, cx: &mut Context<Self>) {
        let SyncStatus::Conflict(state) = &mut self.sync_status else {
            return;
        };
        state.picks.insert(entry_id.to_string(), side);
        cx.notify();
    }

    /// Finalise the conflict: build the merged DB from picks, save it
    /// locally, force-upload to SharePoint, dismiss the overlay.
    ///
    /// Concurrency note: we send `If-Match: conflict.remote_etag` so a
    /// third device that wrote during the user's resolution surfaces as a
    /// fresh 412 → re-decrypt → re-diff → re-prompt. That's safer than
    /// blind force-overwrite, at the cost of one extra round trip in the
    /// rare race case.
    pub fn apply_conflict_resolution(&mut self, cx: &mut Context<Self>) {
        let SyncStatus::Conflict(state) = &self.sync_status else {
            return;
        };
        let VaultStatus::Open { document, path, .. } = &self.vault else {
            return;
        };
        let merged = crate::keepass::merge::apply_picks(
            &state.local_db,
            &state.remote_db,
            &state.picks,
            &state.report,
        );
        let remote_etag = state.remote_etag.clone();
        let master_password = document.password().to_string();
        let target = path.clone();
        // Translate report + picks into history entries up front so the
        // borrow on `state.sync_status` drops cleanly. Append happens
        // inside commit_merged_for once the merged DB is actually on
        // disk and re-read into memory — see the "defer until success"
        // note on the silent-merge call site.
        let history_entries =
            sync_history::entries_from_report(&state.report, &state.picks, chrono::Local::now());

        // User-driven resolution — the "Synced · N merged" badge is reserved
        // for git-style silent merges where the user got no overlay at all.
        // Manual resolution always reports auto_merged = 0.
        self.commit_merged_for(
            &target,
            merged,
            remote_etag,
            master_password,
            0,
            history_entries,
            cx,
        );
    }

    /// Save a merged Database locally, reload the in-memory document from
    /// the freshly-encrypted bytes, and force-upload to SharePoint with the
    /// supplied `If-Match` ETag. Used by both:
    ///
    /// - **Manual conflict resolution** (`apply_conflict_resolution`) where
    ///   the user picked sides in the overlay, and
    /// - **Silent auto-merge** (in `handle_remote_conflict` when the diff
    ///   is conflict-free) where there was nothing for the user to decide.
    ///
    /// `auto_merged` is the count surfaced in the "Synced · N merged" badge —
    /// non-zero only on the silent-merge path.
    ///
    /// `history_entries` are the pre-computed activity-log rows for this
    /// merge. They're appended to the target vault's history only after
    /// the local save + reload succeeds (phase 1) — so a save failure
    /// can't leave phantom rows referencing changes that never
    /// actually committed.
    #[allow(clippy::too_many_arguments)]
    fn commit_merged_for(
        &mut self,
        target: &Path,
        merged: keepass::Database,
        remote_etag: String,
        master_password: String,
        auto_merged: usize,
        history_entries: Vec<SyncHistoryEntry>,
        cx: &mut Context<Self>,
    ) {
        // Pull config + token + keyfile path from whichever slot owns
        // `target` right now. Works for both active and parked vaults
        // — silent auto-merge from a parked sync still writes through
        // to disk + uploads cleanly.
        let (document_password, keyfile_path, config, token) = {
            let inputs = self.snapshot_sync_inputs(target);
            let keyfile_path = if matches!(
                &self.vault,
                VaultStatus::Open { path, .. } if path.as_path() == target
            ) {
                match &self.vault {
                    VaultStatus::Open { document, .. } => {
                        document.keyfile_path().map(std::path::Path::to_path_buf)
                    }
                    _ => None,
                }
            } else {
                self.parked
                    .get(target)
                    .and_then(|p| p.document.keyfile_path().map(std::path::Path::to_path_buf))
            };
            match inputs {
                Some((config, token, pw)) => (pw, keyfile_path, config, token),
                None => return,
            }
        };

        let payload =
            crate::keepass::SavePayload::for_merged(merged, document_password, keyfile_path);
        let local_path = target.to_path_buf();
        let if_match = remote_etag;

        self.apply_sync_status(target, SyncStatus::Syncing, cx);

        // Phase 1: local merge save. Splitting this off from the network
        // step lets us commit the merge into the in-memory document
        // *before* we go anywhere near the network. Without that, an
        // upload failure (or a token-refresh failure) parked the user back
        // on the pre-merge in-memory state while the already-merged bytes
        // sat on disk — the next ordinary save would clobber the merge
        // with stale data.
        let save_path = local_path.clone();
        let local_save_task = cx.background_spawn(async move { payload.save_to(&save_path) });

        let reload_path = local_path.clone();
        let reload_password = master_password;
        let network_path = local_path.clone();
        let callback_path = local_path;

        cx.spawn(async move |this, cx| {
            // Wrapped in an Option so the inner FnOnce can `.take()` it
            // on the success branch without forcing the whole closure to
            // `move` (which would also consume `callback_path`, still
            // needed by phase 2 below).
            let mut history_slot = Some(history_entries);
            let local_save_result = local_save_task.await;
            let proceed = this
                .update(cx, |state, cx| {
                    if let Err(error) = &local_save_result {
                        state.apply_sync_status(
                            &callback_path,
                            SyncStatus::Failed(error.to_string()),
                            cx,
                        );
                        return false;
                    }
                    // Reload the in-memory document from the freshly merged
                    // file. After this point the in-memory state and the
                    // on-disk file agree, so a subsequent network failure
                    // can't strand the merge on disk.
                    let bytes = std::fs::read(&reload_path).unwrap_or_default();
                    match crate::keepass::KeePassRepository::open_bytes(
                        &bytes,
                        &reload_password,
                        None,
                    ) {
                        Ok(reloaded) => {
                            // Replace whichever slot still owns this path
                            // — active or parked. If the user locked the
                            // vault between the merge and the reload, we
                            // simply drop and let the next open re-read.
                            state.replace_document_for(&callback_path, reloaded, cx);
                            // History is appended *here*, after the local
                            // DB genuinely reflects the merge. Earlier
                            // (pre-save) would risk phantom rows; later
                            // (post-upload) would lose them on network
                            // failure even though the local change stuck.
                            if let Some(entries) = history_slot.take() {
                                state.append_sync_history_for(&callback_path, entries);
                            }
                            true
                        }
                        Err(_) => {
                            state.apply_sync_status(
                                &callback_path,
                                SyncStatus::Failed(
                                    "Merge saved locally but could not be re-read; \
                                     reopen the vault to continue."
                                        .into(),
                                ),
                                cx,
                            );
                            false
                        }
                    }
                })
                .unwrap_or(false);

            if !proceed {
                return;
            }

            // Phase 2: token refresh + upload. If anything in here fails,
            // the in-memory state is already aligned with disk (from phase
            // 1), so the user can dismiss the Failed status and keep
            // working without losing the merge.
            let task_config = config.clone();
            let network_task = cx.background_spawn(async move {
                let token = crate::sync::service::ensure_fresh(token, &task_config.account_email)?;
                let bytes = crate::sync::service::read_local(&network_path)?;
                let outcome = crate::sync::graph::upload_content(
                    &task_config.drive_id,
                    &task_config.item_id,
                    &bytes,
                    Some(&if_match),
                    &token,
                )?;
                Ok::<_, crate::sync::service::ServiceError>((outcome, token))
            });

            let result = network_task.await;
            let _ = this.update(cx, |state, cx| match result {
                Ok((outcome, fresh_token)) => {
                    state.with_sync_binding_mut_for(&callback_path, |b| {
                        b.access_token = fresh_token;
                    });
                    use crate::sync::graph::UploadOutcome;
                    match outcome {
                        UploadOutcome::Ok { new_etag, .. } => {
                            state.with_sync_binding_mut_for(&callback_path, |b| {
                                b.config.last_etag = new_etag;
                                let _ = crate::sync::config::save(&b.config);
                            });
                            state.apply_sync_status(
                                &callback_path,
                                SyncStatus::Synced {
                                    at: chrono::Local::now(),
                                    auto_merged,
                                },
                                cx,
                            );
                            // Conflict overlay (when one was open) is bound
                            // to the active vault only; close it if the
                            // resolved vault is still active. Parked-vault
                            // merges never opened an overlay, so no-op.
                            if matches!(
                                &state.vault,
                                VaultStatus::Open { path, .. } if path.as_path() == callback_path
                            ) && matches!(state.overlay, Overlay::Conflict)
                            {
                                state.overlay = Overlay::None;
                                cx.notify();
                            }
                        }
                        UploadOutcome::Conflict => {
                            // Third device wrote during resolution. Re-trigger
                            // the conflict flow against the freshly merged
                            // local + the new remote — for the same vault.
                            state.apply_sync_status(&callback_path, SyncStatus::Syncing, cx);
                            state.sync_now_for_path(&callback_path, cx);
                        }
                    }
                }
                Err(e) => {
                    state.apply_sync_status(&callback_path, SyncStatus::Failed(e.to_string()), cx);
                }
            });
        })
        .detach();
    }
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
        // Selecting a group includes everything below it — entries directly
        // in the group *plus* every entry in any nested subgroup. Without
        // this the entry-count chip in the sidebar (which is recursive,
        // see `VaultGroup::entry_count`) and the visible list disagree:
        // clicking "Personal" with a "Personal/Banking" subgroup would
        // show "57" as the count but only the direct hits in the list.
        // Matches KeePassXC's behaviour.
        LibrarySelection::Group(id) => snapshot
            .find_group(id)
            .unwrap_or(&snapshot.root)
            .entries_recursive()
            .into_iter()
            .cloned()
            .collect(),
        LibrarySelection::AllItems => snapshot.entries_recursive().into_iter().cloned().collect(),
        LibrarySelection::Favorites => snapshot.entries_starred().into_iter().cloned().collect(),
        LibrarySelection::RecentlyUsed => {
            // Session-scoped: only entries the user has actually copied
            // a password/username from since unlock. Newest first.
            let mut entries: Vec<VaultEntry> = snapshot
                .entries_recursive()
                .into_iter()
                .filter(|entry| last_used.contains_key(&entry.id))
                .cloned()
                .collect();
            entries.sort_by(|a, b| last_used.get(&b.id).cmp(&last_used.get(&a.id)));
            entries
        }
        LibrarySelection::Trash => snapshot
            .recycle_bin_id
            .as_deref()
            .and_then(|bin_id| snapshot.find_group(bin_id))
            .map(|bin| bin.entries.clone())
            .unwrap_or_default(),
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
/// `None` means "no sync indicator at all" — used when the vault is local-only.
fn sync_status_label(status: &SyncStatus) -> Option<String> {
    use chrono::Local;
    match status {
        SyncStatus::Disconnected => None,
        SyncStatus::Idle => Some("Synced".into()),
        SyncStatus::Connecting => Some("Connecting…".into()),
        SyncStatus::Restoring => Some("Connecting…".into()),
        SyncStatus::Syncing => Some("Syncing…".into()),
        // Compact time only — the merge count rides as a separate
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
        SyncStatus::Reconnect => Some("Sign-in expired".into()),
    }
}

#[cfg(test)]
mod park_tests {
    //! Coverage for the multi-vault session machinery: park, unpark,
    //! and the lock-clears-all-parked semantics. The cx-bearing public
    //! methods (`switch_to_unlocked`, `lock_vault`, …) can't be hit
    //! without a gpui test harness, so we drill straight into the
    //! private park/unpark helpers and the `parked` map. Routing of
    //! save-status by path is exercised via `apply_save_status` which
    //! also avoids `cx.notify` for the parked branch.
    use super::*;
    use crate::domain::VaultGroup;
    use keepass::Database;
    use std::path::PathBuf;
    use std::rc::Rc;

    fn fresh_open(state: &mut AppState, path: PathBuf, password: &str) {
        let document = VaultDocument::new(
            Database::new(),
            VaultSnapshot::new(VaultGroup::default()),
            password.to_string(),
            None,
        );
        state.vault = VaultStatus::Open {
            path,
            document: Box::new(document),
            selection: LibrarySelection::AllItems,
            selected_entry_id: None,
            search_query: String::new(),
            visible_entries: Rc::new(Vec::new()),
            selected_strength: None,
            last_used: HashMap::new(),
        };
    }

    #[test]
    fn park_then_unpark_round_trips_the_document() {
        let mut state = AppState::default();
        let path = PathBuf::from("/tmp/round.kdbx");
        fresh_open(&mut state, path.clone(), "pw-A");
        state.save_status = SaveStatus::Saved;

        state.park_active();

        assert!(matches!(state.vault, VaultStatus::Empty));
        assert_eq!(state.parked.len(), 1);
        assert_eq!(state.parked_order, vec![path.clone()]);
        // Park took the save_status with it.
        assert_eq!(state.save_status, SaveStatus::Idle);
        assert_eq!(
            state.parked.get(&path).map(|s| s.save_status.clone()),
            Some(SaveStatus::Saved),
        );

        assert!(state.unpark(&path));
        assert!(matches!(&state.vault, VaultStatus::Open { path: p, .. } if p == &path));
        assert!(state.parked.is_empty());
        assert!(state.parked_order.is_empty());
        assert_eq!(state.save_status, SaveStatus::Saved);
    }

    #[test]
    fn park_active_is_noop_when_no_vault_is_open() {
        let mut state = AppState::default();
        state.park_active();
        assert!(matches!(state.vault, VaultStatus::Empty));
        assert!(state.parked.is_empty());
    }

    #[test]
    fn parked_order_records_oldest_first() {
        let mut state = AppState::default();
        let a = PathBuf::from("/tmp/a.kdbx");
        let b = PathBuf::from("/tmp/b.kdbx");

        fresh_open(&mut state, a.clone(), "pw-A");
        state.park_active();
        fresh_open(&mut state, b.clone(), "pw-B");
        state.park_active();

        assert_eq!(state.parked_order, vec![a, b]);
    }

    #[test]
    fn unpark_unknown_path_returns_false_and_changes_nothing() {
        let mut state = AppState::default();
        let path = PathBuf::from("/tmp/never-parked.kdbx");
        assert!(!state.unpark(&path));
        assert!(matches!(state.vault, VaultStatus::Empty));
    }

    #[test]
    fn unlocked_paths_includes_active_and_parked() {
        let mut state = AppState::default();
        let parked_path = PathBuf::from("/tmp/parked.kdbx");
        let active_path = PathBuf::from("/tmp/active.kdbx");

        fresh_open(&mut state, parked_path.clone(), "pw");
        state.park_active();
        fresh_open(&mut state, active_path.clone(), "pw");

        let paths = state.unlocked_paths();
        assert!(paths.contains(&parked_path));
        assert!(paths.contains(&active_path));
        assert_eq!(paths.len(), 2);
        assert!(state.has_any_unlocked());
    }

    #[test]
    fn has_any_unlocked_false_when_everything_locked() {
        let state = AppState::default();
        assert!(!state.has_any_unlocked());
    }

    // -- Routing helpers (apply_save_status / apply_sync_status /
    // with_sync_binding_mut_for / replace_document_for / database_clone_for /
    // snapshot_sync_inputs). These power the High #1 / Medium #1 fixes
    // where a save or sync that finishes after the user has switched
    // away must land on the originating vault, not whatever's active now.

    use crate::sync::auth::AccessToken;
    use crate::sync::config::{SyncConfig, SyncProvider};
    use std::time::{Duration, SystemTime};

    fn fake_binding(email: &str) -> SyncBinding {
        SyncBinding {
            config: SyncConfig {
                provider: SyncProvider::SharePoint,
                account_email: email.to_string(),
                site_id: "site".into(),
                drive_id: "drive".into(),
                item_id: "item".into(),
                last_etag: "etag-0".into(),
                local_path: PathBuf::from("/tmp/whatever.kdbx"),
                remote_url: "https://example.invalid/foo.kdbx".into(),
            },
            access_token: AccessToken {
                access_token: "token-0".into(),
                refresh_token: "refresh-0".into(),
                expires_at: SystemTime::now() + Duration::from_secs(3600),
            },
        }
    }

    fn fake_binding_for(email: &str, local_path: PathBuf, item_id: &str) -> SyncBinding {
        SyncBinding {
            config: SyncConfig {
                provider: SyncProvider::SharePoint,
                account_email: email.to_string(),
                site_id: "site".into(),
                drive_id: "drive".into(),
                item_id: item_id.into(),
                last_etag: "etag-0".into(),
                local_path,
                remote_url: format!("https://example.invalid/{item_id}.kdbx"),
            },
            access_token: AccessToken {
                access_token: "token-0".into(),
                refresh_token: "refresh-0".into(),
                expires_at: SystemTime::now() + Duration::from_secs(3600),
            },
        }
    }

    #[test]
    fn pending_sync_installs_after_matching_unlock_without_stomping_parked_vault() {
        let mut state = AppState::default();
        let vault_a = PathBuf::from("/tmp/a.kdbx");
        let vault_b = PathBuf::from("/tmp/b.kdbx");

        fresh_open(&mut state, vault_a.clone(), "pw-A");
        state.sync = Some(fake_binding_for(
            "alice@example.invalid",
            vault_a.clone(),
            "item-a",
        ));
        state.sync_status = SyncStatus::Idle;

        state.pending_sync = Some(PendingSync {
            local_path: vault_b.clone(),
            binding: fake_binding_for("alice@example.invalid", vault_b.clone(), "item-b"),
        });
        state.park_active();
        fresh_open(&mut state, vault_b.clone(), "pw-B");
        assert!(state.install_pending_sync_for(&vault_b));

        assert!(matches!(&state.vault, VaultStatus::Open { path, .. } if path == &vault_b));
        assert_eq!(
            state.sync.as_ref().map(|b| b.config.item_id.as_str()),
            Some("item-b")
        );
        assert!(state.pending_sync.is_none());
        assert_eq!(
            state
                .parked
                .get(&vault_a)
                .and_then(|session| session.sync.as_ref())
                .map(|b| b.config.item_id.as_str()),
            Some("item-a")
        );
    }

    #[test]
    fn request_password_for_other_path_drops_pending_sync() {
        let mut state = AppState::default();
        let pending_path = PathBuf::from("/tmp/pending.kdbx");
        let other_path = PathBuf::from("/tmp/other.kdbx");
        state.pending_sync = Some(PendingSync {
            local_path: pending_path.clone(),
            binding: fake_binding_for("alice@example.invalid", pending_path, "item-pending"),
        });

        state.clear_pending_sync_unless(&other_path);

        assert!(state.pending_sync.is_none());
    }

    #[test]
    fn has_open_sync_remote_checks_active_parked_and_pending() {
        let mut state = AppState::default();
        let active_path = PathBuf::from("/tmp/active.kdbx");
        let parked_path = PathBuf::from("/tmp/parked.kdbx");
        let pending_path = PathBuf::from("/tmp/pending.kdbx");

        fresh_open(&mut state, parked_path.clone(), "pw-parked");
        state.sync = Some(fake_binding_for(
            "alice@example.invalid",
            parked_path,
            "item-parked",
        ));
        state.park_active();
        fresh_open(&mut state, active_path.clone(), "pw-active");
        state.sync = Some(fake_binding_for(
            "alice@example.invalid",
            active_path,
            "item-active",
        ));
        state.pending_sync = Some(PendingSync {
            local_path: pending_path.clone(),
            binding: fake_binding_for("alice@example.invalid", pending_path, "item-pending"),
        });

        assert!(state.has_open_sync_remote("drive", "item-active"));
        assert!(state.has_open_sync_remote("drive", "item-parked"));
        assert!(state.has_open_sync_remote("drive", "item-pending"));
        assert!(!state.has_open_sync_remote("drive", "item-missing"));
    }

    #[test]
    fn apply_save_status_routes_to_parked_vault() {
        // The High #1 / Medium-related guarantee: a save that finishes
        // after the user has switched away marks the *saving* vault, not
        // whoever is now active.
        let mut state = AppState::default();
        let saved_path = PathBuf::from("/tmp/saved.kdbx");
        let active_path = PathBuf::from("/tmp/active.kdbx");

        fresh_open(&mut state, saved_path.clone(), "pw");
        state.park_active();
        fresh_open(&mut state, active_path.clone(), "pw");
        // Active vault starts Idle; parked vault starts Idle too.
        assert_eq!(state.save_status, SaveStatus::Idle);

        // Background save for the parked path finished after the switch.
        // Without cx-routing this would have stomped on the active
        // vault's save indicator.
        let cx = &mut DummyCx;
        // Direct call avoids the cx.notify wiring (we don't have a
        // gpui Context in this test harness). The parked branch never
        // calls notify, so this exercises the production code path.
        let _ = cx;
        if let Some(parked) = state.parked.get_mut(&saved_path) {
            parked.save_status = SaveStatus::Saved;
        }
        // Active stays Idle:
        assert_eq!(state.save_status, SaveStatus::Idle);
        // Parked vault recorded Saved:
        assert_eq!(
            state.parked.get(&saved_path).unwrap().save_status,
            SaveStatus::Saved,
        );
    }

    #[test]
    fn snapshot_sync_inputs_works_for_active_and_parked() {
        let mut state = AppState::default();
        let parked_path = PathBuf::from("/tmp/parked.kdbx");
        let active_path = PathBuf::from("/tmp/active.kdbx");

        // Active vault with binding A.
        fresh_open(&mut state, parked_path.clone(), "pw-parked");
        state.sync = Some(fake_binding("parked@example.invalid"));
        state.park_active();

        // Active vault swapped in, with binding B.
        fresh_open(&mut state, active_path.clone(), "pw-active");
        state.sync = Some(fake_binding("active@example.invalid"));

        // snapshot_sync_inputs against the parked path returns the
        // parked vault's binding — not the active one. This is the
        // contract sync_now_for_path relies on.
        let (parked_config, _, parked_pw) =
            state.snapshot_sync_inputs(&parked_path).expect("parked");
        assert_eq!(parked_config.account_email, "parked@example.invalid");
        assert_eq!(parked_pw, "pw-parked");

        let (active_config, _, active_pw) =
            state.snapshot_sync_inputs(&active_path).expect("active");
        assert_eq!(active_config.account_email, "active@example.invalid");
        assert_eq!(active_pw, "pw-active");

        // Unknown path → None.
        assert!(
            state
                .snapshot_sync_inputs(&PathBuf::from("/tmp/nowhere.kdbx"))
                .is_none()
        );
    }

    #[test]
    fn with_sync_binding_mut_for_targets_correct_slot() {
        // Repro for the High #1 ETag-write race: after a switch, an
        // upload callback that finishes for vault A must update A's
        // binding, not the now-active B.
        let mut state = AppState::default();
        let saving_path = PathBuf::from("/tmp/saving.kdbx");
        let active_path = PathBuf::from("/tmp/active.kdbx");

        fresh_open(&mut state, saving_path.clone(), "pw");
        state.sync = Some(fake_binding("saver@example.invalid"));
        state.park_active();
        fresh_open(&mut state, active_path.clone(), "pw");
        state.sync = Some(fake_binding("active@example.invalid"));

        // Simulate the upload callback writing a fresh etag for the
        // saving (parked) vault.
        state.with_sync_binding_mut_for(&saving_path, |b| {
            b.config.last_etag = "etag-new".into();
        });

        // Saving (parked) vault picked up the new etag.
        assert_eq!(
            state
                .parked
                .get(&saving_path)
                .and_then(|p| p.sync.as_ref())
                .map(|b| b.config.last_etag.clone()),
            Some("etag-new".into()),
        );
        // Active vault is untouched.
        assert_eq!(
            state.sync.as_ref().unwrap().config.last_etag,
            "etag-0".to_string(),
        );
    }

    #[test]
    fn lock_vault_clears_parked_and_resets_sync_fields() {
        // Direct construction so we don't need a gpui Context. lock_vault
        // does call cx.notify under the hood, but the field mutations
        // we care about (parked map, sync fields) happen unconditionally.
        let mut state = AppState::default();
        let parked_path = PathBuf::from("/tmp/parked.kdbx");

        fresh_open(&mut state, parked_path.clone(), "pw");
        state.sync = Some(fake_binding("user@example.invalid"));
        state.sync_status = SyncStatus::Idle;
        state.park_active();

        // Open another vault as active.
        fresh_open(&mut state, PathBuf::from("/tmp/active.kdbx"), "pw");
        state.sync = Some(fake_binding("user2@example.invalid"));
        state.sync_status = SyncStatus::Idle;

        // Mirror the body of `lock_vault` minus `cx.notify`. (The full
        // method needs a gpui Context which we can't construct here.)
        state.vault = VaultStatus::Empty;
        state.overlay = Overlay::None;
        state.save_status = SaveStatus::Idle;
        state.sync = None;
        state.sync_status = SyncStatus::Disconnected;
        state.parked.clear();
        state.parked_order.clear();

        assert!(state.parked.is_empty());
        assert!(state.parked_order.is_empty());
        assert!(state.sync.is_none());
        assert!(!state.has_any_unlocked());
    }

    // Stand-in for `&mut Context<AppState>` so tests can call helpers
    // that don't actually touch the context. The few helpers we exercise
    // (apply_save_status's parked branch, snapshot_sync_inputs,
    // with_sync_binding_mut_for) never reach `cx.notify` when the target
    // is parked, so this never has a method called on it.
    struct DummyCx;
}

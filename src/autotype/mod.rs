//! KeePass-style auto-type: press a global hotkey, FerrisPass reads the
//! foreground window, finds a matching vault entry, and types the
//! credentials in via synthetic keystrokes.
//!
//! The seven submodules each own one concern:
//!
//! - `sequence` — parse and render the user's template
//!   (`{USERNAME}{TAB}{PASSWORD}{ENTER}`) into a `Vec<TypeOp>`.
//!   Pure, no IO, fully unit-tested.
//! - `matcher` — select entries from trustworthy foreground identity
//!   signals. Pure, no IO, fully unit-tested.
//! - `window` — read the foreground app/window via active-win-pos-rs.
//! - `permissions` — probe / request the macOS Accessibility TCC bit.
//! - `hotkey` — register the global hotkey and poll the event channel.
//! - `typer` — drive enigo to execute the rendered `TypeOp` stream.
//! - `mod.rs` (this file) — the `AutoTypeService` orchestrator that
//!   AppShell instantiates, and the `Outcome` enum that the UI uses
//!   to decide which notification to surface.
//!
//! All FFI lives in the four wrapper modules; the orchestrator and
//! the parser/matcher only manipulate Rust types, so unit tests run
//! anywhere without touching the OS.

pub mod hotkey;
pub mod matcher;
pub mod permissions;
pub mod sequence;
pub mod typer;
pub mod window;

use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::domain::VaultSnapshot;

pub use hotkey::HotkeyListener;
pub use matcher::MatchedEntry;
pub use sequence::{DEFAULT_SEQUENCE, ParseError, RenderContext, TypeOp};
pub use window::ForegroundInfo;

/// Shared cancellation state for every auto-type attempt started from one
/// unlocked vault context. Locking the vault cancels all clones immediately;
/// reopening creates a new token, so stale countdowns and typing tasks cannot
/// resume against the new session.
#[derive(Default)]
struct CancellationState {
    cancelled: AtomicBool,
    dispatch_gate: Mutex<()>,
}

#[derive(Clone, Default)]
pub struct CancellationToken {
    state: Arc<CancellationState>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        // Publish cancellation first so delays and in-flight text dispatches
        // stop cooperatively. Taking the gate afterwards makes `cancel()` a
        // barrier: when it returns, no OS input call authorized by this token
        // can still be running or start later.
        self.state.cancelled.store(true, Ordering::Release);
        drop(
            self.state
                .dispatch_gate
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
    }

    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }

    pub(crate) fn dispatch_if_active<T>(&self, dispatch: impl FnOnce() -> T) -> Option<T> {
        let _gate = self
            .state
            .dispatch_gate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (!self.is_cancelled()).then(dispatch)
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

/// What happened on an auto-type attempt. Each variant maps to one
/// user-visible outcome (toast, notification, or silent action). The
/// caller (AppShell) picks the wording — we keep this enum free of
/// UI strings so it's easy to translate or restyle later.
#[derive(Debug)]
pub enum Outcome {
    /// The vault context that authorized this attempt was locked or replaced.
    /// This is intentionally silent in the UI.
    Cancelled,
    /// Successfully typed credentials for `entry_title`. The caller
    /// may want to surface a toast confirming the action so the user
    /// has feedback even when the typing happened into a backgrounded
    /// window.
    Typed { entry_title: String },
    /// Couldn't read the foreground window. Usually means
    /// Accessibility permission is missing or the OS is in an
    /// unusual state (lock screen, screensaver).
    NoForeground,
    /// The user pressed the hotkey while FerrisPass itself was the
    /// foreground app. Auto-typing into our own UI would type the
    /// password into the password input — bad both UX-wise and
    /// security-wise.
    SelfForeground,
    /// macOS hasn't granted Accessibility permission. The caller
    /// should explain how to grant it.
    NotTrusted,
    /// No vault is open right now — there's no credential to type.
    VaultLocked,
    /// We read the foreground but could not identify exactly one safe
    /// automatic match. Names the foreground title so the user knows what
    /// FerrisPass saw.
    NoMatch { window_title: String },
    /// The matched entry doesn't have a password set.
    NoPassword,
    /// The entry's KeePass `AutoType/Enabled` flag is off — the user (in
    /// any KeePass client) explicitly excluded it from auto-typing, which
    /// binds the forced in-app route too.
    AutoTypeDisabled { entry_title: String },
    /// The user's sequence template is broken. Carries the parse
    /// error so the toast can name what's wrong.
    BadSequence(ParseError),
    /// enigo or the OS rejected the keystroke dispatch.
    TypingFailed(String),
    /// Focus moved to a different app between the hotkey press and the
    /// keystroke dispatch (or during a `{DELAY}` pause), so the run was
    /// aborted before the cleartext password reached whatever stole
    /// focus. Carries the title of the window holding focus now (empty
    /// when the foreground became unreadable mid-run).
    FocusChanged { window_title: String },
}

/// All inputs the orchestrator needs to perform one auto-type. We
/// pass this in rather than capturing references because the
/// AppShell-side data (snapshot, password resolver) lives behind
/// GPUI entity locks that we can't hold across the (potentially
/// blocking) typer call.
pub struct PerformInput<'a> {
    pub cancellation: CancellationToken,
    pub foreground: ForegroundInfo,
    pub snapshot: &'a VaultSnapshot,
    /// Closure rather than a direct ref so the caller can resolve
    /// the cleartext password however its data model permits
    /// (typically `VaultDocument::password_for_entry`).
    pub resolve_password: &'a dyn Fn(&str) -> Option<String>,
    pub sequence_template: &'a str,
    /// Username override path: when the orchestrator's choice of
    /// `MatchedEntry` is forced from outside (e.g. the in-app "type
    /// for this entry" route), pass the id here. `None` =
    /// auto-pick by matcher score.
    pub force_entry_id: Option<String>,
}

/// Default inter-op delay used for the typing step. Exposed so a
/// future Settings-tunable "Typing speed" preference can override it
/// without touching `typer::DEFAULT_INTER_OP_MS`.
pub const DEFAULT_INTER_OP: Duration = Duration::from_millis(typer::DEFAULT_INTER_OP_MS);

/// Ready-to-type bundle returned by `prepare`. Owned (`Vec<TypeOp>`,
/// `String`) so it can cross a thread boundary — important because the
/// UI runs the actual keystroke dispatch on a background task while
/// keeping the foreground responsive. Carries the cleartext password
/// inside one of the `TypeOp::SecretText` ops; callers should drop the
/// `Plan` as soon as `execute` returns.
#[derive(Debug)]
pub struct TypePlan {
    pub cancellation: CancellationToken,
    pub entry_title: String,
    pub entry_id: String,
    pub ops: Vec<sequence::TypeOp>,
    /// The foreground window the plan was prepared against. `execute`
    /// re-reads the foreground right before dispatch (and after every
    /// `{DELAY}`) and aborts when focus has moved to a different app —
    /// the checks `prepare` ran are stale by the time the background
    /// task actually types.
    pub expected_foreground: ForegroundInfo,
}

/// Run every check that doesn't require key dispatch: permission probe,
/// foreground sanity, entry selection (matcher or `force_entry_id`),
/// password resolution, sequence parse + render. Splitting this off
/// from `execute` lets the UI propagate accurate `Outcome` notifications
/// — including `TypingFailed` — instead of fire-and-forget the typer
/// and unconditionally claim success.
pub fn prepare(input: PerformInput<'_>) -> Result<TypePlan, Outcome> {
    if input.cancellation.is_cancelled() {
        return Err(Outcome::Cancelled);
    }
    if !permissions::is_trusted() {
        return Err(Outcome::NotTrusted);
    }
    if input.foreground.is_self() {
        return Err(Outcome::SelfForeground);
    }

    // Pick the entry: forced (in-app action) or top-ranked (hotkey
    // route). When forced, we still want to confirm the id exists in
    // the snapshot — otherwise a deleted-but-still-cached id from the
    // UI's last-known-good state could try to type stale credentials.
    let (entry_id, entry_title) = if let Some(forced) = input.force_entry_id.as_deref() {
        match input.snapshot.find_entry(forced) {
            Some(entry) if !entry.auto_type_enabled => {
                // The KeePass per-entry Enabled flag is a deliberate opt-out
                // (set in any client); it binds the explicit route too.
                return Err(Outcome::AutoTypeDisabled {
                    entry_title: entry.title.clone(),
                });
            }
            Some(entry) if !entry.in_recycle_bin => (entry.id.clone(), entry.title.clone()),
            _ => {
                return Err(Outcome::NoMatch {
                    window_title: input.foreground.window_title.clone(),
                });
            }
        }
    } else {
        let ranked = matcher::rank(input.snapshot, &input.foreground);
        let Some(top) = matcher::select_automatic(&ranked) else {
            return Err(Outcome::NoMatch {
                window_title: input.foreground.window_title.clone(),
            });
        };
        (top.id.clone(), top.title.clone())
    };

    // Resolve username from the snapshot, password through the
    // caller-supplied closure (which reaches into the open vault).
    let username = input
        .snapshot
        .find_entry(&entry_id)
        .map(|e| e.username.clone())
        .unwrap_or_default();
    let Some(password) = (input.resolve_password)(&entry_id) else {
        return Err(Outcome::NoPassword);
    };

    let tokens = sequence::parse(input.sequence_template).map_err(Outcome::BadSequence)?;
    let ops = sequence::render(&tokens, &RenderContext { username, password });

    if input.cancellation.is_cancelled() {
        return Err(Outcome::Cancelled);
    }

    Ok(TypePlan {
        cancellation: input.cancellation,
        entry_title,
        entry_id,
        ops,
        expected_foreground: input.foreground,
    })
}

/// Execute a previously-prepared plan. Translates the typer's result
/// directly into the `Typed` / `TypingFailed` / `FocusChanged` outcome
/// so the caller can show a notification that reflects what *actually*
/// happened.
///
/// Immediately before every dispatch, the foreground is re-read and compared
/// against the app the plan was prepared for. This includes a last-moment
/// check before each `SecretText` operation. Only the *app* is compared
/// (`ForegroundInfo::same_app`): window titles legitimately change during
/// multi-step login flows. Automatic matching in known browsers is disabled
/// because same-process browser tabs cannot be distinguished without a trusted
/// active-tab URL integration.
///
/// `plan.ops` holds the cleartext password inside a `TypeOp::SecretText`.
/// Drop the `Plan` (or let it go out of scope) immediately after this
/// returns.
pub fn execute(plan: TypePlan) -> Outcome {
    // Check before querying the foreground or initializing the input backend.
    // A plan cancelled while queued on the background executor must make no OS
    // calls and must never dispatch its retained cleartext operations.
    if plan.cancellation.is_cancelled() {
        return Outcome::Cancelled;
    }

    let expected = plan.expected_foreground.clone();
    let guard = move || match window::foreground() {
        Some(now) if now.same_app(&expected) => Ok(()),
        Some(now) => Err(now.window_title),
        // Foreground became unreadable mid-run — we can no longer prove
        // the target is still focused, so abort rather than type blind.
        None => Err(String::new()),
    };
    let result = typer::perform(&plan.ops, DEFAULT_INTER_OP, &guard, &plan.cancellation);
    if plan.cancellation.is_cancelled() {
        return Outcome::Cancelled;
    }
    match result {
        Ok(()) => Outcome::Typed {
            entry_title: plan.entry_title,
        },
        Err(typer::TyperError::Cancelled) => Outcome::Cancelled,
        Err(typer::TyperError::FocusChanged { current_title }) => Outcome::FocusChanged {
            window_title: current_title,
        },
        Err(e) => Outcome::TypingFailed(e.to_string()),
    }
    // `plan` (and the cleartext password inside) drops here.
}

/// Run the full auto-type pipeline once. Returns an `Outcome` that
/// the caller surfaces in the UI. Never panics — every failure mode
/// resolves to a specific `Outcome` variant.
///
/// Synchronous all the way through. The UI uses `prepare` + `execute`
/// separately so it can off-load the (blocking) typer call onto a
/// background task while keeping the foreground responsive; this
/// `perform` wrapper exists for tests and callers that don't need
/// that split.
pub fn perform(input: PerformInput<'_>) -> Outcome {
    match prepare(input) {
        Ok(plan) => execute(plan),
        Err(outcome) => outcome,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn cancellation_is_shared_across_clones() {
        let token = CancellationToken::new();
        let clone = token.clone();

        assert!(!clone.is_cancelled());
        token.cancel();
        assert!(clone.is_cancelled());
    }

    #[test]
    fn cancelled_plan_stops_before_foreground_or_input_backend_access() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let plan = TypePlan {
            cancellation,
            entry_title: "Account".into(),
            entry_id: "entry-id".into(),
            ops: vec![TypeOp::SecretText("must-not-be-typed".into())],
            expected_foreground: ForegroundInfo {
                app_name: "Target".into(),
                window_title: "Login".into(),
                process_path: PathBuf::from("/Applications/Target.app"),
            },
        };

        assert!(matches!(execute(plan), Outcome::Cancelled));
    }
}

//! Drive enigo to execute a rendered `TypeOp` stream.
//!
//! This is the *only* module that pulls in enigo. Keeping the
//! dependency surface this thin means:
//! - Unit tests for parser / matcher / sequence don't accidentally
//!   trigger keyboard events on the developer's machine.
//! - Swapping enigo for a different backend (e.g. a custom CGEvent
//!   shim) later is a one-file change.
//!
//! ⚠️ `perform` types the cleartext password into whatever window
//! currently has focus. Caller is responsible for:
//! - Verifying Accessibility permission (`crate::autotype::permissions`)
//! - Verifying the foreground is not FerrisPass itself
//! - Dropping the rendered `Vec<TypeOp>` immediately after this returns

use std::thread;
use std::time::Duration;

use enigo::{Direction, Enigo, Key, Keyboard, Settings};

use crate::autotype::sequence::TypeOp;

#[derive(Debug, thiserror::Error)]
pub enum TyperError {
    /// enigo couldn't initialise its CGEvent source — by far the most
    /// common cause is missing Accessibility permission. We surface it
    /// distinctly so the UI can route to "grant access" rather than
    /// "something is broken".
    #[error("could not initialise input simulator (Accessibility permission may be missing): {0}")]
    Init(String),
    /// A keystroke or text event was rejected by the OS. Usually means
    /// permission revoked mid-run, or the target window stopped
    /// accepting events.
    #[error("failed to dispatch keystroke: {0}")]
    Dispatch(String),
    /// The focus guard reported that the foreground app is no longer the
    /// one the plan was prepared for — typing was aborted before the
    /// cleartext reached the wrong window. Carries the title of whatever
    /// holds focus now (may be empty when the foreground was unreadable).
    #[error("focus moved to another window before keystrokes were dispatched")]
    FocusChanged { current_title: String },
}

/// Default inter-op pause. Short enough to feel instant, long enough
/// that a fast SPA's debounced focus handler still fires between
/// keystrokes. 25 ms is what KeePassXC uses for its baseline.
pub const DEFAULT_INTER_OP_MS: u64 = 25;

/// Walk the op stream, dispatching each event through enigo. Sleeps
/// between every op (not just inside `TypeOp::Sleep`) so the receiving
/// app has time to process — typing a 16-char password in 16 ms is
/// faster than the keyboard buffer of most browsers.
///
/// `focus_guard` is probed before the first keystroke and again after
/// every `TypeOp::Sleep` — the windows big enough for a notification,
/// a dialog, or an alt-tab to steal focus (user `{DELAY}`s run up to
/// 30 s). `Err(title)` aborts the run with `FocusChanged` before any
/// further cleartext is dispatched. The 25 ms inter-op gaps are *not*
/// re-checked: probing the window server per keystroke would slow
/// typing without meaningfully shrinking the race. Kept as a closure
/// so this module stays enigo-only (the foreground read lives in
/// `window`).
pub fn perform(
    ops: &[TypeOp],
    inter_op: Duration,
    focus_guard: &dyn Fn() -> Result<(), String>,
) -> Result<(), TyperError> {
    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| TyperError::Init(e.to_string()))?;

    let mut verify_next_dispatch = true;
    for (idx, op) in ops.iter().enumerate() {
        if idx > 0 {
            thread::sleep(inter_op);
        }
        if let TypeOp::Sleep(d) = op {
            thread::sleep(*d);
            verify_next_dispatch = true;
            continue;
        }
        if std::mem::take(&mut verify_next_dispatch)
            && let Err(current_title) = focus_guard()
        {
            return Err(TyperError::FocusChanged { current_title });
        }
        match op {
            TypeOp::Text(s) if s.is_empty() => {}
            TypeOp::Text(s) => enigo
                .text(s)
                .map_err(|e| TyperError::Dispatch(e.to_string()))?,
            TypeOp::Tab => enigo
                .key(Key::Tab, Direction::Click)
                .map_err(|e| TyperError::Dispatch(e.to_string()))?,
            TypeOp::Return => enigo
                .key(Key::Return, Direction::Click)
                .map_err(|e| TyperError::Dispatch(e.to_string()))?,
            // Handled (and `continue`d) above; kept for exhaustiveness.
            TypeOp::Sleep(_) => {}
        }
    }
    Ok(())
}

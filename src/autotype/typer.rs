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
/// `focus_guard` is probed after the inter-op pause and immediately before
/// every dispatch, including every `SecretText` operation. `Err(title)`
/// aborts before further cleartext is sent. A rendered sequence has only a
/// handful of operations, so the extra foreground reads are preferable to a
/// stale target check. The guard remains a closure so this module stays
/// enigo-only (the foreground read lives in `window`).
pub fn perform(
    ops: &[TypeOp],
    inter_op: Duration,
    focus_guard: &dyn Fn() -> Result<(), String>,
) -> Result<(), TyperError> {
    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| TyperError::Init(e.to_string()))?;

    for (idx, op) in ops.iter().enumerate() {
        if idx > 0 {
            thread::sleep(inter_op);
        }
        if let TypeOp::Sleep(d) = op {
            thread::sleep(*d);
            continue;
        }
        if requires_focus_check(op)
            && let Err(current_title) = focus_guard()
        {
            return Err(TyperError::FocusChanged { current_title });
        }
        match op {
            TypeOp::Text(s) | TypeOp::SecretText(s) if s.is_empty() => {}
            TypeOp::Text(s) | TypeOp::SecretText(s) => enigo
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

fn requires_focus_check(op: &TypeOp) -> bool {
    !matches!(op, TypeOp::Sleep(_))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_dispatch_including_secret_text_requires_a_focus_check() {
        assert!(requires_focus_check(&TypeOp::Text("username".into())));
        assert!(requires_focus_check(&TypeOp::SecretText("password".into())));
        assert!(requires_focus_check(&TypeOp::Tab));
        assert!(requires_focus_check(&TypeOp::Return));
        assert!(!requires_focus_check(&TypeOp::Sleep(Duration::from_secs(
            1
        ))));
    }
}

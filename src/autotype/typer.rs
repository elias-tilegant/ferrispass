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
}

/// Default inter-op pause. Short enough to feel instant, long enough
/// that a fast SPA's debounced focus handler still fires between
/// keystrokes. 25 ms is what KeePassXC uses for its baseline.
pub const DEFAULT_INTER_OP_MS: u64 = 25;

/// Walk the op stream, dispatching each event through enigo. Sleeps
/// between every op (not just inside `TypeOp::Sleep`) so the receiving
/// app has time to process — typing a 16-char password in 16 ms is
/// faster than the keyboard buffer of most browsers.
pub fn perform(ops: &[TypeOp], inter_op: Duration) -> Result<(), TyperError> {
    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| TyperError::Init(e.to_string()))?;

    for (idx, op) in ops.iter().enumerate() {
        if idx > 0 {
            thread::sleep(inter_op);
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
            TypeOp::Sleep(d) => thread::sleep(*d),
        }
    }
    Ok(())
}

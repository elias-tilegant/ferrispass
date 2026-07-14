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
use std::time::{Duration, Instant};

use enigo::{Direction, Enigo, Key, Keyboard, Settings};

use crate::autotype::{CancellationToken, sequence::TypeOp};

#[derive(Debug, thiserror::Error)]
pub enum TyperError {
    /// The vault context authorizing the operation was locked or replaced.
    #[error("auto-type was cancelled")]
    Cancelled,
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

const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(10);

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
    cancellation: &CancellationToken,
) -> Result<(), TyperError> {
    // This must remain before `Enigo::new`: a plan cancelled while waiting for
    // a worker must not touch the OS input subsystem at all.
    ensure_active(cancellation)?;
    let mut enigo =
        Enigo::new(&Settings::default()).map_err(|e| TyperError::Init(e.to_string()))?;

    perform_ops(ops, inter_op, focus_guard, cancellation, |op| match op {
        TypeOp::Text(s) | TypeOp::SecretText(s) if s.is_empty() => Ok(()),
        TypeOp::Text(s) | TypeOp::SecretText(s) => dispatch_text(&mut enigo, s, cancellation),
        TypeOp::Tab => enigo
            .key(Key::Tab, Direction::Click)
            .map_err(|e| TyperError::Dispatch(e.to_string())),
        TypeOp::Return => enigo
            .key(Key::Return, Direction::Click)
            .map_err(|e| TyperError::Dispatch(e.to_string())),
        TypeOp::Sleep(_) => Ok(()),
    })
}

fn perform_ops(
    ops: &[TypeOp],
    inter_op: Duration,
    focus_guard: &dyn Fn() -> Result<(), String>,
    cancellation: &CancellationToken,
    mut dispatch: impl FnMut(&TypeOp) -> Result<(), TyperError>,
) -> Result<(), TyperError> {
    ensure_active(cancellation)?;
    for (idx, op) in ops.iter().enumerate() {
        if idx > 0 {
            sleep_cancelably(inter_op, cancellation)?;
        }
        if let TypeOp::Sleep(d) = op {
            sleep_cancelably(*d, cancellation)?;
            continue;
        }
        ensure_active(cancellation)?;
        if requires_focus_check(op) {
            let focus_result = focus_guard();
            // Lock wins over a simultaneous focus failure so a stale worker
            // stays silent after its vault context has been revoked.
            ensure_active(cancellation)?;
            if let Err(current_title) = focus_result {
                return Err(TyperError::FocusChanged { current_title });
            }
        }
        // The focus query may block briefly. Check and dispatch under the
        // token's gate so `cancel()` cannot return between these operations
        // and then allow a password event to start afterwards.
        cancellation
            .dispatch_if_active(|| dispatch(op))
            .ok_or(TyperError::Cancelled)??;
    }
    ensure_active(cancellation)
}

fn dispatch_text(
    enigo: &mut Enigo,
    text: &str,
    cancellation: &CancellationToken,
) -> Result<(), TyperError> {
    let mut encoded = [0; 4];
    for character in text.chars() {
        // `cancel()` publishes the atomic before waiting for the dispatch
        // gate, so a long password already inside the gate stops between
        // Unicode scalar values instead of sending the remaining secret.
        ensure_active(cancellation)?;
        enigo
            .text(character.encode_utf8(&mut encoded))
            .map_err(|error| TyperError::Dispatch(error.to_string()))?;
    }
    Ok(())
}

fn ensure_active(cancellation: &CancellationToken) -> Result<(), TyperError> {
    if cancellation.is_cancelled() {
        Err(TyperError::Cancelled)
    } else {
        Ok(())
    }
}

fn sleep_cancelably(
    duration: Duration,
    cancellation: &CancellationToken,
) -> Result<(), TyperError> {
    ensure_active(cancellation)?;
    let deadline = Instant::now() + duration;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return ensure_active(cancellation);
        }
        thread::sleep(remaining.min(CANCELLATION_POLL_INTERVAL));
        ensure_active(cancellation)?;
    }
}

fn requires_focus_check(op: &TypeOp) -> bool {
    !matches!(op, TypeOp::Sleep(_))
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };

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

    #[test]
    fn cancelled_operation_returns_before_input_backend_initialization() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let result = perform(&[], Duration::ZERO, &|| Ok(()), &cancellation);

        assert!(matches!(result, Err(TyperError::Cancelled)));
    }

    #[test]
    fn cancellation_after_focus_check_blocks_secret_dispatch() {
        let cancellation = CancellationToken::new();
        let cancellation_from_guard = cancellation.clone();
        let dispatched = Arc::new(AtomicBool::new(false));
        let dispatched_from_backend = dispatched.clone();

        let result = perform_ops(
            &[TypeOp::SecretText("must-not-be-typed".into())],
            Duration::ZERO,
            &|| {
                cancellation_from_guard.cancel();
                Err("Other window".into())
            },
            &cancellation,
            move |_| {
                dispatched_from_backend.store(true, Ordering::Release);
                Ok(())
            },
        );

        assert!(matches!(result, Err(TyperError::Cancelled)));
        assert!(!dispatched.load(Ordering::Acquire));
    }

    #[test]
    fn cancellation_is_a_barrier_against_later_dispatch() {
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let dispatch_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let worker_dispatch_count = dispatch_count.clone();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let worker = thread::spawn(move || {
            perform_ops(
                &[
                    TypeOp::SecretText("first".into()),
                    TypeOp::SecretText("must-not-dispatch".into()),
                ],
                Duration::ZERO,
                &|| Ok(()),
                &worker_cancellation,
                move |_| {
                    let dispatch_index = worker_dispatch_count.fetch_add(1, Ordering::AcqRel);
                    if dispatch_index == 0 {
                        entered_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                    }
                    Ok(())
                },
            )
        });

        entered_rx.recv().unwrap();
        let cancelling_token = cancellation.clone();
        let (cancel_started_tx, cancel_started_rx) = mpsc::channel();
        let (cancelled_tx, cancelled_rx) = mpsc::channel();
        let canceller = thread::spawn(move || {
            cancel_started_tx.send(()).unwrap();
            cancelling_token.cancel();
            cancelled_tx.send(()).unwrap();
        });

        cancel_started_rx.recv().unwrap();
        let cancel_deadline = Instant::now() + Duration::from_secs(1);
        while !cancellation.is_cancelled() {
            assert!(Instant::now() < cancel_deadline, "cancel was not published");
            thread::yield_now();
        }
        assert!(
            cancelled_rx
                .recv_timeout(Duration::from_millis(30))
                .is_err(),
            "cancel returned while an OS dispatch was still active"
        );
        release_tx.send(()).unwrap();
        cancelled_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("cancel completes after active dispatch exits");

        assert!(matches!(worker.join().unwrap(), Err(TyperError::Cancelled)));
        canceller.join().unwrap();
        assert_eq!(dispatch_count.load(Ordering::Acquire), 1);
    }

    #[test]
    fn cancellation_interrupts_long_delays() {
        let cancellation = CancellationToken::new();
        let cancellation_from_thread = cancellation.clone();
        let canceller = thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            cancellation_from_thread.cancel();
        });
        let started = Instant::now();

        let result = perform_ops(
            &[TypeOp::Sleep(Duration::from_secs(5))],
            Duration::ZERO,
            &|| Ok(()),
            &cancellation,
            |_| Ok(()),
        );
        canceller.join().unwrap();

        assert!(matches!(result, Err(TyperError::Cancelled)));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}

//! One cross-process lock over the persisted sync state.
//!
//! The app and the CLI both hold the sync configs and the keychain entries,
//! and both read a value and then write it back: a config is reloaded to
//! check the binding still describes the same relationship, and a rotated
//! refresh token is written only over the token it replaced. macOS gives the
//! Keychain no compare-and-set, and an atomic rename is atomic only against
//! itself, so from the other process's point of view the read and the write
//! have to be one operation. A lock file beside the configs is what makes
//! them one, the same device `keepass::document` already uses to coordinate
//! two FerrisPass processes saving one vault.
//!
//! Nothing holds it across a network request. Every hold is file and keychain
//! calls, so the longest one is milliseconds, and that is what lets a
//! background caller wait a contended lock out instead of reporting it.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::time::{Duration, Instant};

use super::config;

/// What the app waits. Its config writes happen inside update callbacks on
/// the UI thread, so this is the length of a freeze the user would feel; the
/// work it guards is file and keychain calls, which take milliseconds.
pub const INTERACTIVE: Duration = Duration::from_millis(250);

/// What a CLI command waits. It has no interface to freeze, so it can afford
/// to outlast any hold rather than fail a scripted run over one.
pub const BATCH: Duration = Duration::from_secs(30);

const POLL: Duration = Duration::from_millis(25);

/// Why the work did not run.
///
/// The two are worth telling apart because the answer to them differs.
/// Contention passes on its own, so waiting is the remedy; a lock file that
/// cannot be opened at all will say the same thing after every wait, and
/// reporting it as another process both hides the real fault and spends the
/// caller's whole retry budget first.
#[derive(Debug)]
pub enum Unavailable {
    /// Another FerrisPass process held the lock for longer than the wait.
    Contended,
    /// The lock itself could not be reached.
    Broken(io::Error),
}

/// Run `f` while no other FerrisPass process is inside this same call, or
/// return why it did not run.
///
/// Never both: proceeding without the lock is what the lock exists to stop,
/// and it fails at exactly the moment contention proves it was needed. Every
/// caller here is a write, and not writing is recoverable, while writing over
/// somebody else's decision is not.
pub fn held<T>(wait: Duration, f: impl FnOnce() -> T) -> Result<T, Unavailable> {
    with_file(&path().map_err(Unavailable::Broken)?, wait, f)
}

/// How many times a background operation asks again before treating the lock
/// as something to report. Contention is another process finishing its own
/// file and keychain calls, so a few tries over a second is generous.
pub const RETRIES: u32 = 4;

/// Run `f`, and ask again while it says another process was in the way.
///
/// For background work that has nobody to tell: a disconnect's cleanup and a
/// binding restore both run on a task, and "someone else was writing" is not
/// something to surface, it is something to wait out.
pub fn retrying<T, E>(
    mut f: impl FnMut() -> Result<T, E>,
    busy: impl Fn(&E) -> bool,
) -> Result<T, E> {
    for _ in 0..RETRIES {
        match f() {
            Err(error) if busy(&error) => std::thread::sleep(INTERACTIVE),
            outcome => return outcome,
        }
    }
    f()
}

/// The same, on a named file, so a test can hold it from the other side.
fn with_file<T>(
    path: &std::path::Path,
    wait: Duration,
    f: impl FnOnce() -> T,
) -> Result<T, Unavailable> {
    let _guard = acquire(path, wait)?;
    Ok(f())
}

fn acquire(path: &std::path::Path, wait: Duration) -> Result<File, Unavailable> {
    let file = open(path).map_err(Unavailable::Broken)?;
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock() {
            // Released when the handle is dropped, which is the caller's
            // return.
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(POLL);
            }
            Err(TryLockError::WouldBlock) => return Err(Unavailable::Contended),
            Err(TryLockError::Error(error)) => return Err(Unavailable::Broken(error)),
        }
    }
}

fn path() -> io::Result<std::path::PathBuf> {
    let dir = config::app_support_dir()
        .map_err(io::Error::other)?
        .join("sync");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(".lock"))
}

fn open(path: &std::path::Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point: it never runs the work without the lock. Proceeding
    /// anyway is what the lock exists to prevent, and it would give way at
    /// exactly the moment contention proved it was needed.
    #[test]
    fn a_lock_that_cannot_be_taken_does_not_run_the_work() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(".lock");
        let held = File::create(&path).expect("create");
        held.lock().expect("hold it");

        let mut ran = false;
        let result = with_file(&path, Duration::from_millis(30), || ran = true);

        assert!(
            matches!(result, Err(Unavailable::Contended)),
            "it names the other process"
        );
        assert!(!ran, "and it did not run");
    }

    /// A lock that cannot be opened is not a lock somebody else is holding.
    /// Calling it contention sent every caller through its full retry budget
    /// and then blamed a process that was never there, while the real fault,
    /// here a directory in the way of the lock file, went unmentioned.
    #[test]
    fn a_lock_that_cannot_be_opened_is_not_reported_as_contention() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(".lock");
        std::fs::create_dir(&path).expect("occupy the name");

        let mut ran = false;
        let result = with_file(&path, Duration::from_millis(30), || ran = true);

        assert!(
            matches!(result, Err(Unavailable::Broken(_))),
            "it reports the open failure, not contention"
        );
        assert!(!ran, "and it did not run");
    }

    /// Contention is another process finishing its own file and keychain
    /// calls, so asking again is the whole remedy. Background work that
    /// reported it instead told the user their sync had broken.
    #[test]
    fn retrying_asks_again_while_the_answer_is_busy() {
        let mut attempts = 0;
        let result: Result<&str, &str> = retrying(
            || {
                attempts += 1;
                if attempts < 3 {
                    Err("busy")
                } else {
                    Ok("done")
                }
            },
            |error| *error == "busy",
        );
        assert_eq!(result, Ok("done"));
        assert_eq!(attempts, 3);

        // Anything else is an answer, not a queue.
        let mut attempts = 0;
        let result: Result<&str, &str> = retrying(
            || {
                attempts += 1;
                Err("no such file")
            },
            |error| *error == "busy",
        );
        assert_eq!(result, Err("no such file"));
        assert_eq!(attempts, 1);
    }

    #[test]
    fn an_uncontended_lock_runs_the_work_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(".lock");

        let mut runs = 0;
        let result = with_file(&path, Duration::from_millis(30), || runs += 1);

        assert!(result.is_ok());
        assert_eq!(runs, 1);
    }
}

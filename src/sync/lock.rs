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
//! The critical sections are file and keychain calls only. Nothing holds this
//! across a network request: the app would then sit behind a CLI sync for as
//! long as an upload takes.

use std::fs::{File, OpenOptions};
use std::time::{Duration, Instant};

use super::config;

/// Give up rather than hang. Whoever holds this is doing file and keychain
/// calls, so a second is already long; ten means something is wrong, and
/// proceeding is what this code did before the lock existed.
const WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(50);

/// Run `f` while no other FerrisPass process is inside this same call.
///
/// A lock that cannot be taken is not a reason to refuse the work: that would
/// turn a missing directory, a read-only home or a stuck peer into a failure
/// to sync at all. The result is then exactly the behaviour without it, which
/// is a race nobody has reported rather than a certainty.
pub fn held<T>(f: impl FnOnce() -> T) -> T {
    let _guard = acquire();
    f()
}

fn acquire() -> Option<File> {
    let file = open()?;
    let deadline = Instant::now() + WAIT;
    loop {
        match file.try_lock() {
            // Released when the handle is dropped, which is the caller's
            // return.
            Ok(()) => return Some(file),
            Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(POLL);
            }
            _ => return None,
        }
    }
}

fn open() -> Option<File> {
    let dir = config::app_support_dir().ok()?.join("sync");
    std::fs::create_dir_all(&dir).ok()?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(dir.join(".lock")).ok()
}

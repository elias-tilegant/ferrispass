//! Tempdir hygiene for the launch subsystem.
//!
//! Two entry points, all best-effort:
//! - `sweep_stale(max_age)` — deletes any payload from a previous run
//!   that crashed before its TTL timer fired. Anything younger than
//!   `max_age` is left alone in case another instance is mid-launch.
//!   Called from `AppShell::new` (immediately and again after a 120 s
//!   timer, for orphans the immediate pass was too early to age out)
//!   and from `AppState::finish_open_attempt` on every unlock.
//! - `purge_all()` — called from lock/quit hooks. Removes the whole
//!   subdir; `launch_dir()` will recreate it lazily on next launch.

use std::path::Path;
use std::time::{Duration, SystemTime};

use super::tempfile::launch_dir;

/// Delete launch payloads older than `max_age`. Errors are swallowed —
/// a failed sweep doesn't justify aborting startup, and there's
/// nothing the user can do about it from the UI.
pub fn sweep_stale(max_age: Duration) {
    if let Ok(dir) = launch_dir() {
        sweep_stale_in(&dir, max_age);
    }
}

pub(crate) fn sweep_stale_in(dir: &Path, max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let age = now.duration_since(modified).unwrap_or(Duration::ZERO);
        if age > max_age {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Wipe the whole launch tempdir. Called on `lock_vault` and on app
/// quit so we don't sit on cleartext payload files while the vault
/// is closed.
pub fn purge_all() {
    if let Ok(dir) = launch_dir() {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread;

    /// Files older than the threshold get removed; fresh ones stay.
    /// Isolated tempdir per test so parallel runs don't fight over
    /// the production launch_dir.
    #[test]
    fn sweep_removes_old_only() {
        let dir = ::tempfile::TempDir::new().expect("tempdir");
        let old = dir.path().join("launch-old.sapc");
        fs::write(&old, b"old").expect("write old");
        thread::sleep(Duration::from_millis(60));
        let new = dir.path().join("launch-new.sapc");
        fs::write(&new, b"new").expect("write new");

        sweep_stale_in(dir.path(), Duration::from_millis(50));

        assert!(!old.exists(), "old file must be swept");
        assert!(new.exists(), "fresh file must survive");
    }

    /// `read_dir`-walked but non-file entries are ignored — sweep
    /// must never blast a subdir even if one accidentally exists.
    #[test]
    fn sweep_leaves_subdirectories_alone() {
        let dir = ::tempfile::TempDir::new().expect("tempdir");
        let sub = dir.path().join("subdir");
        fs::create_dir(&sub).expect("mkdir");
        thread::sleep(Duration::from_millis(60));

        sweep_stale_in(dir.path(), Duration::from_millis(50));
        assert!(sub.exists(), "subdirs are off-limits to the sweeper");
    }
}

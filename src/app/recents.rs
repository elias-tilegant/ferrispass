//! "Recently opened vaults" persistence — drives the auto-resume on
//! startup and the Recents list on the Welcome screen.
//!
//! Stored as a single JSON file under the platform's app-support directory,
//! beside the per-vault sync configs:
//!
//! On macOS:   `~/Library/Application Support/stc-keepass/recent.json`
//! On Linux:   `$XDG_CONFIG_HOME/stc-keepass/recent.json`
//!
//! Contents are intentionally minimal — paths + last-opened timestamps,
//! nothing else. **No master passwords, no OAuth tokens.** Refresh tokens
//! continue to live in the OS keychain (`sync::tokens`).
//!
//! Atomic-write pattern is the same as `sync::config::save_in`: temp file
//! in the same directory, fsync, rename over the target. That keeps the
//! list safe across crashes / concurrent writes from a second app
//! instance (rare but cheap to defend against).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Cap on how many recent vaults we remember. Eight is roughly the
/// upper bound a user can reasonably eyeball without scrolling, and
/// matches Finder's "Recent Items" default.
pub const MAX_RECENTS: usize = 8;

const FILE_NAME: &str = "recent.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentEntry {
    pub path: PathBuf,
    pub last_opened_at: DateTime<Local>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecentVaults {
    pub entries: Vec<RecentEntry>,
}

#[derive(Debug, Error)]
pub enum RecentsError {
    #[error("could not locate app-support directory: {0}")]
    NoSupportDir(String),

    #[error("io error on {0}: {1}")]
    Io(PathBuf, #[source] io::Error),

    #[error("could not serialise recents: {0}")]
    Serialize(#[source] serde_json::Error),

    #[error("could not parse recents at {0}: {1}")]
    Parse(PathBuf, #[source] serde_json::Error),
}

/// Read the recents file. `Ok(empty)` when the file doesn't exist yet
/// (cold first launch), `Ok(empty)` also when the file is malformed —
/// startup must never block on a corrupt list. Real I/O errors still
/// surface so unrelated problems aren't swallowed.
pub fn load() -> Result<RecentVaults, RecentsError> {
    let dir = match crate::sync::config::app_support_dir() {
        Ok(d) => d,
        Err(e) => return Err(RecentsError::NoSupportDir(e.to_string())),
    };
    load_in(&dir)
}

/// Atomic write to disk. Creates parents on demand. Same crash-safety
/// shape as `sync::config::save`.
pub fn save(recents: &RecentVaults) -> Result<(), RecentsError> {
    let dir = match crate::sync::config::app_support_dir() {
        Ok(d) => d,
        Err(e) => return Err(RecentsError::NoSupportDir(e.to_string())),
    };
    save_in(&dir, recents)
}

/// Convenience for startup: load the list, drop entries whose file no
/// longer exists, and persist the pruned list back to disk if anything
/// changed. Errors are intentionally swallowed — auto-resume is a
/// best-effort feature and shouldn't block the app from starting.
pub fn load_pruned() -> RecentVaults {
    let mut recents = load().unwrap_or_default();
    let before = recents.entries.len();
    recents.entries.retain(|entry| entry.path.exists());
    if recents.entries.len() != before {
        let _ = save(&recents);
    }
    recents
}

/// Move `path` to the front of `entries`, dedup any older copies, and
/// truncate to `max`. Pure — no I/O. Updates `last_opened_at` to `now`.
/// Operates on `Vec<RecentEntry>` directly so AppState can hold a flat
/// field without wrapping in `RecentVaults`.
pub fn push_front_in(entries: &mut Vec<RecentEntry>, path: PathBuf, max: usize) {
    // Drop any prior copy of the same path. PartialEq on PathBuf is byte-
    // exact; we don't canonicalise (symlinks would surprise the user, and
    // `KeePassRepository::open` works just fine on uncanonicalised paths).
    entries.retain(|e| e.path != path);
    entries.insert(
        0,
        RecentEntry {
            path,
            last_opened_at: Local::now(),
        },
    );
    if entries.len() > max {
        entries.truncate(max);
    }
}

// --- *_in variants take the directory explicitly so tests can use a
// tempdir without touching $HOME. Mirrors the same split in sync/config.rs.

pub(crate) fn load_in(dir: &Path) -> Result<RecentVaults, RecentsError> {
    let path = dir.join(FILE_NAME);
    match fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<RecentVaults>(&text) {
            Ok(recents) => Ok(recents),
            // Corrupt file shouldn't block startup. Treat as empty; the
            // next successful open will overwrite with a fresh list.
            Err(_) => Ok(RecentVaults::default()),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(RecentVaults::default()),
        Err(e) => Err(RecentsError::Io(path, e)),
    }
}

pub(crate) fn save_in(dir: &Path, recents: &RecentVaults) -> Result<(), RecentsError> {
    fs::create_dir_all(dir).map_err(|e| RecentsError::Io(dir.to_path_buf(), e))?;
    let target = dir.join(FILE_NAME);
    let tmp = {
        let mut buf = target.as_os_str().to_owned();
        buf.push(".tmp");
        PathBuf::from(buf)
    };

    let text = serde_json::to_string_pretty(recents).map_err(RecentsError::Serialize)?;

    {
        let mut file = fs::File::create(&tmp).map_err(|e| RecentsError::Io(tmp.clone(), e))?;
        use std::io::Write as _;
        file.write_all(text.as_bytes())
            .map_err(|e| RecentsError::Io(tmp.clone(), e))?;
        file.sync_all().map_err(|e| RecentsError::Io(tmp.clone(), e))?;
    }
    fs::rename(&tmp, &target).map_err(|e| RecentsError::Io(target, e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn entry(p: &str) -> RecentEntry {
        RecentEntry {
            path: PathBuf::from(p),
            last_opened_at: Local::now(),
        }
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = TempDir::new().unwrap();
        let recents = RecentVaults {
            entries: vec![entry("/tmp/a.kdbx"), entry("/tmp/b.kdbx")],
        };
        save_in(dir.path(), &recents).unwrap();

        let loaded = load_in(dir.path()).unwrap();
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries[0].path, PathBuf::from("/tmp/a.kdbx"));
        assert_eq!(loaded.entries[1].path, PathBuf::from("/tmp/b.kdbx"));
    }

    #[test]
    fn load_missing_returns_empty() {
        let dir = TempDir::new().unwrap();
        let loaded = load_in(dir.path()).unwrap();
        assert!(loaded.entries.is_empty());
    }

    #[test]
    fn load_corrupt_returns_empty_not_error() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(FILE_NAME), "{ this is not json").unwrap();
        // Must not block startup — corrupt list is treated as empty.
        let loaded = load_in(dir.path()).unwrap();
        assert!(loaded.entries.is_empty());
    }

    #[test]
    fn push_front_dedupes_existing_path() {
        let mut entries = vec![entry("/tmp/a.kdbx"), entry("/tmp/b.kdbx")];
        push_front_in(&mut entries, PathBuf::from("/tmp/b.kdbx"), MAX_RECENTS);
        assert_eq!(entries.len(), 2, "duplicate must not grow the list");
        assert_eq!(entries[0].path, PathBuf::from("/tmp/b.kdbx"));
        assert_eq!(entries[1].path, PathBuf::from("/tmp/a.kdbx"));
    }

    #[test]
    fn push_front_truncates_to_max() {
        let mut entries: Vec<RecentEntry> = (0..MAX_RECENTS)
            .map(|i| entry(&format!("/tmp/v{i}.kdbx")))
            .collect();
        push_front_in(
            &mut entries,
            PathBuf::from("/tmp/new.kdbx"),
            MAX_RECENTS,
        );
        assert_eq!(entries.len(), MAX_RECENTS);
        assert_eq!(entries[0].path, PathBuf::from("/tmp/new.kdbx"));
        // Oldest (index MAX_RECENTS - 1 before the push) got dropped.
        assert!(
            !entries
                .iter()
                .any(|e| e.path == PathBuf::from(format!("/tmp/v{}.kdbx", MAX_RECENTS - 1)))
        );
    }

    #[test]
    fn push_front_updates_timestamp() {
        // Touching an existing path should refresh its timestamp, not
        // preserve the old one.
        let old = RecentEntry {
            path: PathBuf::from("/tmp/a.kdbx"),
            last_opened_at: Local::now() - chrono::Duration::hours(1),
        };
        let mut entries = vec![old.clone()];
        push_front_in(&mut entries, PathBuf::from("/tmp/a.kdbx"), MAX_RECENTS);
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0].last_opened_at > old.last_opened_at,
            "timestamp must advance on re-open"
        );
    }

    #[test]
    fn pruning_drops_nonexistent_paths() {
        // We exercise the pruning logic directly (load_pruned itself
        // touches $HOME, which we can't redirect under forbid(unsafe_code)).
        let dir = TempDir::new().unwrap();
        let real_file = dir.path().join("real.kdbx");
        fs::write(&real_file, b"not actually a kdbx").unwrap();

        let mut recents = RecentVaults {
            entries: vec![
                RecentEntry {
                    path: PathBuf::from("/tmp/definitely-does-not-exist-stc.kdbx"),
                    last_opened_at: Local::now(),
                },
                RecentEntry {
                    path: real_file.clone(),
                    last_opened_at: Local::now(),
                },
            ],
        };
        recents.entries.retain(|e| e.path.exists());
        assert_eq!(recents.entries.len(), 1);
        assert_eq!(recents.entries[0].path, real_file);
    }
}

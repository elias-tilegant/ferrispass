//! App-wide preferences (auto-lock / clipboard-clear timeouts) persisted
//! at `~/Library/Application Support/stc-keepass/settings.json`.
//!
//! Only stores plain numbers — no secrets — so JSON is fine. Same atomic
//! write pattern as `sync/config.rs` and `app/recents.rs` (temp file +
//! fsync + rename).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const FILE_NAME: &str = "settings.json";

/// `None` on a timeout field means "disabled" — i.e. never auto-lock /
/// never auto-clear. We keep the type explicit (rather than a magic 0)
/// so the UI can distinguish "user picked Never" from "the file is
/// missing this field".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSettings {
    pub auto_lock_secs: Option<u64>,
    pub clipboard_clear_secs: Option<u64>,
}

impl Default for AppSettings {
    fn default() -> Self {
        // Mirrors the previous hardcoded constants so users upgrading
        // from a build without a settings file see no behavior change.
        Self {
            auto_lock_secs: Some(240),
            clipboard_clear_secs: Some(10),
        }
    }
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("could not locate app-support directory: {0}")]
    NoSupportDir(String),

    #[error("io error on {0}: {1}")]
    Io(PathBuf, #[source] io::Error),

    #[error("could not serialise settings: {0}")]
    Serialize(#[source] serde_json::Error),
}

/// Read settings from disk. Falls back to `AppSettings::default()` on:
/// missing file (cold first run), parse failure (corrupt file — better
/// to recover than to brick the app on start), or path resolution
/// failure. Real I/O errors still propagate so genuinely broken disks
/// surface.
pub fn load() -> AppSettings {
    let dir = match crate::sync::config::app_support_dir() {
        Ok(d) => d,
        Err(_) => return AppSettings::default(),
    };
    load_in(&dir).unwrap_or_default()
}

pub fn save(settings: &AppSettings) -> Result<(), SettingsError> {
    let dir = match crate::sync::config::app_support_dir() {
        Ok(d) => d,
        Err(e) => return Err(SettingsError::NoSupportDir(e.to_string())),
    };
    save_in(&dir, settings)
}

pub(crate) fn load_in(dir: &Path) -> Result<AppSettings, SettingsError> {
    let path = dir.join(FILE_NAME);
    match fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<AppSettings>(&text) {
            Ok(s) => Ok(s),
            // Corrupt file: don't block startup; treat as defaults.
            Err(_) => Ok(AppSettings::default()),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(AppSettings::default()),
        Err(e) => Err(SettingsError::Io(path, e)),
    }
}

pub(crate) fn save_in(dir: &Path, settings: &AppSettings) -> Result<(), SettingsError> {
    fs::create_dir_all(dir).map_err(|e| SettingsError::Io(dir.to_path_buf(), e))?;
    let target = dir.join(FILE_NAME);
    let tmp = {
        let mut buf = target.as_os_str().to_owned();
        buf.push(".tmp");
        PathBuf::from(buf)
    };

    let text = serde_json::to_string_pretty(settings).map_err(SettingsError::Serialize)?;

    {
        let mut file = fs::File::create(&tmp).map_err(|e| SettingsError::Io(tmp.clone(), e))?;
        use std::io::Write as _;
        file.write_all(text.as_bytes())
            .map_err(|e| SettingsError::Io(tmp.clone(), e))?;
        file.sync_all()
            .map_err(|e| SettingsError::Io(tmp.clone(), e))?;
    }
    fs::rename(&tmp, &target).map_err(|e| SettingsError::Io(target, e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn save_then_load_roundtrips() {
        let dir = TempDir::new().unwrap();
        let s = AppSettings {
            auto_lock_secs: Some(60),
            clipboard_clear_secs: None,
        };
        save_in(dir.path(), &s).unwrap();
        let loaded = load_in(dir.path()).unwrap();
        assert_eq!(loaded, s);
    }

    #[test]
    fn load_missing_returns_defaults() {
        let dir = TempDir::new().unwrap();
        let loaded = load_in(dir.path()).unwrap();
        assert_eq!(loaded, AppSettings::default());
    }

    #[test]
    fn load_corrupt_returns_defaults_not_error() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(FILE_NAME), "{ bogus json").unwrap();
        let loaded = load_in(dir.path()).unwrap();
        // Must recover gracefully — don't brick the app on a stray file.
        assert_eq!(loaded, AppSettings::default());
    }

    #[test]
    fn never_options_serialize_as_null() {
        // Belt-and-braces: the UI's "Never" option must round-trip
        // through JSON as `null`, not be silently coerced to 0.
        let s = AppSettings {
            auto_lock_secs: None,
            clipboard_clear_secs: None,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"auto_lock_secs\":null"));
        assert!(json.contains("\"clipboard_clear_secs\":null"));
    }
}

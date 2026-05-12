//! Persisted "What's New" payload shown once after an update restart.
//!
//! The updater installs the new bundle while the old process is still
//! running, so any in-memory release notes disappear across restart. This
//! small JSON file bridges that one launch without touching vault data.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::info::UpdateInfo;

const FILE_NAME: &str = "pending-whats-new.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingWhatsNew {
    pub info: UpdateInfo,
    #[serde(default)]
    pub auto_shown: bool,
}

#[derive(Debug, Error)]
pub enum WhatsNewError {
    #[error("could not locate app-support directory: {0}")]
    NoSupportDir(String),

    #[error("io error on {0}: {1}")]
    Io(PathBuf, #[source] io::Error),

    #[error("could not serialise update notes: {0}")]
    Serialize(#[source] serde_json::Error),
}

pub fn save_pending(info: &UpdateInfo) -> Result<(), WhatsNewError> {
    let dir = app_support_dir()?;
    save_in(
        &dir,
        &PendingWhatsNew {
            info: info.clone(),
            auto_shown: false,
        },
    )
}

pub fn load_for_version(version: &str) -> Option<PendingWhatsNew> {
    let dir = app_support_dir().ok()?;
    load_for_version_in(&dir, version)
}

pub fn mark_auto_shown(version: &str) -> Result<(), WhatsNewError> {
    let dir = app_support_dir()?;
    mark_auto_shown_in(&dir, version)
}

fn load_for_version_in(dir: &Path, version: &str) -> Option<PendingWhatsNew> {
    load_in(dir)
        .ok()
        .flatten()
        .filter(|pending| pending.info.version == version)
}

fn mark_auto_shown_in(dir: &Path, version: &str) -> Result<(), WhatsNewError> {
    let Some(mut pending) = load_in(dir)? else {
        return Ok(());
    };
    if pending.info.version != version || pending.auto_shown {
        return Ok(());
    }
    pending.auto_shown = true;
    save_in(&dir, &pending)
}

fn app_support_dir() -> Result<PathBuf, WhatsNewError> {
    crate::sync::config::app_support_dir().map_err(|e| WhatsNewError::NoSupportDir(e.to_string()))
}

fn load_in(dir: &Path) -> Result<Option<PendingWhatsNew>, WhatsNewError> {
    let path = dir.join(FILE_NAME);
    match fs::read_to_string(&path) {
        Ok(text) => Ok(serde_json::from_str::<PendingWhatsNew>(&text).ok()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(WhatsNewError::Io(path, e)),
    }
}

fn save_in(dir: &Path, pending: &PendingWhatsNew) -> Result<(), WhatsNewError> {
    fs::create_dir_all(dir).map_err(|e| WhatsNewError::Io(dir.to_path_buf(), e))?;
    let target = dir.join(FILE_NAME);
    let tmp = {
        let mut buf = target.as_os_str().to_owned();
        buf.push(".tmp");
        PathBuf::from(buf)
    };

    let text = serde_json::to_string_pretty(pending).map_err(WhatsNewError::Serialize)?;

    {
        let mut file = fs::File::create(&tmp).map_err(|e| WhatsNewError::Io(tmp.clone(), e))?;
        use std::io::Write as _;
        file.write_all(text.as_bytes())
            .map_err(|e| WhatsNewError::Io(tmp.clone(), e))?;
        file.sync_all()
            .map_err(|e| WhatsNewError::Io(tmp.clone(), e))?;
    }
    fs::rename(&tmp, &target).map_err(|e| WhatsNewError::Io(target, e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn info(version: &str) -> UpdateInfo {
        UpdateInfo {
            version: version.into(),
            notes: "Added restart-aware updates.".into(),
            pub_date: Some("2026-05-12T10:00:00Z".into()),
        }
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = TempDir::new().unwrap();
        let pending = PendingWhatsNew {
            info: info("1.2.3"),
            auto_shown: false,
        };
        save_in(dir.path(), &pending).unwrap();
        assert_eq!(load_in(dir.path()).unwrap(), Some(pending));
    }

    #[test]
    fn load_missing_returns_none() {
        let dir = TempDir::new().unwrap();
        assert_eq!(load_in(dir.path()).unwrap(), None);
    }

    #[test]
    fn load_corrupt_returns_none() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(FILE_NAME), "{not json").unwrap();
        assert_eq!(load_in(dir.path()).unwrap(), None);
    }

    #[test]
    fn load_for_version_filters_other_versions() {
        let dir = TempDir::new().unwrap();
        save_in(
            dir.path(),
            &PendingWhatsNew {
                info: info("1.2.3"),
                auto_shown: false,
            },
        )
        .unwrap();

        assert!(load_for_version_in(dir.path(), "1.2.3").is_some());
        assert_eq!(load_for_version_in(dir.path(), "9.9.9"), None);
    }

    #[test]
    fn mark_auto_shown_updates_matching_version() {
        let dir = TempDir::new().unwrap();
        save_in(
            dir.path(),
            &PendingWhatsNew {
                info: info("1.2.3"),
                auto_shown: false,
            },
        )
        .unwrap();

        mark_auto_shown_in(dir.path(), "1.2.3").unwrap();

        assert!(load_in(dir.path()).unwrap().unwrap().auto_shown);
    }

    #[test]
    fn mark_auto_shown_ignores_other_versions() {
        let dir = TempDir::new().unwrap();
        save_in(
            dir.path(),
            &PendingWhatsNew {
                info: info("1.2.3"),
                auto_shown: false,
            },
        )
        .unwrap();

        mark_auto_shown_in(dir.path(), "9.9.9").unwrap();

        assert!(!load_in(dir.path()).unwrap().unwrap().auto_shown);
    }
}

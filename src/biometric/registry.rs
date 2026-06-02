//! Persistent index of "which vault paths have a biometric
//! enrolment". Lives next to `settings.json` and `recent.json` under
//! the platform's app-support directory. **Contents are deliberately
//! minimal** — only the vault path, a UUID, the keyfile path that
//! applied at enrolment time, and a timestamp. **Never** a password
//! or any vault contents; passwords live in the OS keychain under the
//! UUID.
//!
//! Atomic-write pattern mirrors [`crate::app::recents`] and
//! [`crate::app::settings`]: temp file in the same directory, fsync,
//! rename over the target.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::biometric::EnrollmentId;

const FILE_NAME: &str = "biometric.json";

/// One per-vault enrolment. `id` is the stable key into the OS
/// keychain; `keyfile` is the path the user had selected when they
/// enrolled (we keep it here rather than re-deriving via
/// `KeePassRepository::suggested_keyfile` so a user who had a custom
/// keyfile at enrolment time doesn't silently lose it on next
/// unlock).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BiometricEnrollment {
    pub id: EnrollmentId,
    #[serde(default)]
    pub keyfile: Option<PathBuf>,
    pub enrolled_at: DateTime<Local>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct OnDisk {
    #[serde(default)]
    vaults: HashMap<PathBuf, BiometricEnrollment>,
}

#[derive(Debug, Clone, Default)]
pub struct BiometricRegistry {
    entries: HashMap<PathBuf, BiometricEnrollment>,
}

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("could not locate app-support directory: {0}")]
    NoSupportDir(String),

    #[error("io error on {0}: {1}")]
    Io(PathBuf, #[source] io::Error),

    #[error("could not serialise biometric registry: {0}")]
    Serialize(#[source] serde_json::Error),
}

impl BiometricRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lookup helper for the unlock screen. Returns the matching
    /// enrolment for the *current pending* vault path.
    pub fn get(&self, path: &Path) -> Option<&BiometricEnrollment> {
        self.entries.get(path)
    }

    /// Idempotent: re-enrolling the same path overwrites the prior
    /// entry (caller is expected to `forget` the keychain item under
    /// the *old* id first, otherwise the previous keychain entry is
    /// orphaned).
    pub fn upsert(&mut self, path: PathBuf, enrollment: BiometricEnrollment) {
        self.entries.insert(path, enrollment);
    }

    /// Returns the removed enrolment, if any, so the caller can use
    /// its `id` to also delete the matching keychain item.
    pub fn remove(&mut self, path: &Path) -> Option<BiometricEnrollment> {
        self.entries.remove(path)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&PathBuf, &BiometricEnrollment)> {
        self.entries.iter()
    }
}

/// Read the registry from the platform's app-support dir. Same
/// "treat missing/corrupt as empty" policy as `recents::load` and
/// `settings::load` — startup must never block on a stray file.
pub fn load() -> Result<BiometricRegistry, RegistryError> {
    let dir = crate::sync::config::app_support_dir()
        .map_err(|e| RegistryError::NoSupportDir(e.to_string()))?;
    load_in(&dir)
}

/// Best-effort load used by `AppState::with_resume`. Swallows errors
/// the same way `recents::load_pruned` does: the registry is a
/// quality-of-life feature, not load-bearing for startup.
pub fn load_or_default() -> BiometricRegistry {
    load().unwrap_or_default()
}

pub fn save(registry: &BiometricRegistry) -> Result<(), RegistryError> {
    let dir = crate::sync::config::app_support_dir()
        .map_err(|e| RegistryError::NoSupportDir(e.to_string()))?;
    save_in(&dir, registry)
}

pub(crate) fn load_in(dir: &Path) -> Result<BiometricRegistry, RegistryError> {
    let path = dir.join(FILE_NAME);
    match fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<OnDisk>(&text) {
            Ok(disk) => Ok(BiometricRegistry {
                entries: disk.vaults,
            }),
            // Corrupt file: don't brick the app. Treat as empty;
            // the next enrol overwrites it.
            Err(_) => Ok(BiometricRegistry::default()),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(BiometricRegistry::default()),
        Err(e) => Err(RegistryError::Io(path, e)),
    }
}

pub(crate) fn save_in(dir: &Path, registry: &BiometricRegistry) -> Result<(), RegistryError> {
    fs::create_dir_all(dir).map_err(|e| RegistryError::Io(dir.to_path_buf(), e))?;
    let target = dir.join(FILE_NAME);
    let tmp = {
        let mut buf = target.as_os_str().to_owned();
        buf.push(".tmp");
        PathBuf::from(buf)
    };

    let disk = OnDisk {
        vaults: registry.entries.clone(),
    };
    let text = serde_json::to_string_pretty(&disk).map_err(RegistryError::Serialize)?;

    {
        let mut file = fs::File::create(&tmp).map_err(|e| RegistryError::Io(tmp.clone(), e))?;
        use std::io::Write as _;
        file.write_all(text.as_bytes())
            .map_err(|e| RegistryError::Io(tmp.clone(), e))?;
        file.sync_all()
            .map_err(|e| RegistryError::Io(tmp.clone(), e))?;
    }
    fs::rename(&tmp, &target).map_err(|e| RegistryError::Io(target, e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample(path: &str) -> (PathBuf, BiometricEnrollment) {
        (
            PathBuf::from(path),
            BiometricEnrollment {
                id: EnrollmentId::new_random(),
                keyfile: None,
                enrolled_at: Local::now(),
            },
        )
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = TempDir::new().unwrap();
        let mut reg = BiometricRegistry::new();
        let (path, enrol) = sample("/tmp/a.kdbx");
        reg.upsert(path.clone(), enrol.clone());
        save_in(dir.path(), &reg).unwrap();

        let loaded = load_in(dir.path()).unwrap();
        assert_eq!(loaded.get(&path), Some(&enrol));
    }

    #[test]
    fn load_missing_returns_empty() {
        let dir = TempDir::new().unwrap();
        let loaded = load_in(dir.path()).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn load_corrupt_returns_empty_not_error() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(FILE_NAME), "{ not json").unwrap();
        let loaded = load_in(dir.path()).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn remove_returns_prior_enrollment_for_keychain_cleanup() {
        let mut reg = BiometricRegistry::new();
        let (path, enrol) = sample("/tmp/a.kdbx");
        reg.upsert(path.clone(), enrol.clone());
        let removed = reg.remove(&path).expect("entry must exist");
        assert_eq!(removed.id, enrol.id);
        assert!(reg.remove(&path).is_none(), "double-remove must be no-op");
    }

    /// Security invariant: the on-disk JSON must only ever carry the
    /// allowlisted, non-secret fields. An allowlist (rather than a
    /// "doesn't contain the word password" substring check) is the
    /// robust form: it fails the moment *any* new field appears —
    /// including one named `master`, `credential`, `token`, etc. —
    /// forcing a deliberate review of whether it's safe to persist.
    #[test]
    fn serialised_json_has_only_allowlisted_fields() {
        let dir = TempDir::new().unwrap();
        let mut reg = BiometricRegistry::new();
        let (path, enrol) = sample("/tmp/a.kdbx");
        reg.upsert(path, enrol);
        save_in(dir.path(), &reg).unwrap();
        let text = fs::read_to_string(dir.path().join(FILE_NAME)).unwrap();

        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        // Top level: { "vaults": { "<path>": { ...enrolment... } } }
        let top: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(top, vec!["vaults"], "unexpected top-level keys: {top:?}");

        let vaults = value["vaults"].as_object().unwrap();
        for (_path, enrol) in vaults {
            let mut keys: Vec<&str> = enrol
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            keys.sort_unstable();
            assert_eq!(
                keys,
                vec!["enrolled_at", "id", "keyfile"],
                "biometric.json enrolment carries a non-allowlisted field — \
                 review whether it leaks anything secret before adding it. Got: {keys:?}"
            );
        }
    }

    #[test]
    fn entries_with_keyfile_roundtrip() {
        let dir = TempDir::new().unwrap();
        let mut reg = BiometricRegistry::new();
        let enrol = BiometricEnrollment {
            id: EnrollmentId::new_random(),
            keyfile: Some(PathBuf::from("/tmp/a.key")),
            enrolled_at: Local::now(),
        };
        reg.upsert(PathBuf::from("/tmp/a.kdbx"), enrol.clone());
        save_in(dir.path(), &reg).unwrap();
        let loaded = load_in(dir.path()).unwrap();
        assert_eq!(
            loaded.get(Path::new("/tmp/a.kdbx")).unwrap().keyfile,
            Some(PathBuf::from("/tmp/a.key"))
        );
    }
}

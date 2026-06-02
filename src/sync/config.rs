//! Per-vault sync metadata persisted under the platform's app-support
//! directory. Each synced vault has exactly one config file, keyed by the
//! SHA-256 of the canonical local path so two vaults with the same filename
//! in different folders don't collide and renames are caught.
//!
//! On macOS:   `~/Library/Application Support/ferrispass/sync/<hash>.json`
//! On Linux:   `$XDG_CONFIG_HOME/ferrispass/sync/<hash>.json` (or
//!             `~/.config/ferrispass/sync/<hash>.json` if XDG not set)
//! Windows is unsupported in this MVP — see plan §Risks.
//!
//! What's *not* in this file: the OAuth refresh token. Tokens live in the
//! macOS Keychain (see `tokens.rs`). The config holds the durable identifiers
//! Microsoft Graph needs to address the file plus the last known ETag for
//! optimistic-concurrency uploads.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncConfig {
    pub provider: SyncProvider,
    /// User-facing identity (e.g. `alice@contoso.onmicrosoft.com`). Also used
    /// as the Keychain account key when looking up the refresh token.
    pub account_email: String,
    /// Microsoft Graph site id of the form `{host},{site-guid},{web-guid}`.
    /// Stable across renames of the site's display name.
    pub site_id: String,
    /// Document-library (drive) id within the site.
    pub drive_id: String,
    /// File (drive item) id. Survives the file being renamed/moved within
    /// the library, which makes it our safest cross-session anchor.
    pub item_id: String,
    /// Last ETag we observed on the server. Sent as `If-Match` on the next
    /// upload; a 412 response means another writer landed in between.
    pub last_etag: String,
    /// Canonical absolute path of the local `.kdbx`. The hash of this string
    /// is the config-file name, so it must stay stable across loads.
    pub local_path: PathBuf,
    /// Original SharePoint URL the user pasted. Display-only; never used to
    /// rebuild Graph addresses (those use site_id + drive_id + item_id).
    pub remote_url: String,
    /// Unix seconds of the last *interactive* sign-in (initial Connect or a
    /// user-driven Reconnect). Display-only — drives the "Connected since …"
    /// line in Settings → Sync so the user has a reference point for how
    /// long the current grant has been alive. `#[serde(default)]` so configs
    /// written before this field existed deserialise cleanly as `None`
    /// ("Connected" with no date) instead of failing to load.
    #[serde(default)]
    pub authenticated_at: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SyncProvider {
    SharePoint,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("could not locate app-support directory: {0}")]
    NoSupportDir(String),

    #[error("io error on {0}: {1}")]
    Io(PathBuf, #[source] io::Error),

    #[error("could not serialise config: {0}")]
    Serialize(#[source] serde_json::Error),

    #[error("could not parse config at {0}: {1}")]
    Parse(PathBuf, #[source] serde_json::Error),
}

/// Resolve the directory holding sync-config JSON files, creating it
/// (and parents) on demand. Idempotent.
pub fn ensure_dir() -> Result<PathBuf, ConfigError> {
    let dir = sync_dir()?;
    ensure_dir_in(&dir)?;
    Ok(dir)
}

/// File path for a given vault's config. Doesn't touch the filesystem.
pub fn config_path_for(local_path: &Path) -> Result<PathBuf, ConfigError> {
    Ok(sync_dir()?.join(format!("{}.json", path_hash(local_path))))
}

/// Read the sync config for the given vault path. Returns `Ok(None)` when
/// no config exists (new / unsynced vault) — that's the common case on
/// first launch and not worth error-typing.
pub fn load(local_path: &Path) -> Result<Option<SyncConfig>, ConfigError> {
    load_in(&sync_dir()?, local_path)
}

/// Atomically write a sync config to disk: temp file in the same directory,
/// fsync, rename over the target. Same pattern as `keepass::document::save_to`.
pub fn save(config: &SyncConfig) -> Result<(), ConfigError> {
    let dir = ensure_dir()?;
    save_in(&dir, config)
}

/// Remove the sync config for a vault path (used by Disconnect). No-op when
/// the file already doesn't exist — disconnect should be idempotent so a
/// retry after a partial failure can finish the cleanup.
pub fn delete(local_path: &Path) -> Result<(), ConfigError> {
    delete_in(&sync_dir()?, local_path)
}

// --- *_in variants take the directory explicitly so tests can use a tempdir
// without mutating $HOME (which would require unsafe under the 2024 edition,
// blocked by the crate's `forbid(unsafe_code)` policy).

pub(crate) fn ensure_dir_in(dir: &Path) -> Result<(), ConfigError> {
    fs::create_dir_all(dir).map_err(|e| ConfigError::Io(dir.to_path_buf(), e))
}

pub(crate) fn load_in(dir: &Path, local_path: &Path) -> Result<Option<SyncConfig>, ConfigError> {
    let path = dir.join(format!("{}.json", path_hash(local_path)));
    match fs::read_to_string(&path) {
        Ok(text) => {
            let cfg: SyncConfig =
                serde_json::from_str(&text).map_err(|e| ConfigError::Parse(path.clone(), e))?;
            Ok(Some(cfg))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ConfigError::Io(path, e)),
    }
}

pub(crate) fn save_in(dir: &Path, config: &SyncConfig) -> Result<(), ConfigError> {
    ensure_dir_in(dir)?;
    let target = dir.join(format!("{}.json", path_hash(&config.local_path)));
    let tmp = {
        let mut buf = target.as_os_str().to_owned();
        buf.push(".tmp");
        PathBuf::from(buf)
    };

    let text = serde_json::to_string_pretty(config).map_err(ConfigError::Serialize)?;

    {
        let mut file = fs::File::create(&tmp).map_err(|e| ConfigError::Io(tmp.clone(), e))?;
        use std::io::Write as _;
        file.write_all(text.as_bytes())
            .map_err(|e| ConfigError::Io(tmp.clone(), e))?;
        file.sync_all()
            .map_err(|e| ConfigError::Io(tmp.clone(), e))?;
    }
    fs::rename(&tmp, &target).map_err(|e| ConfigError::Io(target, e))?;
    Ok(())
}

pub(crate) fn delete_in(dir: &Path, local_path: &Path) -> Result<(), ConfigError> {
    let path = dir.join(format!("{}.json", path_hash(local_path)));
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(ConfigError::Io(path, e)),
    }
}

/// Hex-encoded SHA-256 of the path string. Canonicalisation isn't applied
/// here because the path may not exist yet at save time; callers that want
/// canonical paths should pass `fs::canonicalize(local_path)?` themselves.
fn path_hash(local_path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(local_path.as_os_str().as_encoded_bytes());
    let bytes = hasher.finalize();
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

fn sync_dir() -> Result<PathBuf, ConfigError> {
    Ok(app_support_dir()?.join("sync"))
}

pub(crate) fn app_support_dir() -> Result<PathBuf, ConfigError> {
    let home =
        env::var_os("HOME").ok_or_else(|| ConfigError::NoSupportDir("$HOME not set".into()))?;
    let mut p = PathBuf::from(home);
    if cfg!(target_os = "macos") {
        p.push("Library/Application Support");
    } else if cfg!(target_os = "linux") {
        // Honour XDG when set; otherwise fall back to the conventional default.
        if let Some(xdg) = env::var_os("XDG_CONFIG_HOME") {
            p = PathBuf::from(xdg);
        } else {
            p.push(".config");
        }
    } else {
        return Err(ConfigError::NoSupportDir(format!(
            "unsupported platform: {}",
            env::consts::OS
        )));
    }
    p.push("ferrispass");
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fixture(local_path: &str) -> SyncConfig {
        SyncConfig {
            provider: SyncProvider::SharePoint,
            account_email: "alice@contoso.onmicrosoft.com".into(),
            site_id: "contoso.sharepoint.com,abc-guid,def-guid".into(),
            drive_id: "b!drive-id".into(),
            item_id: "01ITEMID".into(),
            last_etag: "\"{guid},1}\"".into(),
            local_path: PathBuf::from(local_path),
            remote_url: "https://contoso.sharepoint.com/sites/MyTeam/Shared%20Documents/p.kdbx"
                .into(),
            authenticated_at: Some(1_700_000_000),
        }
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = TempDir::new().unwrap();
        let cfg = fixture("/tmp/example.kdbx");
        save_in(dir.path(), &cfg).unwrap();
        let loaded = load_in(dir.path(), Path::new("/tmp/example.kdbx")).unwrap();
        assert_eq!(loaded.as_ref(), Some(&cfg));
    }

    /// A config written before `authenticated_at` existed must load with
    /// the field defaulting to `None` (so the "Connected since" line is
    /// simply omitted) rather than failing to parse and dropping the whole
    /// sync binding on upgrade.
    #[test]
    fn pre_feature_config_loads_with_no_authenticated_at() {
        let dir = TempDir::new().unwrap();
        let cfg = fixture("/tmp/legacy.kdbx");
        // Serialise, then strip the new field to mimic an older file.
        let mut json: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        json.as_object_mut().unwrap().remove("authenticated_at");
        let path = dir
            .path()
            .join(format!("{}.json", path_hash(&cfg.local_path)));
        fs::write(&path, serde_json::to_string(&json).unwrap()).unwrap();

        let loaded = load_in(dir.path(), &cfg.local_path).unwrap().unwrap();
        assert_eq!(loaded.authenticated_at, None);
    }

    #[test]
    fn load_missing_returns_none() {
        let dir = TempDir::new().unwrap();
        let result = load_in(dir.path(), Path::new("/tmp/never-saved.kdbx")).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn delete_removes_existing_then_is_noop() {
        let dir = TempDir::new().unwrap();
        let cfg = fixture("/tmp/to-delete.kdbx");
        save_in(dir.path(), &cfg).unwrap();
        delete_in(dir.path(), &cfg.local_path).unwrap();
        assert_eq!(load_in(dir.path(), &cfg.local_path).unwrap(), None);
        // Second delete must succeed (idempotent — disconnect retries are common).
        delete_in(dir.path(), &cfg.local_path).unwrap();
    }

    #[test]
    fn distinct_paths_get_distinct_files() {
        let dir = TempDir::new().unwrap();
        let a = fixture("/tmp/work.kdbx");
        let b = fixture("/tmp/personal.kdbx");
        save_in(dir.path(), &a).unwrap();
        save_in(dir.path(), &b).unwrap();
        // Both still readable after writing the other — would fail if hashes
        // collided or paths weren't part of the key.
        assert_eq!(
            load_in(dir.path(), &a.local_path).unwrap().as_ref(),
            Some(&a)
        );
        assert_eq!(
            load_in(dir.path(), &b.local_path).unwrap().as_ref(),
            Some(&b)
        );
    }

    #[test]
    fn path_hash_is_stable_across_calls() {
        let p = Path::new("/Users/alice/Documents/vault.kdbx");
        assert_eq!(path_hash(p), path_hash(p));
        assert_ne!(path_hash(p), path_hash(Path::new("/elsewhere.kdbx")));
    }
}

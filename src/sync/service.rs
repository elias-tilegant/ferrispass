//! High-level sync orchestration. Strings together `auth` + `graph` +
//! `config` + `tokens` into the operations the rest of the app actually
//! cares about: connect, upload-after-save, refresh-check, disconnect.
//!
//! Functions here are blocking and `Send`. The caller (`AppState`) drives
//! them from `cx.background_spawn(...)` and bridges the result back to the
//! UI via `Entity::update` — same pattern `save_async` already uses.
//!
//! No GPUI types in this module by design. Keeps the orchestration
//! testable in isolation and makes the dependency direction one-way:
//! `app -> sync` only, never back.

use std::{
    fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use thiserror::Error;

use crate::sync::auth::{self, AccessToken, AuthError};
use crate::sync::config::{self, ConfigError, SyncConfig, SyncProvider};
use crate::sync::graph::{self, DriveItem, DriveItemHit, GraphError, UploadOutcome};
use crate::sync::tokens::{self, TokenError};

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Graph(#[from] GraphError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Tokens(#[from] TokenError),
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Reconnect signed in as a different Microsoft account than the one
    /// this vault is bound to. We refuse to rebind because the stored
    /// drive/item ids belong to `expected`'s tenant — a `got` token can't
    /// address them. Reversible: the user can sign in with the original
    /// account, or Disconnect and Connect afresh.
    #[error(
        "signed in as {got}, but this vault is connected to {expected}. \
         Sign in with the original account, or Disconnect and reconnect to pick a different file."
    )]
    AccountMismatch { expected: String, got: String },
    #[error("remote vault response did not identify the downloaded revision")]
    MissingRemoteEtag,
    #[error("remote vault changed repeatedly while it was being downloaded")]
    UnstableRemoteDownload,
}

/// Prepared Connect result: enough to publish the encrypted local file and
/// sync credentials, then hand off to the unlock flow.
pub struct ConnectResult {
    pub config: SyncConfig,
    pub access_token: AccessToken,
    pub remote_bytes: Vec<u8>,
}

/// Result of `upload_after_save`: either we synced cleanly, or the server
/// already had a newer version and we've fetched it for conflict handling.
pub enum UploadAfterSave {
    Synced {
        new_etag: String,
        item: DriveItem,
    },
    Conflict {
        remote_bytes: Vec<u8>,
        remote_etag: String,
    },
}

/// Result of the on-startup `refresh_check`: lets the AppState decide
/// whether to nudge the user with "remote has new changes".
pub enum RefreshCheck {
    Same,
    RemoteAhead {
        remote_etag: String,
        item: DriveItem,
    },
}

/// Step 1 of connect: request a device code. Just wraps `auth::` so the
/// caller doesn't need to know about both modules.
pub fn request_device_code() -> Result<auth::DeviceCodeChallenge, ServiceError> {
    Ok(auth::request_device_code()?)
}

/// Step 2 of connect, after sign-in: enumerate every `.kdbx` file the
/// user has access to (across all SharePoint sites + personal OneDrive).
/// Returned list is sorted alphabetically by name for deterministic UI.
pub fn list_kdbx_files(token: &AccessToken) -> Result<Vec<DriveItemHit>, ServiceError> {
    let mut hits = graph::search_kdbx_files(token)?;
    hits.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(hits)
}

/// Prepare step 3 of Connect without persistent side effects. The caller
/// publishes the result only after verifying that the operation is current.
///
/// Uses the etag from the download response (not from the search hit) so
/// our `last_etag` is exactly the version that produced these bytes —
/// race-free with the next upload.
pub fn prepare_connect_picked(
    hit: &DriveItemHit,
    token: AccessToken,
    local_path: &Path,
) -> Result<ConnectResult, ServiceError> {
    let user = graph::me(&token)?;
    let (remote_bytes, etag_from_download) =
        download_versioned(&hit.drive_id, &hit.item_id, &token)?;

    let cfg = SyncConfig {
        provider: SyncProvider::SharePoint,
        account_email: user.email,
        site_id: hit.site_id.clone(),
        drive_id: hit.drive_id.clone(),
        item_id: hit.item_id.clone(),
        last_etag: etag_from_download,
        local_path: local_path.to_path_buf(),
        remote_url: hit.web_url.clone(),
        authenticated_at: now_unix(),
    };
    Ok(ConnectResult {
        config: cfg,
        access_token: token,
        remote_bytes,
    })
}

/// Publish a prepared Connect result transactionally. The caller only starts
/// this while the operation is current; once publication begins it either
/// rolls back the local file on error or completes as a discoverable vault.
pub fn persist_connect_picked(result: &ConnectResult) -> Result<(), ServiceError> {
    if config::load(&result.config.local_path)?.is_some() {
        return Err(io_error(
            &result.config.local_path,
            io::Error::new(
                io::ErrorKind::AlreadyExists,
                "sync configuration already exists for this local vault",
            ),
        ));
    }
    let mut staged = StagedVault::new(&result.config.local_path, &result.remote_bytes)?;
    staged.publish()?;
    tokens::store(
        &result.config.account_email,
        &result.access_token.refresh_token,
    )?;
    config::save(&result.config)?;
    staged.commit();
    Ok(())
}

/// Prepare re-authentication for a vault whose refresh token expired. Takes
/// the existing `SyncConfig` and a fresh interactive token, then:
///   1. verifies the re-authed account matches `config.account_email`
///      (case-insensitive) — refuses with `AccountMismatch` otherwise, so
///      we never store a token that can't reach the bound drive item;
///   2. re-stamps `authenticated_at` in memory.
///
/// Persistence is deliberately deferred to `persist_reconnect_rebind`, which
/// the caller runs only while its cancellation gate still owns the operation.
pub fn prepare_reconnect_rebind(
    mut config: SyncConfig,
    token: &AccessToken,
) -> Result<SyncConfig, ServiceError> {
    let user = graph::me(token)?;
    if !user.email.eq_ignore_ascii_case(&config.account_email) {
        return Err(ServiceError::AccountMismatch {
            expected: config.account_email,
            got: user.email,
        });
    }
    config.authenticated_at = now_unix();
    Ok(config)
}

/// Publish a prepared reconnect while the caller holds its operation gate.
pub fn persist_reconnect_rebind(
    config: &SyncConfig,
    token: &AccessToken,
) -> Result<(), ServiceError> {
    tokens::store(&config.account_email, &token.refresh_token)?;
    config::save(config)?;
    Ok(())
}

struct StagedVault {
    target: PathBuf,
    temp: PathBuf,
    temp_identity: Option<FileIdentity>,
    published_identity: Option<FileIdentity>,
    keep_target: bool,
}

impl StagedVault {
    fn new(target: &Path, bytes: &[u8]) -> Result<Self, ServiceError> {
        if fs::symlink_metadata(target).is_ok() {
            return Err(io_error(
                target,
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "refusing to replace an existing local vault",
                ),
            ));
        }

        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        let file_name = target.file_name().ok_or_else(|| {
            io_error(
                target,
                io::Error::new(io::ErrorKind::InvalidInput, "vault path has no file name"),
            )
        })?;
        let seq = CONNECT_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temp = parent.join(format!(
            ".{}.ferrispass-connect-{}-{seq}.tmp",
            file_name.to_string_lossy(),
            std::process::id()
        ));

        let mut file = private_create_new(&temp)?;
        file.write_all(bytes)
            .map_err(|error| io_error(&temp, error))?;
        file.sync_all().map_err(|error| io_error(&temp, error))?;
        let temp_identity =
            FileIdentity::from_metadata(file.metadata().map_err(|error| io_error(&temp, error))?);

        Ok(Self {
            target: target.to_path_buf(),
            temp,
            temp_identity,
            published_identity: None,
            keep_target: false,
        })
    }

    fn publish(&mut self) -> Result<(), ServiceError> {
        self.published_identity = match fs::hard_link(&self.temp, &self.target) {
            Ok(()) => self.temp_identity,
            Err(error) if error.kind() == io::ErrorKind::Unsupported => self.publish_by_copy()?,
            Err(error) => return Err(io_error(&self.target, error)),
        };
        Ok(())
    }

    fn publish_by_copy(&self) -> Result<Option<FileIdentity>, ServiceError> {
        let mut created_identity = None;
        let result = (|| {
            let mut source =
                fs::File::open(&self.temp).map_err(|error| io_error(&self.temp, error))?;
            let mut target = private_create_new(&self.target)?;
            created_identity = FileIdentity::from_metadata(
                target
                    .metadata()
                    .map_err(|error| io_error(&self.target, error))?,
            );
            io::copy(&mut source, &mut target).map_err(|error| io_error(&self.target, error))?;
            target
                .sync_all()
                .map_err(|error| io_error(&self.target, error))
        })();
        if result.is_err()
            && created_identity.is_some_and(|identity| identity.matches(&self.target))
        {
            let _ = fs::remove_file(&self.target);
        }
        result.map(|()| created_identity)
    }

    fn commit(mut self) {
        self.keep_target = true;
        let _ = fs::remove_file(&self.temp);
        #[cfg(unix)]
        if let Some(parent) = self.target.parent() {
            let _ = fs::File::open(parent).and_then(|directory| directory.sync_all());
        }
    }
}

impl Drop for StagedVault {
    fn drop(&mut self) {
        if !self.keep_target
            && self
                .published_identity
                .is_some_and(|identity| identity.matches(&self.target))
        {
            let _ = fs::remove_file(&self.target);
        }
        let _ = fs::remove_file(&self.temp);
    }
}

#[derive(Clone, Copy)]
struct FileIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

impl FileIdentity {
    fn read(path: &Path) -> Option<Self> {
        fs::symlink_metadata(path)
            .ok()
            .and_then(Self::from_metadata)
    }

    fn from_metadata(metadata: fs::Metadata) -> Option<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            Some(Self {
                device: metadata.dev(),
                inode: metadata.ino(),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            None
        }
    }

    fn matches(self, path: &Path) -> bool {
        Self::read(path).is_some_and(|current| {
            #[cfg(unix)]
            {
                current.device == self.device && current.inode == self.inode
            }
            #[cfg(not(unix))]
            {
                let _ = current;
                false
            }
        })
    }
}

fn private_create_new(path: &Path) -> Result<fs::File, ServiceError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path).map_err(|error| io_error(path, error))
}

#[cfg(test)]
fn persist_downloaded_vault(target: &Path, bytes: &[u8]) -> Result<(), ServiceError> {
    let mut staged = StagedVault::new(target, bytes)?;
    staged.publish()?;
    staged.commit();
    Ok(())
}

fn io_error(path: &Path, source: io::Error) -> ServiceError {
    ServiceError::Io {
        path: path.to_path_buf(),
        source,
    }
}

static CONNECT_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Push local bytes to SharePoint with optimistic-concurrency guard.
/// On 412, fetches the remote bytes so the caller can build a conflict
/// report without an extra round-trip from the UI thread.
pub fn upload_after_save(
    config: &SyncConfig,
    token: &AccessToken,
    local_bytes: &[u8],
) -> Result<UploadAfterSave, ServiceError> {
    if config.last_etag.trim().is_empty() {
        // Legacy configs (pre-etag-hardening) can carry an empty revision.
        // Failing hard here would wedge every future push with no self-heal —
        // the Failed-recovery tick only retries the push, never pulls. Route
        // through the conflict path instead: the caller's merge machinery
        // re-downloads the remote and persists a fresh, non-empty etag.
        let (remote_bytes, remote_etag) =
            download_versioned(&config.drive_id, &config.item_id, token)?;
        return Ok(UploadAfterSave::Conflict {
            remote_bytes,
            remote_etag,
        });
    }
    let outcome = graph::upload_content(
        &config.drive_id,
        &config.item_id,
        local_bytes,
        Some(&config.last_etag),
        token,
    )?;
    match outcome {
        UploadOutcome::Ok { new_etag, item } => Ok(UploadAfterSave::Synced { new_etag, item }),
        UploadOutcome::Conflict => {
            let (remote_bytes, etag) =
                download_versioned(&config.drive_id, &config.item_id, token)?;
            Ok(UploadAfterSave::Conflict {
                remote_bytes,
                remote_etag: etag,
            })
        }
    }
}

/// Force-push local bytes ignoring the etag — used by Conflict-resolve
/// "Keep local" path, and by manual override flows. Returns the new etag
/// so the caller can update SyncConfig.
pub fn force_upload(
    config: &SyncConfig,
    token: &AccessToken,
    local_bytes: &[u8],
) -> Result<DriveItem, ServiceError> {
    match graph::upload_content(
        &config.drive_id,
        &config.item_id,
        local_bytes,
        None, // no If-Match → always wins
        token,
    )? {
        UploadOutcome::Ok { item, .. } => Ok(item),
        UploadOutcome::Conflict => {
            // Without If-Match, Graph shouldn't ever return 412. If it does,
            // surface as a generic graph error.
            Err(ServiceError::Graph(GraphError::Status {
                status: 412,
                body: "force upload returned 412 unexpectedly".into(),
            }))
        }
    }
}

/// On app launch: compare the cached etag to what's currently on the server.
/// Cheap (one metadata fetch, no body download). Used to nudge the user
/// when another device wrote since they last synced.
pub fn refresh_check(
    config: &SyncConfig,
    token: &AccessToken,
) -> Result<RefreshCheck, ServiceError> {
    let item = graph::get_item_metadata(&config.drive_id, &config.item_id, token)?;
    if item.etag == config.last_etag {
        Ok(RefreshCheck::Same)
    } else {
        Ok(RefreshCheck::RemoteAhead {
            remote_etag: item.etag.clone(),
            item,
        })
    }
}

/// Download the current remote bytes plus the ETag that produced them.
/// Used by the auto-sync *pull* path after `refresh_check` reports the
/// server moved ahead — we fetch the body and hand it to the same merge
/// machinery the 412-conflict path uses. Falls back to a metadata
/// round-trip for the ETag on the rare occasion the content response
/// omits the header (mirrors `upload_after_save`'s conflict branch).
pub fn download_remote(
    config: &SyncConfig,
    token: &AccessToken,
) -> Result<(Vec<u8>, String), ServiceError> {
    download_versioned(&config.drive_id, &config.item_id, token)
}

/// Download bytes together with the exact revision that produced them.
/// Graph normally returns ETag on the content response. If a proxy strips it,
/// a metadata value fetched only *after* the body is unsafe: the item may have
/// changed between those requests. In that rare case we bracket a fresh
/// download with equal, non-empty metadata ETags and discard unstable bodies.
fn download_versioned(
    drive_id: &str,
    item_id: &str,
    token: &AccessToken,
) -> Result<(Vec<u8>, String), ServiceError> {
    let (bytes, etag) = graph::download_content(drive_id, item_id, token)?;
    if !etag.trim().is_empty() {
        return Ok((bytes, etag));
    }

    for _ in 0..2 {
        let before = graph::get_item_metadata(drive_id, item_id, token)?.etag;
        if before.trim().is_empty() {
            return Err(ServiceError::MissingRemoteEtag);
        }
        let (bytes, header_etag) = graph::download_content(drive_id, item_id, token)?;
        if !header_etag.trim().is_empty() {
            return Ok((bytes, header_etag));
        }
        let after = graph::get_item_metadata(drive_id, item_id, token)?.etag;
        if stable_fallback_etag(&before, &after) {
            return Ok((bytes, before));
        }
    }

    Err(ServiceError::UnstableRemoteDownload)
}

fn stable_fallback_etag(before: &str, after: &str) -> bool {
    !before.trim().is_empty() && before == after
}

/// Refresh the access token using the keychain-stored refresh token.
/// On `InvalidGrant` the refresh token is gone forever — caller should
/// transition the UI to "reconnect required".
pub fn refresh_access_token(account_email: &str) -> Result<AccessToken, ServiceError> {
    let refresh = tokens::load(account_email)?.ok_or_else(|| {
        ServiceError::Auth(AuthError::InvalidGrant(Some(
            "no stored refresh token for this account".into(),
        )))
    })?;
    let token = auth::refresh(&refresh)?;
    // Microsoft sometimes rotates the refresh token; persist whatever came back.
    if token.refresh_token != refresh {
        tokens::store(account_email, &token.refresh_token)?;
    }
    Ok(token)
}

/// Make sure `token` is fresh enough to use; refresh in-place if not.
/// Slack of 60 s rides out clock skew + a typical Graph round-trip.
pub fn ensure_fresh(token: AccessToken, account_email: &str) -> Result<AccessToken, ServiceError> {
    if token.is_near_expiry(std::time::Duration::from_secs(60)) {
        refresh_access_token(account_email)
    } else {
        Ok(token)
    }
}

/// Tear down one vault's sync binding. The vault-local config is removed
/// first, so a later Keychain failure cannot silently reconnect it on restart.
/// The account-level refresh token stays while another vault still uses it.
pub fn disconnect(config: &SyncConfig) -> Result<(), ServiceError> {
    config::delete(&config.local_path)?;
    if !config::has_account_binding(&config.account_email)? {
        tokens::delete(&config.account_email)?;
    }
    Ok(())
}

/// Wall-clock "now" as Unix seconds, or `None` if the system clock is set
/// before the epoch (effectively never). Stamped onto `SyncConfig` at
/// interactive sign-in so the UI can show how long the grant has lived.
fn now_unix() -> Option<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::{persist_downloaded_vault, stable_fallback_etag};

    #[test]
    fn fallback_etag_requires_the_same_non_empty_revision() {
        assert!(stable_fallback_etag("\"item,7\"", "\"item,7\""));
        assert!(!stable_fallback_etag("", ""));
        assert!(!stable_fallback_etag("   ", "   "));
        assert!(!stable_fallback_etag("\"item,7\"", "\"item,8\""));
    }

    #[test]
    fn downloaded_vault_is_published_without_clobbering() {
        let dir = tempfile::tempdir().expect("temp dir");
        let target = dir.path().join("vault.kdbx");

        persist_downloaded_vault(&target, b"first").expect("publish vault");
        assert_eq!(std::fs::read(&target).expect("read vault"), b"first");

        let error = persist_downloaded_vault(&target, b"second").expect_err("refuse clobber");
        assert!(error.to_string().contains("existing local vault"));
        assert_eq!(std::fs::read(&target).expect("read vault"), b"first");
    }

    #[cfg(unix)]
    #[test]
    fn downloaded_vault_uses_private_permissions() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("temp dir");
        let target = dir.path().join("private.kdbx");
        persist_downloaded_vault(&target, b"encrypted").expect("publish vault");

        let mode = std::fs::metadata(target)
            .expect("vault metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn uncommitted_download_publication_rolls_back() {
        let dir = tempfile::tempdir().expect("temp dir");
        let target = dir.path().join("rolled-back.kdbx");
        {
            let mut staged = super::StagedVault::new(&target, b"encrypted").expect("stage");
            staged.publish().expect("publish");
            assert!(target.exists());
        }
        assert!(!target.exists());
    }
}

/// Helper for the AppState side: read local bytes for upload, surfacing a
/// typed io error so we don't have to thread `std::io::Error` through every
/// caller. Keeps the upload entrypoint single-arg.
pub fn read_local(path: &Path) -> Result<Vec<u8>, ServiceError> {
    std::fs::read(path).map_err(|source| ServiceError::Io {
        path: path.to_path_buf(),
        source,
    })
}

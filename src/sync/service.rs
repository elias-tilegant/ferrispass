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

use std::path::{Path, PathBuf};

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
}

/// Result of `complete_connect`: what the caller needs to (a) save the
/// downloaded bytes locally and (b) hand off to the unlock flow.
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
    RemoteAhead { remote_etag: String, item: DriveItem },
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

/// Step 3 of connect, after the user picks a file from the search results:
/// download the bytes, persist SyncConfig + keychain refresh token, return
/// everything the caller needs to write the file locally + open the unlock
/// flow.
///
/// Uses the etag from the download response (not from the search hit) so
/// our `last_etag` is exactly the version that produced these bytes —
/// race-free with the next upload.
pub fn complete_connect_picked(
    hit: &DriveItemHit,
    token: AccessToken,
    local_path: &Path,
) -> Result<ConnectResult, ServiceError> {
    let user = graph::me(&token)?;
    let (remote_bytes, etag_from_download) =
        graph::download_content(&hit.drive_id, &hit.item_id, &token)?;

    // Refresh token to keychain *before* persisting the config — if the
    // keychain write fails we never leave a sync config pointing at a
    // refresh token we can't actually load.
    tokens::store(&user.email, &token.refresh_token)?;

    let cfg = SyncConfig {
        provider: SyncProvider::SharePoint,
        account_email: user.email,
        site_id: hit.site_id.clone(),
        drive_id: hit.drive_id.clone(),
        item_id: hit.item_id.clone(),
        last_etag: etag_from_download,
        local_path: local_path.to_path_buf(),
        remote_url: hit.web_url.clone(),
    };
    config::save(&cfg)?;

    Ok(ConnectResult {
        config: cfg,
        access_token: token,
        remote_bytes,
    })
}

/// Push local bytes to SharePoint with optimistic-concurrency guard.
/// On 412, fetches the remote bytes so the caller can build a conflict
/// report without an extra round-trip from the UI thread.
pub fn upload_after_save(
    config: &SyncConfig,
    token: &AccessToken,
    local_bytes: &[u8],
) -> Result<UploadAfterSave, ServiceError> {
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
            let (remote_bytes, remote_etag) =
                graph::download_content(&config.drive_id, &config.item_id, token)?;
            // Prefer the etag header from download (always present and matches
            // the bytes we just got); fall back to a metadata round-trip if
            // SharePoint didn't return one (unusual).
            let etag = if remote_etag.is_empty() {
                graph::get_item_metadata(&config.drive_id, &config.item_id, token)?.etag
            } else {
                remote_etag
            };
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
pub fn refresh_check(config: &SyncConfig, token: &AccessToken) -> Result<RefreshCheck, ServiceError> {
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

/// Refresh the access token using the keychain-stored refresh token.
/// On `InvalidGrant` the refresh token is gone forever — caller should
/// transition the UI to "reconnect required".
pub fn refresh_access_token(account_email: &str) -> Result<AccessToken, ServiceError> {
    let refresh = tokens::load(account_email)?
        .ok_or_else(|| ServiceError::Auth(AuthError::InvalidGrant))?;
    let token = auth::refresh(&refresh)?;
    // Microsoft sometimes rotates the refresh token; persist whatever came back.
    if token.refresh_token != refresh {
        tokens::store(account_email, &token.refresh_token)?;
    }
    Ok(token)
}

/// Make sure `token` is fresh enough to use; refresh in-place if not.
/// Slack of 60 s rides out clock skew + a typical Graph round-trip.
pub fn ensure_fresh(
    token: AccessToken,
    account_email: &str,
) -> Result<AccessToken, ServiceError> {
    if token.is_near_expiry(std::time::Duration::from_secs(60)) {
        refresh_access_token(account_email)
    } else {
        Ok(token)
    }
}

/// Tear down the sync binding for a vault: remove the local config file
/// and forget the refresh token. Idempotent — safe to retry after a partial
/// failure (e.g., we cleared the keychain but the disk write failed).
pub fn disconnect(config: &SyncConfig) -> Result<(), ServiceError> {
    tokens::delete(&config.account_email)?;
    config::delete(&config.local_path)?;
    Ok(())
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

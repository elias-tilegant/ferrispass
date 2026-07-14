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
        download_versioned(&hit.drive_id, &hit.item_id, &token)?;

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
        authenticated_at: now_unix(),
    };
    config::save(&cfg)?;

    Ok(ConnectResult {
        config: cfg,
        access_token: token,
        remote_bytes,
    })
}

/// Re-authenticate an existing vault whose refresh token expired. Takes
/// the *existing* on-disk `SyncConfig` (loaded by the caller) and a fresh
/// interactive `token`, and:
///   1. verifies the re-authed account matches `config.account_email`
///      (case-insensitive) — refuses with `AccountMismatch` otherwise, so
///      we never store a token that can't reach the bound drive item;
///   2. persists the (possibly rotated) refresh token to the keychain;
///   3. re-stamps `authenticated_at` to now and saves the config back.
///
/// Everything else in the config — drive_id / item_id / site_id /
/// last_etag / local_path / remote_url — is preserved verbatim, so no new
/// local file or duplicate sync binding is created. Returns the updated
/// config for the caller to wrap in a fresh `SyncBinding`.
pub fn reconnect_rebind(
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
    // Keychain write before config save (mirrors `complete_connect_picked`):
    // if the keychain write fails we never leave a config claiming a grant
    // we can't reload.
    tokens::store(&config.account_email, &token.refresh_token)?;
    config.authenticated_at = now_unix();
    config::save(&config)?;
    Ok(config)
}

/// Push local bytes to SharePoint with optimistic-concurrency guard.
/// On 412, fetches the remote bytes so the caller can build a conflict
/// report without an extra round-trip from the UI thread.
pub fn upload_after_save(
    config: &SyncConfig,
    token: &AccessToken,
    local_bytes: &[u8],
) -> Result<UploadAfterSave, ServiceError> {
    if config.last_etag.trim().is_empty() {
        return Err(ServiceError::MissingRemoteEtag);
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

/// Tear down the sync binding for a vault: remove the local config file
/// and forget the refresh token. Idempotent — safe to retry after a partial
/// failure (e.g., we cleared the keychain but the disk write failed).
pub fn disconnect(config: &SyncConfig) -> Result<(), ServiceError> {
    tokens::delete(&config.account_email)?;
    config::delete(&config.local_path)?;
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
    use super::stable_fallback_etag;

    #[test]
    fn fallback_etag_requires_the_same_non_empty_revision() {
        assert!(stable_fallback_etag("\"item,7\"", "\"item,7\""));
        assert!(!stable_fallback_etag("", ""));
        assert!(!stable_fallback_etag("   ", "   "));
        assert!(!stable_fallback_etag("\"item,7\"", "\"item,8\""));
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

//! Thin wrapper around `cargo-packager-updater` that translates the library's
//! API surface into the friendlier `Result<_, UpdateError>` shape the rest of
//! FerrisPass works with, plus an early-out when the placeholder public key
//! is still in place (which would otherwise produce a confusing
//! signature-verification error from a perfectly valid manifest).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use cargo_packager_updater::{Config, check_update};
use semver::Version;

use super::info::{UpdateError, UpdateInfo};
use super::{MINISIGN_PUBLIC_KEY, UPDATE_ENDPOINT};

/// Ask the GitHub-hosted manifest whether a newer release exists. Returns
/// `Ok(None)` when we're already on the latest version (the common case),
/// `Ok(Some(info))` when a newer build is published. Blocking — call from
/// `cx.background_spawn`.
pub fn check() -> Result<Option<UpdateInfo>, UpdateError> {
    let config = build_config()?;
    let current = current_version()?;

    let update = check_update(current, config).map_err(map_err)?;

    Ok(update.map(|u| UpdateInfo {
        version: u.version,
        notes: u.body.unwrap_or_default(),
        pub_date: u.date.map(|d| format!("{} {}, {}", d.month(), d.day(), d.year())),
    }))
}

/// Fetch the latest bundle, verify its minisign signature against the
/// embedded public key, atomic-replace the running app, return. The caller
/// is responsible for prompting the user to restart afterwards (or quitting
/// the app, which causes the OS to launch the new binary on next open).
///
/// Blocking — call from `cx.background_spawn`. Re-checks the manifest
/// internally so the install operates on whatever's current on the server,
/// not on stale info from a previous `check()` call.
///
/// `on_progress(downloaded, total)` fires periodically during download.
/// `total` is `None` when the server didn't report `Content-Length`.
pub fn install<F>(on_progress: F) -> Result<UpdateInfo, UpdateError>
where
    F: Fn(usize, Option<u64>) + Send + 'static,
{
    let config = build_config()?;
    let current = current_version()?;

    let update = check_update(current, config)
        .map_err(map_err)?
        .ok_or_else(|| UpdateError::Network("no update available at install time".into()))?;

    let info = UpdateInfo {
        version: update.version.clone(),
        notes: update.body.clone().unwrap_or_default(),
        pub_date: update
            .date
            .map(|d| format!("{} {}, {}", d.month(), d.day(), d.year())),
    };

    update
        .download_and_install_extended(on_progress, || {})
        .map_err(map_err)?;

    Ok(info)
}

// ---------- internals ----------

fn build_config() -> Result<Config, UpdateError> {
    let raw = MINISIGN_PUBLIC_KEY.trim();
    if raw.contains("PLACEHOLDER") || raw.contains("AAAAAAAAAAAA") {
        return Err(UpdateError::PlaceholderKey);
    }

    // `cargo-packager-updater` (Tauri-updater convention) expects `pubkey` as
    // base64 of the entire minisign-pub.txt contents — internally it base64-
    // decodes back to the original two-line file before calling
    // `PublicKey::decode`. Embedding the raw file here would have it try to
    // base64-decode "untrusted comment: …" and fail at the first space.
    let pubkey = BASE64.encode(raw.as_bytes());

    let endpoint = UPDATE_ENDPOINT
        .parse()
        .map_err(|e: url::ParseError| UpdateError::Parse(format!("endpoint URL: {e}")))?;

    Ok(Config {
        endpoints: vec![endpoint],
        pubkey,
        ..Default::default()
    })
}

fn current_version() -> Result<Version, UpdateError> {
    Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|e| UpdateError::Parse(format!("CARGO_PKG_VERSION: {e}")))
}

/// Crude classification of `cargo-packager-updater` errors into our enum.
/// The library doesn't expose a stable variant taxonomy we can match
/// exhaustively, so we string-match on the Display output. Acceptable
/// for an MVP — refine once we observe real failure modes in the wild.
fn map_err(e: cargo_packager_updater::Error) -> UpdateError {
    let msg = e.to_string();
    let lower = msg.to_lowercase();

    if lower.contains("base64") || lower.contains("invalid symbol") || lower.contains("decode") {
        UpdateError::Parse(msg)
    } else if lower.contains("signature") || lower.contains("pubkey") || lower.contains("verify") {
        UpdateError::SignatureInvalid
    } else if lower.contains("install")
        || lower.contains("relocate")
        || lower.contains("extract")
        || lower.contains("permission")
    {
        UpdateError::Install(msg)
    } else if lower.contains("parse") || lower.contains("json") || lower.contains("deserialize") {
        UpdateError::Parse(msg)
    } else {
        UpdateError::Network(msg)
    }
}

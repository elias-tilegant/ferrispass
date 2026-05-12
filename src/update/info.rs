//! Friendly metadata for an available update — what the UI needs to display
//! the "FerrisPass 0.2.1 available" banner and the release-notes modal.
//!
//! Distinct from `cargo_packager_updater::Update` because that type owns
//! HTTP state and isn't `Clone`. We extract the parts the UI cares about
//! into this `Clone + Send + Sync` struct so it can live inside `AppState`
//! enum variants and survive the round-trip through GPUI subscriptions.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UpdateInfo {
    /// Semver string of the upstream release (e.g. `"0.2.1"`).
    pub version: String,
    /// Free-form release notes from the manifest's `notes` field. Empty
    /// string when the manifest didn't include any.
    pub notes: String,
    /// Optional publish timestamp from the manifest's `pub_date` field
    /// (RFC 3339). Display-only; we don't gate updates on it.
    pub pub_date: Option<String>,
}

#[derive(Debug, Error)]
pub enum UpdateError {
    /// Network failure during manifest fetch or update download.
    #[error("network error: {0}")]
    Network(String),

    /// Manifest reachable but malformed, or signature couldn't be parsed.
    #[error("could not parse update response: {0}")]
    Parse(String),

    /// Public key embedded in the binary doesn't match the signature on
    /// the downloaded bundle. Hard fail — never apply unverified updates.
    #[error("update signature did not verify against the embedded public key")]
    SignatureInvalid,

    /// Disk write or atomic-replace failed during install.
    #[error("could not install update: {0}")]
    Install(String),

    /// Embedded public key is the zeroed placeholder. Means the maintainer
    /// hasn't run `scripts/setup-minisign.sh` yet — signed updates aren't
    /// possible until they do.
    #[error("update signing isn't configured for this build (placeholder public key)")]
    PlaceholderKey,
}

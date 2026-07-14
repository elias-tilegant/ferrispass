//! Cross-platform auto-update via [`cargo-packager-updater`].
//!
//! Two responsibilities:
//! 1. **Check**: verify the complete release manifest before displaying it.
//! 2. **Install**: require the confirmed version, bind the installer candidate
//!    to that signed manifest, then verify and install the signed bundle.
//!
//! Both steps are blocking I/O — call from `cx.background_spawn(...)`, never
//! from the main thread.
//!
//! ## Why cargo-packager-updater (not Sparkle)
//!
//! Sparkle is the gold standard on macOS but macOS-only. FerrisPass aims at
//! Linux and Windows in later releases, so we need a cross-platform updater
//! from day one. `cargo-packager-updater` (by the Tauri team) handles all
//! three platforms with a single Rust API and the same minisign-signed
//! manifest format.
//!
//! ## Manifest URL
//!
//! Hosted on GitHub Releases as `update.json` plus its detached
//! `update.json.minisig`. GitHub's `/releases/latest/download/<asset>` redirect
//! always points at the most recent release, so the URLs stay stable.
//!
//! ## Public key
//!
//! Embedded at compile time from `bundle/minisign-pub.txt` via `include_str!`.
//! That file is committed to the repo; the matching private key lives only
//! on the maintainer's machine + GitHub Secret. Rotating the keypair would
//! invalidate every existing install's update path, so don't.

mod client;
mod info;
#[cfg(target_os = "macos")]
mod macos_installer;
mod notes;
mod status;

pub use client::{check, install};
pub use info::{UpdateError, UpdateInfo};
pub use notes::{PendingWhatsNew, load_for_version as load_whats_new_for_version};
pub use notes::{
    mark_auto_shown as mark_whats_new_auto_shown, save_pending as save_pending_whats_new,
};
pub use status::UpdateStatus;

/// URL of the JSON manifest the updater fetches. The `/latest/download/...`
/// path on GitHub Releases is a server-side redirect to the most recent
/// release's assets — stable across versions, no separate hosting needed.
pub(crate) const UPDATE_ENDPOINT: &str =
    "https://github.com/elias-tilegant/ferrispass/releases/latest/download/update.json";

/// Detached minisign signature over the exact `update.json` bytes. Signing the
/// whole manifest binds version, bundle URL and bundle signature together.
pub(crate) const UPDATE_SIGNATURE_ENDPOINT: &str =
    "https://github.com/elias-tilegant/ferrispass/releases/latest/download/update.json.minisig";

/// Minisign Ed25519 public key, embedded at compile time. Used to verify both
/// the release manifest and every downloaded update bundle before applying it.
///
/// The placeholder shipped in fresh checkouts is a zeroed key — it can verify
/// nothing. Run `scripts/setup-minisign.sh` once to generate a real keypair.
pub(crate) const MINISIGN_PUBLIC_KEY: &str = include_str!("../../bundle/minisign-pub.txt");

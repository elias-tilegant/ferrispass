//! SharePoint cloud sync. Owns the OAuth flow, Microsoft Graph client,
//! per-vault sync configuration, and the upload-on-save orchestration.
//!
//! Short overview:
//!
//! - Auth: device-code flow against `login.microsoftonline.com/common`,
//!   refresh tokens persisted to the macOS Keychain.
//! - Storage: each synced vault has a `SyncConfig` JSON file under the app's
//!   support dir, keyed by the SHA-256 of the canonical local path.
//! - Save flow: `AppState::save_async`'s success branch chains an upload
//!   via `AppState::sync_now_for_path`. Disk saves are serialized per path
//!   with a dirty-on-burst guard (`saves_in_flight`) so rapid edits collapse
//!   into the latest write; the chained upload rides on the last save.
//! - Conflicts: 412 on upload triggers a remote-decrypt + entry-level diff
//!   (see `crate::keepass::merge`); user resolves per entry; merged file is
//!   re-uploaded with the fresh ETag.

pub mod auth;
pub mod config;
pub mod graph;
pub mod service;
pub mod tokens;

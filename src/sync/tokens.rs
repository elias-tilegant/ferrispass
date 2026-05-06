//! Refresh-token persistence via the OS keychain. Thin wrapper around the
//! `keyring` crate so the rest of the sync code doesn't need to know about
//! platform-specific backends.
//!
//! Storage shape: one keychain entry per (provider × account email).
//! - service = `"ferrispass-sync"` (constant)
//! - account = the user's account email (e.g. `alice@contoso.onmicrosoft.com`)
//! - secret  = the OAuth refresh token (opaque string, ~1–4 KB)
//!
//! Multiple accounts can coexist — each lookup is by email. Disconnect
//! removes the entry. Access tokens are *not* stored here; they live in
//! memory inside `SyncBinding` and are short-lived (~1 h) anyway.

use keyring::Entry;
use thiserror::Error;

const SERVICE: &str = "ferrispass-sync";

#[derive(Debug, Error)]
pub enum TokenError {
    #[error("keychain error: {0}")]
    Backend(#[from] keyring::Error),
}

/// Save the refresh token for the given account, overwriting any existing
/// secret. Idempotent (re-saving the same value is a no-op from the user's
/// perspective).
pub fn store(account_email: &str, refresh_token: &str) -> Result<(), TokenError> {
    let entry = Entry::new(SERVICE, account_email)?;
    entry.set_password(refresh_token)?;
    Ok(())
}

/// Read the refresh token for the given account. Returns `Ok(None)` when
/// no entry exists — common case before first connect or after disconnect,
/// not worth typing as an error.
pub fn load(account_email: &str) -> Result<Option<String>, TokenError> {
    let entry = Entry::new(SERVICE, account_email)?;
    match entry.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Remove the refresh token for the given account. No-op when the entry
/// already doesn't exist (Disconnect should be safe to retry).
pub fn delete(account_email: &str) -> Result<(), TokenError> {
    let entry = Entry::new(SERVICE, account_email)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    //! These tests touch the real macOS Keychain. They're `#[ignore]` by
    //! default so `cargo test` stays hermetic; run them explicitly with:
    //!
    //! ```sh
    //! cargo test --lib sync::tokens -- --ignored
    //! ```
    //!
    //! On CI / Linux they'd fail without a working backend; gating on
    //! `target_os = "macos"` keeps that noise away.

    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "touches real macOS Keychain — run explicitly with --ignored"]
    fn round_trip_store_load_delete() {
        let account = format!("test-{}@ferrispass.invalid", std::process::id());
        let token = "abc123-refresh-token";

        // Pre-clean in case a prior run left state behind.
        let _ = delete(&account);
        assert_eq!(load(&account).unwrap(), None);

        store(&account, token).unwrap();
        assert_eq!(load(&account).unwrap().as_deref(), Some(token));

        // Overwrite must replace, not append.
        store(&account, "rotated").unwrap();
        assert_eq!(load(&account).unwrap().as_deref(), Some("rotated"));

        delete(&account).unwrap();
        assert_eq!(load(&account).unwrap(), None);

        // Second delete must be a no-op (idempotent — Disconnect retries).
        delete(&account).unwrap();
    }
}

//! Refresh-token persistence via the OS keychain. Thin wrapper around the
//! `keyring` crate so the rest of the sync code doesn't need to know about
//! platform-specific backends.
//!
//! Storage shape: one keychain entry per (provider × account email).
//! - service = `"ferrispass-sync"` (constant)
//! - account = the user's account email (e.g. `alice@contoso.onmicrosoft.com`)
//! - secret  = the OAuth refresh token (opaque string, ~1-4 KB)
//!
//! Multiple accounts can coexist - each lookup is by email. Disconnect
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
/// no entry exists - common case before first connect or after disconnect,
/// not worth typing as an error.
/// Replace the stored refresh token, but only while the one we started from
/// is still the one stored.
///
/// A refresh runs on a background task and can finish after the user has
/// disconnected, which deletes the entry, or after they have connected the
/// same account again, which writes a newer one. A blind write then recreated
/// an entry for a relationship that is over, or replaced a live token with a
/// stale one. Returns whether it wrote.
pub fn replace(account_email: &str, expected: &str, rotated: &str) -> Result<bool, TokenError> {
    if load(account_email)?.as_deref() != Some(expected) {
        return Ok(false);
    }
    store(account_email, rotated)?;
    Ok(true)
}

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

    /// A refresh that finishes after the user disconnected, or after they
    /// reconnected the same account, must not write. Blind stores recreated
    /// an entry for a relationship that was over, or replaced a live token
    /// with a stale one.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "touches real macOS Keychain - run explicitly with --ignored"]
    fn replace_only_writes_over_the_token_it_started_from() {
        let account = format!("replace-{}@ferrispass.invalid", std::process::id());
        let _ = delete(&account);

        // Nothing stored: a disconnect got there first.
        assert!(!replace(&account, "old", "rotated").expect("keychain"));
        assert_eq!(load(&account).expect("keychain"), None);

        store(&account, "old").expect("keychain");
        assert!(replace(&account, "old", "rotated").expect("keychain"));
        assert_eq!(
            load(&account).expect("keychain").as_deref(),
            Some("rotated")
        );

        // Something newer is stored: a reconnect got there first.
        store(&account, "from-a-newer-connection").expect("keychain");
        assert!(!replace(&account, "old", "rotated").expect("keychain"));
        assert_eq!(
            load(&account).expect("keychain").as_deref(),
            Some("from-a-newer-connection")
        );

        delete(&account).expect("keychain");
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "touches real macOS Keychain - run explicitly with --ignored"]
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

        // Second delete must be a no-op (idempotent - Disconnect retries).
        delete(&account).unwrap();
    }
}

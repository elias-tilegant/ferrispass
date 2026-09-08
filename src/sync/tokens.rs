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

use std::sync::{Mutex, MutexGuard, PoisonError};

use keyring::Entry;
use thiserror::Error;

const SERVICE: &str = "ferrispass-sync";

#[derive(Debug, Error)]
pub enum TokenError {
    #[error("keychain error: {0}")]
    Backend(#[from] keyring::Error),

    /// See `ConfigError::Busy`: the same lock, the same reasoning.
    #[error("another FerrisPass process is using the stored tokens")]
    Busy,

    /// See `ConfigError::Lock`: a lock that cannot be reached is not a lock
    /// somebody else is holding.
    #[error("could not take the sync lock: {0}")]
    Lock(#[source] std::io::Error),
}

impl From<super::lock::Unavailable> for TokenError {
    fn from(reason: super::lock::Unavailable) -> Self {
        match reason {
            super::lock::Unavailable::Contended => Self::Busy,
            super::lock::Unavailable::Broken(error) => Self::Lock(error),
        }
    }
}

impl TokenError {
    /// See `ConfigError::is_busy`.
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Busy)
    }
}

/// Serialises this process's keychain access for these entries.
///
/// The Keychain offers no compare-and-set, so [`replace`] has to read and
/// then write, and the two things it races with, a disconnect's delete and a
/// reconnect's store, both run in this process on background tasks. One lock
/// across each operation is what makes "only while it is still the one
/// stored" true rather than merely likely.
///
/// Always taken *inside* the cross-process lock, never around it. A caller
/// that already holds the file lock reaches these entries through the
/// `*_unlocked` functions, and holding the two in one order everywhere is
/// what keeps that from meeting a thread holding them in the other.
static LOCK: Mutex<()> = Mutex::new(());

fn locked() -> MutexGuard<'static, ()> {
    // A panic while holding this leaves no invariant broken: the guard exists
    // to order calls, not to protect a value.
    LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Save the refresh token for the given account, overwriting any existing
/// secret. Idempotent (re-saving the same value is a no-op from the user's
/// perspective).
pub fn store(account_email: &str, refresh_token: &str) -> Result<(), TokenError> {
    super::lock::held(super::lock::INTERACTIVE, || {
        store_unlocked(account_email, refresh_token)
    })
    .unwrap_or_else(|reason| Err(reason.into()))
}

/// The same write without taking the cross-process lock, for a caller that
/// already holds it because the token and something else have to land as one
/// operation. See `config::save_unlocked`.
pub(super) fn store_unlocked(account_email: &str, refresh_token: &str) -> Result<(), TokenError> {
    let _guard = locked();
    write(account_email, refresh_token)
}

/// Read the refresh token for the given account. Returns `Ok(None)` when
/// no entry exists - common case before first connect or after disconnect,
/// not worth typing as an error.
pub fn load(account_email: &str) -> Result<Option<String>, TokenError> {
    super::lock::held(super::lock::INTERACTIVE, || {
        let _guard = locked();
        read(account_email)
    })
    .unwrap_or_else(|reason| Err(reason.into()))
}

/// Remove the refresh token for the given account. No-op when the entry
/// already doesn't exist (Disconnect should be safe to retry).
pub fn delete(account_email: &str) -> Result<(), TokenError> {
    super::lock::held(super::lock::INTERACTIVE, || {
        let _guard = locked();
        let entry = Entry::new(SERVICE, account_email)?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    })
    .unwrap_or_else(|reason| Err(reason.into()))
}

/// Replace the stored refresh token, but only while the one we started from
/// is still the one stored. Returns whether it wrote.
///
/// A refresh runs on a background task and can finish after the user has
/// disconnected, which deletes the entry, or after they have connected the
/// same account again, which writes a newer one. A blind write then recreated
/// an entry for a relationship that is over, or replaced a live token with a
/// stale one.
pub fn replace(account_email: &str, expected: &str, rotated: &str) -> Result<bool, TokenError> {
    // And across processes: the app and the CLI both write these, and the
    // Keychain offers no compare-and-set of its own.
    super::lock::held(super::lock::INTERACTIVE, || {
        let _guard = locked();
        if read(account_email)?.as_deref() != Some(expected) {
            return Ok(false);
        }
        write(account_email, rotated)?;
        Ok(true)
    })
    .unwrap_or_else(|reason| Err(reason.into()))
}

fn read(account_email: &str) -> Result<Option<String>, TokenError> {
    let entry = Entry::new(SERVICE, account_email)?;
    match entry.get_password() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn write(account_email: &str, refresh_token: &str) -> Result<(), TokenError> {
    let entry = Entry::new(SERVICE, account_email)?;
    entry.set_password(refresh_token)?;
    Ok(())
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

    /// See the matching test in `config`: the same two answers, told apart
    /// for the same reason. This one needs no keychain.
    #[test]
    fn a_lock_that_cannot_be_reached_is_not_something_to_wait_out() {
        let contended: TokenError = super::super::lock::Unavailable::Contended.into();
        assert!(contended.is_busy());

        let broken: TokenError =
            super::super::lock::Unavailable::Broken(std::io::Error::other("gone")).into();
        assert!(!broken.is_busy(), "waiting cannot make the lock appear");
    }

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

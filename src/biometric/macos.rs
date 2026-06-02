//! macOS Touch ID implementation of [`BiometricStore`]. Two-part
//! design that side-steps the data-protection-keychain entitlement
//! (granted only to App-Store apps via provisioning profiles, not to
//! Developer-ID builds like ours):
//!
//! 1. **Storage** — the master password lives in the legacy file-based
//!    keychain (`SecKeychainAddGenericPassword`), with no biometric ACL.
//! 2. **Gate** — `LAContext.evaluatePolicy` runs *before* the read; we
//!    only touch the keychain after the OS confirms Touch ID.
//!
//! The security boundary this draws (and what it deliberately does not
//! protect against) is documented once in `biometric/mod.rs`.
//!
//! `unsafe_code` is `deny` workspace-wide; this file opts in because
//! the objc2 / LocalAuthentication bindings mark ordinary calls
//! `unsafe`. The surface is small: LAContext construction,
//! `evaluatePolicy`, and the `NSError` deref in the reply block.

#![allow(unsafe_code)]

use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_foundation::{NSError, NSString};
use objc2_local_authentication::{LAContext, LAError, LAPolicy};
use security_framework::os::macos::keychain::SecKeychain;
use security_framework::os::macos::passwords::find_generic_password;
use zeroize::Zeroizing;

use crate::biometric::{
    BiometricError, BiometricResult, BiometricStore, EnrollmentId, RetrieveOptions,
};

/// `kSecAttrService` value for every biometric enrolment. Visible to
/// the user in Schlüsselbundverwaltung.app, lets them audit / wipe
/// our items by hand if they ever want to.
const KEYCHAIN_SERVICE: &str = "ferrispass-biometric";

/// Biometry-only policy. Used for the `is_available` / `is_supported`
/// probes and as the retrieve policy when passcode fallback is off.
///
/// Plain `…WithBiometrics`, *not* `…WithBiometricsOrCompanion`: the
/// OrCompanion probe errors on a Mac with no configured Apple Watch
/// even when Touch ID is available, which made the unlock button
/// vanish on every watch-less Mac. Clamshell unlock is served by the
/// passcode-fallback policy instead.
const STRICT_POLICY: LAPolicy = LAPolicy::DeviceOwnerAuthenticationWithBiometrics;

/// Permissive policy for `RetrieveOptions::allow_device_passcode`.
/// Mirrors macOS's "Unlock with Touch ID or password" sheet: Touch ID
/// when the sensor is reachable, the macOS account password otherwise
/// (clamshell mode) or after repeated biometric failures.
const PERMISSIVE_POLICY: LAPolicy = LAPolicy::DeviceOwnerAuthentication;

fn policy_for(options: RetrieveOptions) -> LAPolicy {
    if options.allow_device_passcode {
        PERMISSIVE_POLICY
    } else {
        STRICT_POLICY
    }
}

/// Upper bound for blocking on the LAContext reply. The Touch ID
/// dialog has its own UI-level timeout (≈ 30 s of no fingerprint
/// input before macOS dismisses it); we wait a bit longer so we
/// never time out *before* the OS does, but not so long that a
/// stuck reply leaves the UI hanging forever.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Default)]
pub struct MacOsBiometricStore;

impl MacOsBiometricStore {
    pub fn new() -> Self {
        Self
    }

    /// Legacy keychain helper — same default behaviour as
    /// `security`-cli's `find-generic-password -s … -a …` and what
    /// the `keyring` crate's `apple-native` backend already uses for
    /// sync tokens. Returns `None` for the canonical "not enrolled"
    /// status (`errSecItemNotFound`); other errors are bubbled.
    fn read_stored_password(&self, account: &str) -> BiometricResult<Option<Zeroizing<Vec<u8>>>> {
        match find_generic_password(None, KEYCHAIN_SERVICE, account) {
            Ok((password, _item)) => {
                let bytes: &[u8] = password.as_ref();
                Ok(Some(Zeroizing::new(bytes.to_vec())))
            }
            Err(err) if err.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
            Err(err) => Err(map_keychain_error(err)),
        }
    }
}

impl BiometricStore for MacOsBiometricStore {
    fn is_available(&self) -> bool {
        // "Is the sensor reachable right now?" — drives the dedicated
        // Touch ID button. `false` in clamshell mode; the unlock
        // screen separately keeps the button when passcode fallback
        // can carry that case.
        let ctx = unsafe { LAContext::new() };
        unsafe { ctx.canEvaluatePolicy_error(STRICT_POLICY) }.is_ok()
    }

    fn is_supported(&self) -> bool {
        // "Does this Mac have biometric hardware at all?" — drives the
        // enrolment gate, which must survive a temporarily unreachable
        // sensor (clamshell). `PasscodeNotSet` is the only LAError that
        // rules biometry out at the OS level (no Mac password = no
        // trust anchor); every other code is recoverable or merely
        // transient, so we accept it. `BiometryNotAvailable` is
        // ambiguous (also "no hardware"), but accepting it only costs a
        // hardware-less Mac a button that never fires — far cheaper
        // than locking MacBook users out of enrolment with the lid shut.
        let ctx = unsafe { LAContext::new() };
        match unsafe { ctx.canEvaluatePolicy_error(STRICT_POLICY) } {
            Ok(()) => true,
            Err(err) => err.code() as i64 != LAError::PasscodeNotSet.0 as i64,
        }
    }

    fn enroll(&self, id: &EnrollmentId, password: &str) -> BiometricResult<()> {
        // Use the user's default keychain (login.keychain-db on a
        // standard macOS install). `set_generic_password` is
        // upsert-shaped: it tries find-then-update, falls back to
        // add. Re-enrolling the same vault therefore overwrites the
        // prior bytes without leaving a duplicate.
        let keychain = SecKeychain::default().map_err(map_keychain_error)?;
        keychain
            .set_generic_password(KEYCHAIN_SERVICE, &id.as_str(), password.as_bytes())
            .map_err(map_keychain_error)
    }

    fn retrieve(
        &self,
        id: &EnrollmentId,
        prompt: &str,
        options: RetrieveOptions,
    ) -> BiometricResult<Zeroizing<String>> {
        // Step 1 — drive the OS biometric prompt *before* touching
        // the keychain. The cleartext master password must not enter
        // our process until biometry actually succeeds; reading it up
        // front (even just to fail-fast on a missing item) would leave
        // the secret resident in memory for the whole duration of a
        // prompt the user then cancels. The block is invoked on an
        // internal Apple queue; we marshal the outcome back through a
        // bounded channel and block this (background) thread until it
        // lands.
        let context = unsafe { LAContext::new() };
        let reason = NSString::from_str(prompt);

        let (tx, rx) = mpsc::sync_channel::<Result<(), i64>>(1);
        let reply = RcBlock::new(move |success: Bool, error: *mut NSError| {
            let outcome = if success.as_bool() {
                Ok(())
            } else if error.is_null() {
                // `success=false` without an error object isn't
                // documented behaviour; treat as a generic auth
                // failure so the UI surfaces it as "Touch ID failed".
                Err(LAError::AuthenticationFailed.0 as i64)
            } else {
                // SAFETY: Apple's contract for this block: when
                // `success` is NO, `error` is a valid Objective-C
                // NSError pointer that lives for the duration of
                // the block invocation. We only call `.code()`
                // (which doesn't take ownership) and copy the int
                // out before the block returns.
                let code = unsafe { (*error).code() } as i64;
                Err(code)
            };
            // LocalAuthentication guarantees exactly one reply, so the
            // bounded(1) channel always has room. `try_send` is the
            // defensive choice: if the receiver has already dropped
            // (our task was cancelled while the prompt was up) it
            // returns an error rather than blocking the OS thread; we
            // ignore it because there's nobody left to receive.
            let _ = tx.try_send(outcome);
        });

        let policy = policy_for(options);
        unsafe {
            context.evaluatePolicy_localizedReason_reply(policy, &reason, &reply);
        }

        match rx.recv_timeout(RESPONSE_TIMEOUT) {
            Ok(Ok(())) => {
                // Step 2 — biometry confirmed. *Now* read the keychain
                // item. A NotFound here means the registry pointed at
                // an enrolment whose keychain item is gone (manually
                // deleted, OS reset); surface it so the caller can
                // clean up the stale registry entry.
                let stored = match self.read_stored_password(&id.as_str())? {
                    Some(bytes) => bytes,
                    None => return Err(BiometricError::NotFound),
                };
                let text = std::str::from_utf8(&stored)
                    .map_err(|_| BiometricError::Backend("stored password is not UTF-8".into()))?;
                Ok(Zeroizing::new(text.to_owned()))
            }
            Ok(Err(code)) => Err(map_la_error(code)),
            Err(_) => {
                // Cancel the in-flight evaluation so the prompt
                // doesn't linger if it eventually resolves after
                // we've given up.
                unsafe { context.invalidate() };
                Err(BiometricError::Backend(
                    "Touch ID response timed out".into(),
                ))
            }
        }
    }

    fn forget(&self, id: &EnrollmentId) -> BiometricResult<()> {
        let account = id.as_str();
        match find_generic_password(None, KEYCHAIN_SERVICE, &account) {
            Ok((_password, item)) => {
                // The legacy `SecKeychainItem::delete` wrapper returns
                // `()` — it swallows the OSStatus. To give the caller
                // a trustworthy result (it gates registry cleanup on
                // it) we confirm the deletion with a read-back: a
                // follow-up lookup that misses means the item is
                // genuinely gone.
                item.delete();
                match find_generic_password(None, KEYCHAIN_SERVICE, &account) {
                    Err(err) if err.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
                    Ok(_) => Err(BiometricError::Backend(
                        "keychain item still present after delete".into(),
                    )),
                    Err(err) => Err(map_keychain_error(err)),
                }
            }
            // Idempotent: missing item is "already forgotten".
            Err(err) if err.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
            Err(err) => Err(map_keychain_error(err)),
        }
    }
}

/// Translate a [`security_framework::base::Error`] from the legacy
/// keychain API into our [`BiometricError`]. Centralised so the
/// macOS impl never leaks OSStatus codes into the UI.
pub(crate) fn map_keychain_error(err: security_framework::base::Error) -> BiometricError {
    match err.code() {
        c if c == ERR_SEC_ITEM_NOT_FOUND => BiometricError::NotFound,
        c if c == ERR_SEC_USER_CANCELED => BiometricError::UserCancelled,
        c if c == ERR_SEC_AUTH_FAILED => BiometricError::AuthFailed,
        code => BiometricError::Backend(format!("Keychain OSStatus {code}")),
    }
}

/// Translate an [`LAError`] code (carried out of the reply block as a
/// raw `i64`, so the block stays `Send` without moving Objective-C
/// types across threads) into our [`BiometricError`].
pub(crate) fn map_la_error(code: i64) -> BiometricError {
    use LAError as E;
    let is = |e: LAError| code == e.0 as i64;

    if is(E::AuthenticationFailed) || is(E::PasscodeNotSet) || is(E::BiometryLockout) {
        BiometricError::AuthFailed
    } else if is(E::UserCancel) || is(E::UserFallback) || is(E::SystemCancel) || is(E::AppCancel) {
        BiometricError::UserCancelled
    } else if is(E::BiometryNotAvailable) || is(E::BiometryNotEnrolled) {
        BiometricError::Unsupported
    } else {
        BiometricError::Backend(format!("LAError {code}"))
    }
}

// Apple keychain OSStatus codes (see <Security/SecBase.h>), hardcoded
// because security-framework-sys re-exports only a subset. Stable
// across macOS versions.
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
const ERR_SEC_USER_CANCELED: i32 = -128;
const ERR_SEC_AUTH_FAILED: i32 = -25293;

#[cfg(test)]
mod tests {
    use super::*;

    // -- Policy selection -------------------------------------------------

    /// Default-shaped options must produce the strict policy; an
    /// accidental flip to permissive at the trait default would
    /// silently weaken every Touch-ID prompt for users on older
    /// builds where the setting field is missing from settings.json.
    #[test]
    fn default_options_select_strict_policy() {
        let policy = policy_for(RetrieveOptions::default());
        assert_eq!(policy, STRICT_POLICY);
    }

    #[test]
    fn passcode_fallback_opt_in_selects_permissive_policy() {
        let policy = policy_for(RetrieveOptions {
            allow_device_passcode: true,
        });
        assert_eq!(policy, PERMISSIVE_POLICY);
    }

    // -- LAError mapping (pure, no Keychain / Touch ID needed) ----------

    #[test]
    fn user_cancel_maps_to_cancelled() {
        assert_eq!(
            map_la_error(LAError::UserCancel.0 as i64),
            BiometricError::UserCancelled
        );
    }

    #[test]
    fn user_fallback_maps_to_cancelled() {
        // We don't enable the fallback button, but if some future
        // localisation re-enables it, treat the click as a cancel
        // rather than an auth failure — semantically the user opted
        // out, not failed to authenticate.
        assert_eq!(
            map_la_error(LAError::UserFallback.0 as i64),
            BiometricError::UserCancelled
        );
    }

    #[test]
    fn authentication_failed_maps_to_auth_failed() {
        assert_eq!(
            map_la_error(LAError::AuthenticationFailed.0 as i64),
            BiometricError::AuthFailed
        );
    }

    #[test]
    fn biometry_lockout_maps_to_auth_failed() {
        assert_eq!(
            map_la_error(LAError::BiometryLockout.0 as i64),
            BiometricError::AuthFailed
        );
    }

    #[test]
    fn biometry_not_available_maps_to_unsupported() {
        assert_eq!(
            map_la_error(LAError::BiometryNotAvailable.0 as i64),
            BiometricError::Unsupported
        );
        assert_eq!(
            map_la_error(LAError::BiometryNotEnrolled.0 as i64),
            BiometricError::Unsupported
        );
    }

    #[test]
    fn unknown_la_code_maps_to_backend() {
        match map_la_error(-9999) {
            BiometricError::Backend(msg) => assert!(msg.contains("-9999")),
            other => panic!("expected Backend, got {other:?}"),
        }
    }

    // -- Keychain OSStatus mapping (pure) -----------------------------------

    #[test]
    fn keychain_item_not_found_maps_to_not_found() {
        let err = security_framework::base::Error::from_code(ERR_SEC_ITEM_NOT_FOUND);
        assert_eq!(map_keychain_error(err), BiometricError::NotFound);
    }

    #[test]
    fn keychain_user_canceled_maps_to_cancelled() {
        let err = security_framework::base::Error::from_code(ERR_SEC_USER_CANCELED);
        assert_eq!(map_keychain_error(err), BiometricError::UserCancelled);
    }

    /// Hits the real Keychain *and* triggers a Touch ID prompt.
    /// Ignored by default; run manually with
    /// `cargo test --lib biometric::macos -- --ignored`. Leaves a
    /// uniquely-named keychain item on failure so the user can find
    /// and delete it in Keychain Access.
    #[test]
    #[ignore]
    fn roundtrip_ignored() {
        let store = MacOsBiometricStore::new();
        let id = EnrollmentId::new_random();
        store
            .enroll(&id, "ferrispass-test-secret")
            .expect("enroll must succeed");
        let got = store
            .retrieve(&id, "FerrisPass test", RetrieveOptions::default())
            .expect("retrieve must succeed");
        assert_eq!(&*got, "ferrispass-test-secret");
        store.forget(&id).expect("forget must succeed");
        store.forget(&id).expect("second forget must be no-op");
    }
}

//! Per-vault biometric unlock. The trait + error type live here so the
//! rest of the app can talk to "the biometric store" without knowing
//! which OS implements it. macOS Touch ID is the only real backend
//! today; everything else falls back to [`noop::NoopBiometricStore`].
//!
//! Design points worth keeping in mind when extending this module:
//! - The trait returns `Zeroizing<String>` so the cleartext password
//!   the OS hands back gets wiped on drop. The window between the OS
//!   call and `KeePassRepository::open` consuming the password is the
//!   one place we can plausibly protect.
//! - Enrollment is identified by a UUID, not by the vault path. The
//!   path lives in [`registry::BiometricRegistry`] alongside the UUID
//!   so renaming/moving a vault leaves a clean trail (path lookups
//!   miss, the keychain item is orphaned until "Forget" is invoked).
//! - The trait is `Send + Sync + 'static` so it can sit inside
//!   `Arc<dyn BiometricStore>` and be shared between the foreground UI
//!   thread and the background task that calls `retrieve`. The macOS
//!   keychain call blocks for as long as the OS prompt is open —
//!   always invoke it from `cx.background_spawn`.
//!
//! ## Security boundary (read before changing the macOS backend)
//!
//! The macOS backend stores the master password as a *plain* generic
//! password in the legacy file-based keychain — it has **no biometric
//! ACL** (`kSecAttrAccessControl`). The biometric gate is enforced by
//! us calling `LAContext.evaluatePolicy` before reading the item, not
//! by the OS refusing the read. Two consequences flow from that and
//! must be kept in mind / surfaced to users:
//!
//! 1. **A process in the same login session can read the item without
//!    biometry.** The LA check is voluntary on our part. This is the
//!    same boundary Bitwarden / 1Password draw for their Developer-ID
//!    Mac builds; the data-protection keychain that would harden it
//!    needs an App-Store provisioning profile we can't ship with.
//! 2. **Changing the enrolled fingerprint set does NOT invalidate a
//!    FerrisPass enrolment.** With a real biometric ACL,
//!    `BiometryCurrentSet` would void the item when a finger is
//!    added/removed; here any *current* Touch ID identity (or the
//!    macOS password, when the fallback setting is on) passes the LA
//!    precheck. [`BiometricError::Invalidated`] therefore never
//!    originates from the macOS backend today — it's reserved for a
//!    future hardening pass that compares `LAContext`'s
//!    `evaluatedPolicyDomainState` across unlocks and forces
//!    re-enrolment when the biometric set changes.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

pub mod noop;
pub mod registry;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(test)]
pub mod memory;

pub use noop::NoopBiometricStore;
pub use registry::{BiometricEnrollment, BiometricRegistry, RegistryError};

/// Stable per-enrollment identifier. Used as the keychain account
/// string (`kSecAttrAccount`) and as the registry key on the
/// `BiometricEnrollment`. Newtype so we can't accidentally substitute
/// a path or a vault id.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EnrollmentId(Uuid);

impl EnrollmentId {
    pub fn new_random() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn as_str(&self) -> String {
        self.0.to_string()
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for EnrollmentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Failure modes the unlock-screen UI must distinguish. `Backend` is
/// the catch-all for OS errors we don't model individually — render it
/// as a generic "Touch ID failed" with the message as a tooltip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BiometricError {
    /// The host platform has no biometric backend (Linux/Windows MVP)
    /// or the hardware doesn't support Touch ID.
    Unsupported,
    /// User dismissed the OS prompt without authenticating.
    UserCancelled,
    /// Biometric mismatch or lockout. Caller should fall back to the
    /// password input rather than retry the prompt.
    AuthFailed,
    /// The keychain item's biometric ACL was invalidated (e.g. user
    /// added/removed a fingerprint). Caller must drop the enrollment
    /// and ask the user to re-enroll.
    ///
    /// NOTE: the current macOS backend never produces this — its
    /// legacy-keychain items carry no ACL, so a biometric-set change
    /// can't invalidate them. Kept for the data-protection-keychain
    /// path and a future domain-state comparison (see the module
    /// header's "Security boundary" note).
    Invalidated,
    /// No keychain item exists for this id. Treat as "not enrolled" —
    /// the registry probably has a stale entry that should be cleaned.
    NotFound,
    /// Any other OS-level error. Message is for logs/tooltips, never
    /// for security-relevant logic.
    Backend(String),
}

impl fmt::Display for BiometricError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BiometricError::Unsupported => write!(f, "Biometric unlock is not available"),
            BiometricError::UserCancelled => write!(f, "Touch ID cancelled"),
            BiometricError::AuthFailed => write!(f, "Touch ID did not recognise you"),
            BiometricError::Invalidated => {
                write!(
                    f,
                    "Touch ID enrolment is no longer valid — re-enrol required"
                )
            }
            BiometricError::NotFound => write!(f, "No Touch ID enrolment found for this vault"),
            BiometricError::Backend(msg) => write!(f, "Touch ID error: {msg}"),
        }
    }
}

impl std::error::Error for BiometricError {}

pub type BiometricResult<T> = Result<T, BiometricError>;

/// Per-call options for [`BiometricStore::retrieve`]. Lets callers
/// adjust the auth surface without rewiring the trait every time a
/// new toggle ships. Default = "strict biometry only", so the
/// safest behaviour is what a caller gets without thinking about
/// it; user-visible settings opt into broader policies.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RetrieveOptions {
    /// When `true`, the OS prompt accepts the user's device
    /// passcode (macOS account password on Mac) as a successful
    /// authentication in addition to biometrics. Maps to
    /// `LAPolicy::DeviceOwnerAuthentication` on macOS — the same
    /// policy macOS uses for its own "Unlock with Touch ID or
    /// password" sheets.
    ///
    /// **Security tradeoff:** with this on, anyone who knows the
    /// user's macOS login password can unlock the vault, even
    /// without biometry. The threat boundary collapses to "any
    /// auth-on-this-Mac unlocks the vault". This matches the
    /// model password managers like Bitwarden / 1Password offer
    /// as an opt-in convenience, especially for users in
    /// clamshell mode where the built-in Touch ID sensor is
    /// physically unreachable. Off by default at the trait level;
    /// the app-wide setting opts in (default-on) per the user's
    /// product decision documented in `AppSettings`.
    pub allow_device_passcode: bool,
}

/// Cross-platform contract for "store + read a password protected by
/// the OS biometric prompt". Implementations:
/// - [`macos::MacOsBiometricStore`] — `cfg(target_os = "macos")`
/// - [`noop::NoopBiometricStore`] — every other target + CI
/// - [`memory::InMemoryBiometricStore`] — tests
pub trait BiometricStore: fmt::Debug + Send + Sync + 'static {
    /// Quick capability probe for the UI: should we show the
    /// "Unlock with Touch ID" button at all? Implementations must not
    /// trigger an OS prompt here — this is called on every render.
    ///
    /// "Available" is the strong claim: the sensor is *reachable
    /// right now*. On macOS, this returns `false` while a MacBook is
    /// in clamshell mode (the Touch ID sensor sits in the lid-side
    /// Power button and is physically unreachable when the lid is
    /// closed). Use [`Self::is_supported`] for capability-existence
    /// checks that should survive temporary unreachability.
    fn is_available(&self) -> bool;

    /// Capability probe one step weaker than [`Self::is_available`]:
    /// returns `true` when the host *has* biometric hardware at all,
    /// even if it isn't reachable in the current physical setup. The
    /// UI uses this to decide whether to render the "Enable Touch
    /// ID" enrolment checkbox — enrolment itself only writes to the
    /// keychain and doesn't need a live sensor, so a user in
    /// clamshell mode can still opt in for the next time they open
    /// the lid. Default implementation returns whatever
    /// `is_available` returns; backends with finer-grained
    /// introspection override it.
    fn is_supported(&self) -> bool {
        self.is_available()
    }

    /// Persist `password` against `id` in the OS-backed store with the
    /// biometric ACL applied. Overwrites any prior entry for the same
    /// id (idempotent re-enrol). Must not show a biometric prompt.
    fn enroll(&self, id: &EnrollmentId, password: &str) -> BiometricResult<()>;

    /// Trigger the OS biometric prompt and, on success, return the
    /// stored password. The `prompt` string is shown by the OS (e.g.
    /// "Unlock <vault name>"); pass a UI-friendly hint, never the
    /// vault path. Returns a [`Zeroizing<String>`] so the caller can't
    /// accidentally leave the cleartext on the heap.
    ///
    /// `options` adjusts the auth policy. Backends without an OS-side
    /// policy concept (Noop, InMemory) ignore it; only the macOS
    /// backend currently consumes it.
    fn retrieve(
        &self,
        id: &EnrollmentId,
        prompt: &str,
        options: RetrieveOptions,
    ) -> BiometricResult<Zeroizing<String>>;

    /// Delete the keychain entry for `id`. Idempotent: missing entries
    /// resolve to `Ok(())`.
    fn forget(&self, id: &EnrollmentId) -> BiometricResult<()>;
}

/// Factory used by `app::run` and the production AppState wiring. Tests
/// construct their own store directly.
pub fn default_store() -> Arc<dyn BiometricStore> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(macos::MacOsBiometricStore::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(NoopBiometricStore)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enrollment_id_roundtrips_through_string() {
        let id = EnrollmentId::new_random();
        let s = id.as_str();
        let parsed: Uuid = s.parse().unwrap();
        assert_eq!(parsed, id.as_uuid());
    }

    #[test]
    fn error_display_does_not_leak_paths() {
        // The Backend variant carries OS-level messages; everything
        // else is a fixed string. Sanity-check the fixed strings.
        assert_eq!(
            BiometricError::Unsupported.to_string(),
            "Biometric unlock is not available"
        );
        assert_eq!(
            BiometricError::UserCancelled.to_string(),
            "Touch ID cancelled"
        );
    }
}

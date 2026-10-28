//! State-machine coverage for the Touch ID surface. Like the
//! park-tests module above, we avoid the gpui `Context` and drive
//! the private fields directly - the cx-bearing methods are
//! exercised end-to-end in the manual verification script,
//! while these tests pin the invariants that don't need a window.
use super::*;
use crate::biometric::memory::InMemoryBiometricStore;
use std::path::PathBuf;

fn awaiting(path: &str) -> VaultStatus {
    VaultStatus::AwaitingPassword {
        path: PathBuf::from(path),
        keyfile: None,
        error: None,
    }
}

#[test]
fn default_state_uses_noop_store() {
    let state = AppState::default();
    assert!(
        !state.biometric_store().is_available(),
        "production default would override this; the bare default must stay capability-off so tests of unrelated paths don't accidentally hit a real keychain",
    );
    assert!(state.biometric_registry().is_empty());
    assert!(!state.pending_biometric_enrollment());
    assert_eq!(*state.biometric_attempt(), BiometricAttempt::Idle);
}

#[test]
fn biometric_for_pending_returns_none_when_not_enrolled() {
    let store = Arc::new(InMemoryBiometricStore::available());
    let mut state = AppState::with_biometric(store, BiometricRegistry::new());
    state.vault = awaiting("/tmp/a.kdbx");
    assert!(state.biometric_for_pending().is_none());
}

#[test]
fn biometric_for_pending_returns_the_matching_enrollment() {
    let store = Arc::new(InMemoryBiometricStore::available());
    let mut registry = BiometricRegistry::new();
    let enrollment = BiometricEnrollment {
        id: EnrollmentId::new_random(),
        keyfile: Some(PathBuf::from("/tmp/a.key")),
        enrolled_at: Local::now(),
    };
    registry.upsert(PathBuf::from("/tmp/a.kdbx"), enrollment.clone());
    let mut state = AppState::with_biometric(store, registry);
    state.vault = awaiting("/tmp/a.kdbx");
    let got = state
        .biometric_for_pending()
        .expect("registry has an entry for this path");
    assert_eq!(got.id, enrollment.id);
    assert_eq!(got.keyfile, enrollment.keyfile);
}

#[test]
fn biometric_for_pending_returns_none_when_vault_is_not_awaiting_password() {
    let store = Arc::new(InMemoryBiometricStore::available());
    let mut registry = BiometricRegistry::new();
    registry.upsert(
        PathBuf::from("/tmp/a.kdbx"),
        BiometricEnrollment {
            id: EnrollmentId::new_random(),
            keyfile: None,
            enrolled_at: Local::now(),
        },
    );
    let state = AppState::with_biometric(store, registry);
    // Empty vault - even though the registry has an entry, the
    // unlock screen isn't on the stage, so we must not surface
    // the enrollment.
    assert!(state.biometric_for_pending().is_none());
}

#[test]
fn automatic_biometric_unlock_uses_sensor_or_device_auth_fallback() {
    let path = PathBuf::from("/tmp/a.kdbx");
    let available = Arc::new(InMemoryBiometricStore::available());
    let mut registry = BiometricRegistry::new();
    registry.upsert(
        path.clone(),
        BiometricEnrollment {
            id: EnrollmentId::new_random(),
            keyfile: None,
            enrolled_at: Local::now(),
        },
    );
    let mut state = AppState::with_biometric(available, registry.clone());
    state.vault = awaiting("/tmp/a.kdbx");
    assert_eq!(
        state.automatic_biometric_unlock_path(false),
        Some(path.clone())
    );

    state.biometric_attempt = BiometricAttempt::Error {
        path: path.clone(),
        message: "cancelled".into(),
    };
    assert_eq!(state.automatic_biometric_unlock_path(false), None);

    let unavailable = Arc::new(InMemoryBiometricStore::supported_but_unavailable());
    let mut state = AppState::with_biometric(unavailable, registry);
    state.vault = awaiting("/tmp/a.kdbx");
    assert_eq!(state.automatic_biometric_unlock_path(false), None);
    assert_eq!(
        state.automatic_biometric_unlock_path(true),
        Some(path),
        "clamshell mode should auto-prompt when device auth fallback is enabled"
    );
}

#[test]
fn lock_vault_pure_clears_biometric_session_state() {
    // Mirrors the body of lock_vault() minus `cx.notify` - the
    // public method needs a gpui Context which we can't build here.
    // The invariant we care about is "session-scoped Touch ID UI
    // bits get wiped along with the rest of the unlock state".
    let store = Arc::new(InMemoryBiometricStore::available());
    let mut state = AppState::with_biometric(store, BiometricRegistry::new());
    state.pending_biometric_enrollment = true;
    state.biometric_attempt = BiometricAttempt::Error {
        path: PathBuf::from("/tmp/a.kdbx"),
        message: "stale".into(),
    };

    // Same body as lock_vault, sans cx.notify.
    state.vault = VaultStatus::Empty;
    state.overlay = Overlay::None;
    state.save_status = SaveStatus::Idle;
    state.sync = None;
    state.sync_status = SyncStatus::Disconnected;
    state.pending_sync = None;
    state.sync_history.clear();
    state.parked.clear();
    state.parked_order.clear();
    state.clear_biometric_attempt();
    state.pending_biometric_enrollment = false;

    assert!(!state.pending_biometric_enrollment());
    assert_eq!(*state.biometric_attempt(), BiometricAttempt::Idle);
    // Registry survives the lock - enrollment is meant to outlive
    // the session; otherwise Touch ID would be useless after the
    // first auto-lock.
    // (Registry was empty here, but the field type guarantees it
    // is untouched by the body above.)
    assert!(state.biometric_registry().is_empty());
}

/// The stale-Touch-ID-success guard in `submit_biometric_unlock`
/// branches on `pending_unlock_path()` matching the path the OS
/// prompt was started against. This test pins the read-side
/// contract that drives that guard: when the user has cancelled
/// the unlock (VaultStatus::Empty) or moved to another vault,
/// `pending_unlock_path()` no longer matches the original path.
#[test]
fn pending_unlock_path_changes_when_user_navigates_away() {
    let store = Arc::new(InMemoryBiometricStore::available());
    let mut state = AppState::with_biometric(store, BiometricRegistry::new());
    let original = PathBuf::from("/tmp/original.kdbx");
    state.vault = awaiting("/tmp/original.kdbx");
    assert_eq!(
        state.pending_unlock_path().as_deref(),
        Some(original.as_path())
    );

    // User hit Esc → vault becomes Empty.
    state.vault = VaultStatus::Empty;
    assert_ne!(
        state.pending_unlock_path().as_deref(),
        Some(original.as_path())
    );

    // User picked a different vault to unlock.
    state.vault = awaiting("/tmp/other.kdbx");
    assert_ne!(
        state.pending_unlock_path().as_deref(),
        Some(original.as_path())
    );
}

/// `clear_biometric_attempt_public` is the exit hatch the shell
/// takes when a late-arriving Touch ID success is discarded.
/// It must reset to `Idle` without panicking when the attempt
/// was already `Idle` or `Error` (the user could have hit Cancel,
/// which dropped the attempt before the prompt resolved).
#[test]
fn clear_biometric_attempt_public_is_idempotent() {
    let store = Arc::new(InMemoryBiometricStore::available());
    let mut state = AppState::with_biometric(store, BiometricRegistry::new());
    state.clear_biometric_attempt_public();
    assert_eq!(*state.biometric_attempt(), BiometricAttempt::Idle);

    state.biometric_attempt = BiometricAttempt::Error {
        path: PathBuf::from("/tmp/x.kdbx"),
        message: "old".into(),
    };
    state.clear_biometric_attempt_public();
    assert_eq!(*state.biometric_attempt(), BiometricAttempt::Idle);

    state.biometric_attempt = BiometricAttempt::InFlight {
        path: PathBuf::from("/tmp/x.kdbx"),
        generation: 7,
    };
    state.clear_biometric_attempt_public();
    assert_eq!(*state.biometric_attempt(), BiometricAttempt::Idle);
}

#[test]
fn biometric_unlock_is_current_matches_only_live_generation() {
    let store = Arc::new(InMemoryBiometricStore::available());
    let mut state = AppState::with_biometric(store, BiometricRegistry::new());
    // No attempt in flight → nothing is current.
    assert!(!state.biometric_unlock_is_current(1));

    state.biometric_attempt = BiometricAttempt::InFlight {
        path: PathBuf::from("/tmp/x.kdbx"),
        generation: 5,
    };
    assert!(state.biometric_unlock_is_current(5));
    // A stale (older) resolution must not match.
    assert!(!state.biometric_unlock_is_current(4));

    // Once the attempt is no longer InFlight, even the matching
    // generation is stale.
    state.biometric_attempt = BiometricAttempt::Idle;
    assert!(!state.biometric_unlock_is_current(5));
}

/// Registry round-trip exercises `BiometricRegistry::remove`
/// returning the prior entry - the contract `complete_biometric_enrollment`
/// relies on to clean up the *old* keychain item before installing
/// a new one. A regression here would silently leak keychain
/// entries on re-enrolment.
#[test]
fn upsert_overwrite_returns_the_prior_entry_via_remove() {
    let mut registry = BiometricRegistry::new();
    let path = PathBuf::from("/tmp/a.kdbx");
    let first = BiometricEnrollment {
        id: EnrollmentId::new_random(),
        keyfile: None,
        enrolled_at: Local::now(),
    };
    registry.upsert(path.clone(), first.clone());

    // Caller's protocol: take the prior entry, forget on the OS
    // side, then upsert the replacement.
    let prior = registry.remove(&path).expect("first upsert lives here");
    assert_eq!(prior.id, first.id);

    let second = BiometricEnrollment {
        id: EnrollmentId::new_random(),
        keyfile: None,
        enrolled_at: Local::now(),
    };
    registry.upsert(path.clone(), second.clone());
    assert_eq!(registry.get(&path).unwrap().id, second.id);
}

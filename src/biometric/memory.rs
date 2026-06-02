//! In-memory `BiometricStore` for tests. Records every call so state-
//! transition tests can assert "did we enroll", "did we forget", etc.,
//! and supports forcing the next `retrieve`/`enroll` call to fail with
//! a specific [`BiometricError`] variant.
//!
//! Not gated behind `#[cfg(test)]` only — also reachable from
//! `app::state` tests in another file. We keep the impl inside the
//! same crate so it never ships to release binaries (the module is
//! gated at `src/biometric/mod.rs:32`).

use std::collections::HashMap;
use std::sync::Mutex;

use zeroize::Zeroizing;

use crate::biometric::{
    BiometricError, BiometricResult, BiometricStore, EnrollmentId, RetrieveOptions,
};

#[derive(Debug)]
pub struct InMemoryBiometricStore {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    available: bool,
    /// Capability gate that mirrors `BiometricStore::is_supported`.
    /// Defaults to whatever `available` is so existing tests don't
    /// have to opt in; the clamshell-mode scenario (supported but
    /// not currently available) is constructed via
    /// [`InMemoryBiometricStore::supported_but_unavailable`].
    supported: bool,
    entries: HashMap<EnrollmentId, String>,
    next_retrieve_error: Option<BiometricError>,
    next_enroll_error: Option<BiometricError>,
    enroll_calls: Vec<EnrollmentId>,
    retrieve_calls: Vec<(EnrollmentId, RetrieveOptions)>,
    forget_calls: Vec<EnrollmentId>,
}

impl InMemoryBiometricStore {
    pub fn available() -> Self {
        Self {
            inner: Mutex::new(Inner {
                available: true,
                supported: true,
                ..Inner::default()
            }),
        }
    }

    pub fn unavailable() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
        }
    }

    /// Models the MacBook-in-clamshell-mode case: hardware exists
    /// (so the enrolment UI should still render), but the sensor
    /// isn't reachable right now (so the actual unlock button stays
    /// hidden). `enroll`/`forget` still succeed; `retrieve` returns
    /// `BiometricError::Unsupported` to simulate the OS refusing
    /// the prompt.
    pub fn supported_but_unavailable() -> Self {
        Self {
            inner: Mutex::new(Inner {
                available: false,
                supported: true,
                ..Inner::default()
            }),
        }
    }

    /// Force the next `retrieve` call to fail with `err`. Cleared after
    /// one call.
    pub fn fail_next_retrieve(&self, err: BiometricError) {
        self.inner.lock().unwrap().next_retrieve_error = Some(err);
    }

    /// Force the next `enroll` call to fail with `err`.
    pub fn fail_next_enroll(&self, err: BiometricError) {
        self.inner.lock().unwrap().next_enroll_error = Some(err);
    }

    pub fn enroll_calls(&self) -> Vec<EnrollmentId> {
        self.inner.lock().unwrap().enroll_calls.clone()
    }

    pub fn retrieve_calls(&self) -> Vec<(EnrollmentId, RetrieveOptions)> {
        self.inner.lock().unwrap().retrieve_calls.clone()
    }

    pub fn forget_calls(&self) -> Vec<EnrollmentId> {
        self.inner.lock().unwrap().forget_calls.clone()
    }

    pub fn entry_count(&self) -> usize {
        self.inner.lock().unwrap().entries.len()
    }
}

impl BiometricStore for InMemoryBiometricStore {
    fn is_available(&self) -> bool {
        self.inner.lock().unwrap().available
    }

    fn is_supported(&self) -> bool {
        self.inner.lock().unwrap().supported
    }

    fn enroll(&self, id: &EnrollmentId, password: &str) -> BiometricResult<()> {
        let mut inner = self.inner.lock().unwrap();
        inner.enroll_calls.push(id.clone());
        if let Some(err) = inner.next_enroll_error.take() {
            return Err(err);
        }
        inner.entries.insert(id.clone(), password.to_string());
        Ok(())
    }

    fn retrieve(
        &self,
        id: &EnrollmentId,
        _prompt: &str,
        options: RetrieveOptions,
    ) -> BiometricResult<Zeroizing<String>> {
        let mut inner = self.inner.lock().unwrap();
        inner.retrieve_calls.push((id.clone(), options));
        if let Some(err) = inner.next_retrieve_error.take() {
            return Err(err);
        }
        inner
            .entries
            .get(id)
            .cloned()
            .map(Zeroizing::new)
            .ok_or(BiometricError::NotFound)
    }

    fn forget(&self, id: &EnrollmentId) -> BiometricResult<()> {
        let mut inner = self.inner.lock().unwrap();
        inner.forget_calls.push(id.clone());
        inner.entries.remove(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enroll_then_retrieve_returns_password() {
        let store = InMemoryBiometricStore::available();
        let id = EnrollmentId::new_random();
        store.enroll(&id, "hunter2").unwrap();
        let got = store
            .retrieve(&id, "prompt", RetrieveOptions::default())
            .unwrap();
        assert_eq!(&*got, "hunter2");
    }

    #[test]
    fn retrieve_missing_returns_not_found() {
        let store = InMemoryBiometricStore::available();
        let id = EnrollmentId::new_random();
        assert_eq!(
            store
                .retrieve(&id, "prompt", RetrieveOptions::default())
                .unwrap_err(),
            BiometricError::NotFound
        );
    }

    #[test]
    fn fail_next_retrieve_consumes_after_one_call() {
        let store = InMemoryBiometricStore::available();
        let id = EnrollmentId::new_random();
        store.enroll(&id, "hunter2").unwrap();
        store.fail_next_retrieve(BiometricError::UserCancelled);
        assert_eq!(
            store
                .retrieve(&id, "prompt", RetrieveOptions::default())
                .unwrap_err(),
            BiometricError::UserCancelled
        );
        // Next call succeeds — the injected error was a one-shot.
        assert_eq!(
            &*store
                .retrieve(&id, "prompt", RetrieveOptions::default())
                .unwrap(),
            "hunter2"
        );
    }

    #[test]
    fn forget_removes_entry_and_is_idempotent() {
        let store = InMemoryBiometricStore::available();
        let id = EnrollmentId::new_random();
        store.enroll(&id, "hunter2").unwrap();
        store.forget(&id).unwrap();
        store.forget(&id).unwrap(); // second call must not error
        assert_eq!(
            store
                .retrieve(&id, "prompt", RetrieveOptions::default())
                .unwrap_err(),
            BiometricError::NotFound
        );
    }

    #[test]
    fn supported_but_unavailable_models_clamshell() {
        // The clamshell-mode scenario: enrolment must remain
        // possible (writes to the keychain don't need a live
        // sensor), but the actual retrieve fails with the same
        // "currently unreachable" semantics the OS reports.
        let store = InMemoryBiometricStore::supported_but_unavailable();
        assert!(!store.is_available());
        assert!(store.is_supported(), "user must still be able to enrol");

        let id = EnrollmentId::new_random();
        // Enrol succeeds (no biometric prompt required).
        store.enroll(&id, "hunter2").unwrap();
        // Forget likewise (the user must always be able to back out).
        store.forget(&id).unwrap();
    }

    #[test]
    fn default_supported_tracks_available() {
        // The convenience constructors keep `supported` and
        // `available` in sync; only the clamshell-shaped
        // constructor splits them. Pinning this so a regression
        // in the constructor doesn't surprise a test that uses
        // `InMemoryBiometricStore::available()` without setting
        // anything else.
        assert!(InMemoryBiometricStore::available().is_supported());
        assert!(!InMemoryBiometricStore::unavailable().is_supported());
    }

    #[test]
    fn call_recorders_track_each_op() {
        let store = InMemoryBiometricStore::available();
        let id = EnrollmentId::new_random();
        store.enroll(&id, "p").unwrap();
        let _ = store.retrieve(&id, "prompt", RetrieveOptions::default());
        store.forget(&id).unwrap();
        assert_eq!(store.enroll_calls(), vec![id.clone()]);
        assert_eq!(
            store.retrieve_calls(),
            vec![(id.clone(), RetrieveOptions::default())]
        );
        assert_eq!(store.forget_calls(), vec![id]);
    }

    /// Pins the contract that `RetrieveOptions` is recorded on the
    /// store — production wiring reads `AppSettings::biometric_allow_passcode_fallback`
    /// and forwards it via this struct, so a regression that loses
    /// the options would silently re-strict the OS prompt.
    #[test]
    fn retrieve_options_are_captured_for_assertions() {
        let store = InMemoryBiometricStore::available();
        let id = EnrollmentId::new_random();
        store.enroll(&id, "p").unwrap();
        let options = RetrieveOptions {
            allow_device_passcode: true,
        };
        let _ = store.retrieve(&id, "prompt", options);
        assert_eq!(store.retrieve_calls(), vec![(id, options)]);
    }
}

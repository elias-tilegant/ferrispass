//! No-op implementation used on platforms without a biometric backend
//! (Linux, Windows in this MVP) and in CI runners that can't talk to a
//! real keychain. Every call resolves to
//! [`BiometricError::Unsupported`]; `is_available` is `false`, which is
//! what the UI keys off to hide the Touch ID button entirely.

use zeroize::Zeroizing;

use crate::biometric::{
    BiometricError, BiometricResult, BiometricStore, EnrollmentId, RetrieveOptions,
};

#[derive(Debug, Default)]
pub struct NoopBiometricStore;

impl BiometricStore for NoopBiometricStore {
    fn is_available(&self) -> bool {
        false
    }

    fn enroll(&self, _id: &EnrollmentId, _password: &str) -> BiometricResult<()> {
        Err(BiometricError::Unsupported)
    }

    fn retrieve(
        &self,
        _id: &EnrollmentId,
        _prompt: &str,
        _options: RetrieveOptions,
    ) -> BiometricResult<Zeroizing<String>> {
        Err(BiometricError::Unsupported)
    }

    fn forget(&self, _id: &EnrollmentId) -> BiometricResult<()> {
        Err(BiometricError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_by_default() {
        let store = NoopBiometricStore;
        assert!(!store.is_available());
    }

    #[test]
    fn every_op_returns_unsupported() {
        let store = NoopBiometricStore;
        let id = EnrollmentId::new_random();
        assert_eq!(
            store.enroll(&id, "secret").unwrap_err(),
            BiometricError::Unsupported
        );
        assert_eq!(
            store
                .retrieve(&id, "prompt", RetrieveOptions::default())
                .unwrap_err(),
            BiometricError::Unsupported
        );
        assert_eq!(store.forget(&id).unwrap_err(), BiometricError::Unsupported);
    }
}

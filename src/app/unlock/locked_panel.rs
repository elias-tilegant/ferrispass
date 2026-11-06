//! What the quick access panel offers while it is locked, and when it takes
//! Return.
//!
//! [`kind`] sorts the locked panel into one of four cases, from the state
//! alone:
//!
//! - Nothing is set up: no vault waits on the unlock screen and there is no
//!   recent one.
//! - The vault's file is missing.
//! - Touch ID unlocks from the panel, with the Mac password as the way in
//!   when no sensor is in reach and the setting allows it.
//! - Everything else needs the main window: no Touch ID enrolment for the
//!   vault, no sensor and the Mac password not allowed, a cost the user still
//!   has to confirm, or a header check that has not come back yet. The panel
//!   never asks for the master password itself, and never answers a cost
//!   question.
//!
//! [`return_accepted`] holds Return back until the results are drawn, 300 ms
//! have passed since ([`RETURN_SETTLE`]), and every vault the panel searches
//! has finished unlocking, so Return acts on what you saw rather than on a
//! row that arrived a moment later. The one exception is a single exact
//! match in a vault that was already open, as long as it is not a production
//! system ([`exact_hit_takes_return_at_once`]): that takes Return as soon as
//! it is drawn.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::app::AppState;
use crate::app::kinds::EntryKind;

use super::touch_id::cost_question_settled;

/// What the locked panel offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Return or the unlock button starts Touch ID. `device_password`: no
    /// sensor is in reach (a closed lid), so macOS will ask for the Mac
    /// password instead of a finger.
    TouchId { device_password: bool },
    /// The panel sends you to the main window to unlock.
    NeedsMainWindow,
    /// There is no vault to unlock yet.
    NotSetUp,
    /// The vault's file is not where it was.
    FileMissing(PathBuf),
}

/// The case the locked panel is in. The vault is the one waiting on the
/// unlock screen, or else the most recent one. Only the vault on the unlock
/// screen has had its header read, so for a recent one this answers
/// [`Kind::NeedsMainWindow`] until the panel puts it there
/// (`AppState::request_password`) and the header check is back.
///
/// Meant for a locked app: with a vault open, the panel is not locked and
/// does not ask.
pub fn kind(state: &AppState, allow_device_passcode: bool) -> Kind {
    let Some(path) = state
        .pending_unlock_path()
        .or_else(|| state.recents().first().map(|recent| recent.path.clone()))
    else {
        return Kind::NotSetUp;
    };
    match path.try_exists() {
        Ok(true) => {}
        Ok(false) => return Kind::FileMissing(path),
        // Unreadable rather than gone: the main window shows the real error.
        Err(_) => return Kind::NeedsMainWindow,
    }
    if state.biometric_registry().get(&path).is_none() || !cost_question_settled(state) {
        return Kind::NeedsMainWindow;
    }
    if state.biometric_store().is_available() {
        Kind::TouchId {
            device_password: false,
        }
    } else if allow_device_passcode {
        Kind::TouchId {
            device_password: true,
        }
    } else {
        Kind::NeedsMainWindow
    }
}

/// How long after the results are drawn Return starts to count.
pub const RETURN_SETTLE: Duration = Duration::from_millis(300);

/// What the panel knows when Return comes in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReturnGate {
    /// When the results on screen were drawn; `None` while there are none.
    pub results_drawn_at: Option<Instant>,
    /// Whether every vault the panel searches has finished unlocking.
    pub all_vaults_done: bool,
    /// Whether the results are one exact match in a vault that was already
    /// open, and not a production system. Set it from
    /// [`exact_hit_takes_return_at_once`].
    pub single_exact_hit_in_open_vault_not_production: bool,
}

/// Whether Return at `now` acts on the results, or is dropped.
pub fn return_accepted(gate: &ReturnGate, now: Instant) -> bool {
    let Some(drawn) = gate.results_drawn_at else {
        return false;
    };
    if gate.single_exact_hit_in_open_vault_not_production {
        return true;
    }
    gate.all_vaults_done && now.saturating_duration_since(drawn) >= RETURN_SETTLE
}

/// Whether the results qualify for Return at once: `exact_hits` is the number
/// of exact matches and must be one, `vault_was_open` says whether its vault
/// was open before this search, and `kind` is what the entry is. A production
/// system always waits.
pub fn exact_hit_takes_return_at_once(
    exact_hits: usize,
    vault_was_open: bool,
    kind: &EntryKind,
) -> bool {
    exact_hits == 1 && vault_was_open && !kind.is_production()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::kinds::classify;
    use crate::biometric::memory::InMemoryBiometricStore;
    use crate::biometric::registry::{BiometricEnrollment, BiometricRegistry};
    use crate::biometric::{BiometricStore, EnrollmentId};
    use crate::domain::VaultEntry;
    use crate::keepass::limits::{Argon2Variant, KdfCost};
    use gpui::{AppContext as _, TestAppContext};
    use std::path::Path;
    use std::sync::Arc;

    const NOWHERE: &str = "/nonexistent/ferrispass-locked-panel-tests/nordhafen.kdbx";

    /// What the header check found for the vault on the unlock screen.
    enum Header {
        NotBack,
        Fine,
        AsksForCost,
    }

    /// `path` on the unlock screen. `enrolled` and `sensor` set up Touch ID.
    fn locked(
        cx: &mut TestAppContext,
        path: &Path,
        enrolled: bool,
        sensor: bool,
        header: Header,
        allow_device_passcode: bool,
    ) -> Kind {
        let store: Arc<dyn BiometricStore> = Arc::new(if sensor {
            InMemoryBiometricStore::available()
        } else {
            InMemoryBiometricStore::supported_but_unavailable()
        });
        let mut registry = BiometricRegistry::new();
        if enrolled {
            registry.upsert(
                path.to_path_buf(),
                BiometricEnrollment {
                    id: EnrollmentId::new_random(),
                    keyfile: None,
                    enrolled_at: chrono::Local::now(),
                },
            );
        }
        let state = cx.new(|cx| {
            let mut state = AppState::with_biometric(store, registry);
            state.request_password(path.to_path_buf(), cx);
            let cost = match header {
                Header::NotBack => return state,
                Header::Fine => None,
                Header::AsksForCost => Some(KdfCost {
                    variant: Argon2Variant::Argon2id,
                    memory: 2 * 1024 * 1024 * 1024,
                    iterations: 3,
                    parallelism: 4,
                }),
            };
            let check = state.begin_kdf_check(cx);
            state.finish_kdf_check(check, path.to_path_buf(), cost, cx);
            state
        });
        state.read_with(cx, |state, _| kind(state, allow_device_passcode))
    }

    #[gpui::test]
    fn locked_panel_kind_for_every_case(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().expect("a folder for the vault");
        let vault = dir.path().join("nordhafen.kdbx");
        std::fs::write(&vault, b"not opened here").expect("a vault file");
        let missing = Path::new(NOWHERE);

        let nothing = cx.new(|_| AppState::default());
        assert_eq!(
            nothing.read_with(cx, |state, _| kind(state, true)),
            Kind::NotSetUp
        );

        // (file, enrolled, sensor, header, Mac password allowed) -> kind
        let cases = [
            (
                missing,
                true,
                true,
                Header::Fine,
                true,
                Kind::FileMissing(missing.to_path_buf()),
            ),
            (
                vault.as_path(),
                true,
                true,
                Header::AsksForCost,
                true,
                Kind::NeedsMainWindow,
            ),
            (
                vault.as_path(),
                true,
                true,
                Header::NotBack,
                true,
                Kind::NeedsMainWindow,
            ),
            (
                vault.as_path(),
                false,
                true,
                Header::Fine,
                true,
                Kind::NeedsMainWindow,
            ),
            (
                vault.as_path(),
                true,
                true,
                Header::Fine,
                false,
                Kind::TouchId {
                    device_password: false,
                },
            ),
            (
                vault.as_path(),
                true,
                false,
                Header::Fine,
                true,
                Kind::TouchId {
                    device_password: true,
                },
            ),
            (
                vault.as_path(),
                true,
                false,
                Header::Fine,
                false,
                Kind::NeedsMainWindow,
            ),
        ];
        for (index, (path, enrolled, sensor, header, allowed, expected)) in
            cases.into_iter().enumerate()
        {
            assert_eq!(
                locked(cx, path, enrolled, sensor, header, allowed),
                expected,
                "case {index}"
            );
        }
    }

    fn drawn_at(at: Instant) -> ReturnGate {
        ReturnGate {
            results_drawn_at: Some(at),
            all_vaults_done: true,
            single_exact_hit_in_open_vault_not_production: false,
        }
    }

    #[test]
    fn return_before_results_are_drawn_is_dropped() {
        let now = Instant::now();
        let nothing_drawn = ReturnGate {
            results_drawn_at: None,
            all_vaults_done: true,
            single_exact_hit_in_open_vault_not_production: false,
        };
        assert!(!return_accepted(&nothing_drawn, now));
        assert!(!return_accepted(
            &nothing_drawn,
            now + Duration::from_secs(5)
        ));
        let exception_not_drawn = ReturnGate {
            single_exact_hit_in_open_vault_not_production: true,
            ..nothing_drawn
        };
        assert!(
            !return_accepted(&exception_not_drawn, now + Duration::from_secs(5)),
            "Return acted on a row nobody saw"
        );
    }

    #[test]
    fn return_within_300ms_of_drawing_is_dropped() {
        let drawn = Instant::now();
        let gate = drawn_at(drawn);
        assert!(!return_accepted(&gate, drawn));
        assert!(!return_accepted(&gate, drawn + Duration::from_millis(299)));
        assert!(return_accepted(&gate, drawn + Duration::from_millis(300)));
        assert!(return_accepted(&gate, drawn + Duration::from_secs(10)));
    }

    #[test]
    fn return_waits_for_every_vault_of_the_set() {
        let drawn = Instant::now();
        let later = drawn + Duration::from_secs(1);
        let still_unlocking = ReturnGate {
            all_vaults_done: false,
            ..drawn_at(drawn)
        };
        assert!(!return_accepted(&still_unlocking, later));
        assert!(return_accepted(&drawn_at(drawn), later));
    }

    #[test]
    fn one_exact_hit_in_an_open_vault_takes_return_at_once_unless_production() {
        let drawn = Instant::now();
        let entry = |title: &str| {
            classify(
                &VaultEntry {
                    title: title.to_string(),
                    ..VaultEntry::default()
                },
                None,
            )
        };
        let quality = entry("NHQ 100");
        let production = entry("NHP 100");
        assert!(!quality.is_production());
        assert!(production.is_production());

        // Still unlocking another vault, and drawn just now.
        let at_once =
            |kind: &EntryKind, exact_hits: usize, vault_was_open: bool| {
                let gate =
                    ReturnGate {
                        results_drawn_at: Some(drawn),
                        all_vaults_done: false,
                        single_exact_hit_in_open_vault_not_production:
                            exact_hit_takes_return_at_once(exact_hits, vault_was_open, kind),
                    };
                return_accepted(&gate, drawn)
            };
        assert!(at_once(&quality, 1, true));
        assert!(
            !at_once(&production, 1, true),
            "a production system took Return at once"
        );
        assert!(
            !at_once(&quality, 2, true),
            "two exact matches are no exception"
        );
        assert!(
            !at_once(&quality, 1, false),
            "a vault this unlock opened is no exception"
        );
    }
}

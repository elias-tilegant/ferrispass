//! The unlock screen's own Touch ID prompt next to the quick access panel's.
//!
//! The unlock screen schedules its prompt when it comes to the front and
//! starts it a moment later. An attempt the panel starts in between must not
//! be replaced by it: the second attempt would make the first one stale,
//! and the vault would stay locked after two prompts.

use super::*;
use crate::app::services::clipboard::ClipboardService;
use crate::app::unlock::touch_id::{self, Answer, PanelTrigger};
use crate::biometric::memory::InMemoryBiometricStore;
use crate::biometric::registry::{BiometricEnrollment, BiometricRegistry};
use crate::biometric::{BiometricError, BiometricStore, EnrollmentId};
use crate::clipboard::{AccessBehavior, fake::FakePasteboard};
use crate::platform::secure_input::{self, fake::FakeApi};
use gpui::{TestAppContext, WindowHandle};
use gpui_component::Root;
use keepass::config::{DatabaseConfig, KdfConfig};
use keepass::{Database, DatabaseKey};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// The unlock screen of an enrolled vault, in a window of the real shell.
struct Screen {
    state: Entity<AppState>,
    store: Arc<InMemoryBiometricStore>,
    shell: Entity<AppShell>,
    window: WindowHandle<Root>,
    path: PathBuf,
    _dir: tempfile::TempDir,
}

/// What every shell here needs: the GPUI Kit, the actions, and fakes for
/// the clipboard and secure input.
fn app(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::app::actions::init(cx);
        ClipboardService::install_for_test(
            cx,
            Box::new(FakePasteboard::new(AccessBehavior::Default)),
        );
        secure_input::fake::install(cx, FakeApi::new());
    });
}

/// The unlock screen for a vault file that exists, enrolled for Touch ID
/// with the sensor in reach. The window comes to the front while the
/// vault's header check is still out, so the screen schedules nothing yet.
fn screen(cx: &mut TestAppContext) -> Screen {
    let dir = tempfile::tempdir().expect("a folder for the vault");
    let path = dir.path().join("nordhafen.kdbx");
    let database = Database::with_config(DatabaseConfig {
        kdf_config: KdfConfig::Aes { rounds: 1_000 },
        ..DatabaseConfig::default()
    });
    let mut file = std::fs::File::create(&path).expect("create the vault file");
    database
        .save(&mut file, DatabaseKey::new().with_password("pw"))
        .expect("write the vault file");
    let store = Arc::new(InMemoryBiometricStore::available());
    let id = EnrollmentId::new_random();
    store.enroll(&id, "pw").expect("enrol");
    let mut registry = BiometricRegistry::new();
    registry.upsert(
        path.clone(),
        BiometricEnrollment {
            id,
            keyfile: None,
            enrolled_at: chrono::Local::now(),
        },
    );
    let shared: Arc<dyn BiometricStore> = store.clone();
    let waiting = path.clone();
    let state = cx.new(|cx| {
        let mut state = AppState::with_biometric(shared, registry);
        state.request_password(waiting, cx);
        state
    });
    let mut shell = None;
    let window = cx.add_window(|window, cx| {
        let view = cx.new(|cx| AppShell::new(state.clone(), window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    // A header check that has not come back holds the prompt.
    state.update(cx, |state, cx| {
        state.begin_kdf_check(cx);
    });
    AnyWindowHandle::from(window)
        .update(cx, |_, window, _| window.activate_window())
        .expect("the window");
    cx.run_until_parked();
    let screen = Screen {
        state,
        store,
        shell: shell.expect("the shell"),
        window,
        path,
        _dir: dir,
    };
    assert_eq!(screen.retrieves(), 0, "a prompt before the header was read");
    screen
}

impl Screen {
    fn retrieves(&self) -> usize {
        self.store.retrieve_calls().len()
    }

    /// The header check comes back and the screen schedules its prompt.
    /// When `panel` says so, the panel starts its attempt right after, in
    /// the same turn, before the scheduled prompt starts. Then everything
    /// runs. Returns the panel's answer, if it had one.
    fn schedule(&self, cx: &mut TestAppContext, panel: bool) -> Vec<Answer> {
        let answers = Rc::new(RefCell::new(Vec::new()));
        let noted = answers.clone();
        let (shell, state, path) = (self.shell.clone(), self.state.clone(), self.path.clone());
        AnyWindowHandle::from(self.window)
            .update(cx, |_, window, cx| {
                state.update(cx, |state, cx| {
                    let check = state.begin_kdf_check(cx);
                    state.finish_kdf_check(check, path, None, cx);
                });
                shell.update(cx, |shell, cx| {
                    shell.schedule_automatic_biometric_unlock(window, cx);
                });
                if panel {
                    let token = touch_id::confirm(PanelTrigger::ReturnKey).expect("Return");
                    let on_done = Box::new(move |answer, _: &mut gpui::App| {
                        noted.borrow_mut().push(answer);
                    });
                    touch_id::unlock_from_panel(token, state, false, Box::new(|_| {}), on_done, cx)
                        .expect("the panel's attempt starts");
                }
            })
            .expect("the window");
        cx.run_until_parked();
        answers.borrow().clone()
    }
}

/// The screen schedules its prompt, the panel starts an attempt before the
/// prompt starts, and then the scheduled start runs: it starts nothing, so
/// Touch ID asks once, and the panel's attempt is the one answered. Without
/// the panel's attempt, the screen's own prompt runs as before.
#[gpui::test]
fn the_automatic_prompt_yields_to_a_panel_attempt(cx: &mut TestAppContext) {
    app(cx);
    let screen = screen(cx);
    screen
        .store
        .fail_next_retrieve(BiometricError::UserCancelled);
    let answers = screen.schedule(cx, true);
    assert_eq!(
        screen.retrieves(),
        1,
        "the automatic prompt replaced the panel's"
    );
    assert_eq!(
        answers,
        vec![Answer::Failed(BiometricError::UserCancelled)],
        "the panel's attempt was not the one answered"
    );

    let screen = self::screen(cx);
    screen
        .store
        .fail_next_retrieve(BiometricError::UserCancelled);
    let answers = screen.schedule(cx, false);
    assert!(answers.is_empty());
    assert_eq!(screen.retrieves(), 1, "the automatic prompt did not run");
}

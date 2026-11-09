//! The shared Touch ID flow, through its entries and, where the lock screen's
//! own rules are the point, through the shell.
//!
//! The store is the in-memory one, wrapped so a test can script what each
//! prompt answers and read back what it was shown. The vault's password is
//! always enrolled there and no script answers `NotFound` or `Invalidated`:
//! either would make `fail_biometric_unlock` rewrite the Touch ID registry,
//! which lives under the user's Application Support folder.
//!
//! Opening a vault writes the list of recent vaults to that folder too, and
//! the shell reads its settings from there. Tests that do either run again in
//! a child process with a home of its own ([`in_a_home_of_its_own`]). The
//! others point at a vault in a folder that does not exist, so nothing opens
//! and nothing is written.

use super::*;
use crate::app::AppSettings;
use crate::app::BiometricAttempt;
use crate::app::services::clipboard::ClipboardService;
use crate::app::unlock::locked_panel::{self, Kind};
use crate::app::unlock::typed_ahead::TypedAhead;
use crate::biometric::memory::InMemoryBiometricStore;
use crate::biometric::registry::{BiometricEnrollment, BiometricRegistry};
use crate::biometric::{BiometricResult, BiometricStore, EnrollmentId};
use crate::clipboard::{AccessBehavior, fake::FakePasteboard};
use crate::keepass::VaultDocument;
use crate::keepass::limits::{Argon2Variant, KdfCost};
use crate::keepass::repository::snapshot_from_database;
use crate::platform::secure_input::{self, fake::FakeApi};
use crate::ui::AppShell;
use gpui::{AnyWindowHandle, TestAppContext, VisualTestContext, Window, WindowHandle};
use gpui_component::Root;
use keepass::config::{DatabaseConfig, KdfConfig};
use keepass::db::fields;
use keepass::{Database, DatabaseKey};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use zeroize::Zeroizing;

/// The master password of every vault here.
const PASSWORD: &str = "correct horse battery staple";

/// The parent folder does not exist, so this vault never opens and nothing
/// is ever written for it.
const NOWHERE: &str = "/nonexistent/ferrispass-touch-id-tests/nordhafen.kdbx";

/// The in-memory store, with a script of answers and a record of the
/// prompts it was shown.
#[derive(Debug)]
struct Store {
    inner: InMemoryBiometricStore,
    script: Mutex<VecDeque<BiometricError>>,
    prompts: Mutex<Vec<String>>,
}

impl Store {
    fn new(inner: InMemoryBiometricStore) -> Arc<Self> {
        Arc::new(Self {
            inner,
            script: Mutex::new(VecDeque::new()),
            prompts: Mutex::new(Vec::new()),
        })
    }

    /// The next prompts fail with `errors`, one each and in order. Prompts
    /// after that give the stored password.
    fn answering(&self, errors: impl IntoIterator<Item = BiometricError>) {
        let mut script = self.script.lock().unwrap_or_else(PoisonError::into_inner);
        for error in errors {
            assert!(
                !matches!(
                    error,
                    BiometricError::NotFound | BiometricError::Invalidated
                ),
                "{error:?} makes the unlock rewrite the Touch ID registry on disk"
            );
            script.push_back(error);
        }
    }

    /// How many times the Touch ID prompt was shown.
    fn retrieves(&self) -> usize {
        self.inner.retrieve_calls().len()
    }

    fn prompts(&self) -> Vec<String> {
        self.prompts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl BiometricStore for Store {
    fn is_available(&self) -> bool {
        self.inner.is_available()
    }

    fn is_supported(&self) -> bool {
        self.inner.is_supported()
    }

    fn enroll(&self, id: &EnrollmentId, password: &str) -> BiometricResult<()> {
        self.inner.enroll(id, password)
    }

    fn retrieve(
        &self,
        id: &EnrollmentId,
        prompt: &str,
        options: RetrieveOptions,
    ) -> BiometricResult<Zeroizing<String>> {
        self.prompts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(prompt.to_string());
        let next = self
            .script
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        if let Some(error) = next {
            self.inner.fail_next_retrieve(error);
        }
        self.inner.retrieve(id, prompt, options)
    }

    fn forget(&self, id: &EnrollmentId) -> BiometricResult<()> {
        self.inner.forget(id)
    }
}

/// A vault waiting on the unlock screen, enrolled for Touch ID.
struct Vault {
    state: Entity<AppState>,
    store: Arc<Store>,
    path: PathBuf,
}

/// `path` on the unlock screen, enrolled, with [`PASSWORD`] in the store.
/// `sensor` says whether Touch ID is reachable.
fn waiting(cx: &mut TestAppContext, path: &Path, sensor: InMemoryBiometricStore) -> Vault {
    let store = Store::new(sensor);
    let id = EnrollmentId::new_random();
    store.inner.enroll(&id, PASSWORD).expect("enrol");
    let mut registry = BiometricRegistry::new();
    registry.upsert(
        path.to_path_buf(),
        BiometricEnrollment {
            id,
            keyfile: None,
            enrolled_at: chrono::Local::now(),
        },
    );
    let shared: Arc<dyn BiometricStore> = store.clone();
    let state = cx.new(|cx| {
        let mut state = AppState::with_biometric(shared, registry);
        state.request_password(path.to_path_buf(), cx);
        state
    });
    Vault {
        state,
        store,
        path: path.to_path_buf(),
    }
}

/// A vault file in `dir` that opens with [`PASSWORD`]. AES-KDF with few
/// rounds keeps the open quick.
fn vault_file(dir: &Path) -> PathBuf {
    let path = dir.join("nordhafen.kdbx");
    let mut database = Database::with_config(DatabaseConfig {
        kdf_config: KdfConfig::Aes { rounds: 1_000 },
        ..DatabaseConfig::default()
    });
    database
        .root_mut()
        .add_entry()
        .set_unprotected(fields::TITLE, "NHQ 100");
    let mut file = std::fs::File::create(&path).expect("create the vault file");
    database
        .save(&mut file, DatabaseKey::new().with_password(PASSWORD))
        .expect("write the vault file");
    path
}

type Answers = Rc<RefCell<Vec<(&'static str, Answer)>>>;

/// An `on_done` that notes the answer under `name`.
fn noting(answers: &Answers, name: &'static str) -> OnDone {
    let answers = answers.clone();
    Box::new(move |answer, _| answers.borrow_mut().push((name, answer)))
}

fn open_path(cx: &mut TestAppContext, state: &Entity<AppState>) -> Option<PathBuf> {
    state.read_with(cx, |state, _| match state.vault_status() {
        VaultStatus::Open { path, .. } => Some(path.clone()),
        _ => None,
    })
}

/// Runs the calling test again in a child process of this test binary, with
/// `HOME` and `TMPDIR` pointing at a new temporary folder, and says whether
/// the caller is that child.
///
/// Opening a vault writes the list of recent vaults under
/// `~/Library/Application Support/ferrispass` and sweeps the launch folder in
/// the temporary directory, and the shell reads its settings from the same
/// place. A test that does either goes on only in the child, where all of
/// that lands in the throwaway folder. In the parent this starts the child,
/// requires that exactly this test ran there and passed, and returns false.
fn in_a_home_of_its_own(test: &str) -> bool {
    const OWN_HOME: &str = "FERRISPASS_TEST_OWN_HOME";
    if let Some(home) = std::env::var_os(OWN_HOME) {
        assert_eq!(
            std::env::var_os("HOME"),
            Some(home),
            "a child run has to have the home it was given"
        );
        return true;
    }
    let home = tempfile::tempdir().expect("a temporary home");
    let module = module_path!()
        .split_once("::")
        .map_or(module_path!(), |(_, rest)| rest);
    let name = format!("{module}::{test}");
    let output = std::process::Command::new(std::env::current_exe().expect("the test binary"))
        .args([name.as_str(), "--exact", "--test-threads=1"])
        .env(OWN_HOME, home.path())
        .env("HOME", home.path())
        .env("TMPDIR", home.path())
        .output()
        .expect("run the test binary");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("test result: ok. 1 passed"),
        "{name} failed in a home of its own:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    false
}

/// The unlock screen in a window of the real shell.
struct LockScreen {
    vault: Vault,
    shell: Entity<AppShell>,
    window: WindowHandle<Root>,
}

impl LockScreen {
    /// The shell for `path`, in a window that is not in front. Secure input
    /// and the clipboard are fakes.
    fn new(cx: &mut TestAppContext, path: &Path) -> Self {
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::app::actions::init(cx);
            ClipboardService::install_for_test(
                cx,
                Box::new(FakePasteboard::new(AccessBehavior::Default)),
            );
            secure_input::fake::install(cx, FakeApi::new());
        });
        let vault = waiting(cx, path, InMemoryBiometricStore::available());
        let mut shell = None;
        let window = cx.add_window(|window, cx| {
            let view = cx.new(|cx| AppShell::new(vault.state.clone(), window, cx));
            shell = Some(view.clone());
            Root::new(view, window, cx)
        });
        cx.run_until_parked();
        Self {
            vault,
            shell: shell.expect("shell view"),
            window,
        }
    }

    fn in_window<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut AppShell, &mut Window, &mut Context<AppShell>) -> R,
    ) -> R {
        let shell = self.shell.clone();
        AnyWindowHandle::from(self.window)
            .update(cx, |_, window, cx| {
                shell.update(cx, |shell, cx| f(shell, window, cx))
            })
            .expect("the window is open")
    }

    /// The window comes to the front.
    fn activate(&self, cx: &mut TestAppContext) {
        AnyWindowHandle::from(self.window)
            .update(cx, |_, window, _| window.activate_window())
            .expect("the window is open");
        cx.run_until_parked();
    }

    /// Another app comes to the front.
    fn deactivate(&self, cx: &mut TestAppContext) {
        VisualTestContext::from_window(self.window.into(), cx).deactivate_window();
        cx.run_until_parked();
        let in_front = AnyWindowHandle::from(self.window)
            .update(cx, |_, window, _| window.is_window_active())
            .expect("the window is open");
        assert!(!in_front, "the window is still in front");
    }

    /// macOS withdraws a click with the window in front, and another with
    /// the window in the background, which then comes back to the front.
    /// Says how many prompts each click brought, its own included.
    fn withdraw_two_clicks(&self, cx: &mut TestAppContext) -> Vec<(&'static str, usize)> {
        let store = &self.vault.store;
        let click = |cx: &mut TestAppContext| {
            self.in_window(cx, |shell, window, cx| {
                shell.submit_biometric_unlock(window, cx)
            });
            cx.run_until_parked();
        };
        let mut prompts = Vec::new();

        let before = store.retrieves();
        store.answering([BiometricError::Interrupted]);
        click(cx);
        prompts.push(("in front", store.retrieves() - before));

        self.deactivate(cx);
        let before = store.retrieves();
        store.answering([BiometricError::Interrupted]);
        click(cx);
        self.activate(cx);
        prompts.push((
            "in the background, then back in front",
            store.retrieves() - before,
        ));
        prompts
    }
}

/// What [`LockScreen::withdraw_two_clicks`] has to find: each click brought
/// its own prompt and no other.
const ONE_PROMPT_EACH: [(&str, usize); 2] = [
    ("in front", 1),
    ("in the background, then back in front", 1),
];

#[gpui::test]
fn the_lock_screen_unlocks_through_the_shared_flow(cx: &mut TestAppContext) {
    if !in_a_home_of_its_own("the_lock_screen_unlocks_through_the_shared_flow") {
        return;
    }
    let dir = tempfile::tempdir().expect("a folder for the vault");
    let path = vault_file(dir.path());
    let screen = LockScreen::new(cx, &path);
    assert_eq!(
        screen.vault.store.retrieves(),
        0,
        "the window is not in front, so there is no automatic prompt"
    );

    screen.in_window(cx, |shell, window, cx| {
        shell.submit_biometric_unlock(window, cx)
    });
    cx.run_until_parked();

    assert_eq!(screen.vault.store.retrieves(), 1);
    assert_eq!(screen.vault.store.prompts(), ["Unlock nordhafen"]);
    // The settings are the defaults here. Whether the prompt follows the
    // setting when it is changed is the next test.
    assert_eq!(
        screen.vault.store.inner.retrieve_calls()[0]
            .1
            .allow_device_passcode,
        AppSettings::default().biometric_allow_passcode_fallback,
        "the prompt did not follow the default setting for the Mac password"
    );
    assert_eq!(open_path(cx, &screen.vault.state), Some(path));
    screen.vault.state.read_with(cx, |state, _| {
        assert_eq!(*state.biometric_attempt(), BiometricAttempt::Idle);
    });
}

/// The unlock screen's prompt takes the Mac password only while the setting
/// allows it. The setting is turned off while the shell runs, and the window
/// comes to the front, which raises the screen's own prompt. Then the unlock
/// is clicked, the setting is turned on again, and the unlock is clicked once
/// more.
#[gpui::test]
fn the_lock_screen_prompt_follows_the_mac_password_setting(cx: &mut TestAppContext) {
    if !in_a_home_of_its_own("the_lock_screen_prompt_follows_the_mac_password_setting") {
        return;
    }
    let screen = LockScreen::new(cx, Path::new(NOWHERE));
    let allow = |cx: &mut TestAppContext, allowed: bool| {
        screen.in_window(cx, |shell, _, cx| {
            let settings = AppSettings {
                biometric_allow_passcode_fallback: allowed,
                ..shell.settings().clone()
            };
            shell.update_settings(settings, cx);
        });
    };
    let asked = || -> Vec<bool> {
        screen
            .vault
            .store
            .inner
            .retrieve_calls()
            .iter()
            .map(|(_, options)| options.allow_device_passcode)
            .collect()
    };

    allow(cx, false);
    screen
        .vault
        .store
        .answering([BiometricError::UserCancelled]);
    screen.activate(cx);
    assert_eq!(
        asked(),
        [false],
        "the prompt the unlock screen raised on its own took the Mac password with the setting off"
    );

    for allowed in [false, true] {
        allow(cx, allowed);
        screen.in_window(cx, |shell, window, cx| {
            shell.submit_biometric_unlock(window, cx)
        });
        cx.run_until_parked();
    }
    assert_eq!(
        asked(),
        [false, false, true],
        "a clicked prompt did not follow the setting for the Mac password, off and then on"
    );
}

#[gpui::test]
fn an_interrupted_automatic_prompt_is_retried_once_on_the_lock_screen(cx: &mut TestAppContext) {
    if !in_a_home_of_its_own("an_interrupted_automatic_prompt_is_retried_once_on_the_lock_screen") {
        return;
    }
    let screen = LockScreen::new(cx, Path::new(NOWHERE));
    assert_eq!(screen.vault.store.retrieves(), 0);

    // The window comes to the front, the automatic prompt goes up, and macOS
    // takes it away again, twice.
    screen
        .vault
        .store
        .answering([BiometricError::Interrupted, BiometricError::Interrupted]);
    screen.activate(cx);
    assert_eq!(
        screen.vault.store.retrieves(),
        2,
        "a withdrawn automatic prompt is asked again exactly once"
    );
    screen.vault.state.read_with(cx, |state, _| {
        assert_eq!(*state.biometric_attempt(), BiometricAttempt::Idle);
        assert!(state.unlock_prompt().expect("still locked").error.is_none());
    });

    // With that one retry spent, a withdrawn click brings no other prompt,
    // neither at once with the window in front nor when the window comes
    // back from the background. A click withdrawn while the retry is still
    // to come is the next test.
    assert_eq!(
        screen.withdraw_two_clicks(cx),
        ONE_PROMPT_EACH,
        "with the retry spent, a withdrawn click brought a prompt nobody clicked for"
    );
}

/// A click that macOS withdraws while the automatic prompt's one retry is
/// still to come is left for the user to click again: no other prompt comes
/// up, neither at once with the window in front nor when the window comes
/// back from the background.
#[gpui::test]
fn a_withdrawn_click_is_not_asked_again_on_the_lock_screen(cx: &mut TestAppContext) {
    if !in_a_home_of_its_own("a_withdrawn_click_is_not_asked_again_on_the_lock_screen") {
        return;
    }
    let screen = LockScreen::new(cx, Path::new(NOWHERE));

    // The window comes to the front and the user cancels the automatic
    // prompt. macOS did not withdraw it, so its retry is still to come.
    screen
        .vault
        .store
        .answering([BiometricError::UserCancelled]);
    screen.activate(cx);
    assert_eq!(screen.vault.store.retrieves(), 1);

    assert_eq!(
        screen.withdraw_two_clicks(cx),
        ONE_PROMPT_EACH,
        "with the retry still to come, a withdrawn click brought a prompt nobody clicked for"
    );
    screen.vault.state.read_with(cx, |state, _| {
        assert_eq!(*state.biometric_attempt(), BiometricAttempt::Idle);
        assert!(state.unlock_prompt().expect("still locked").error.is_none());
    });
}

#[gpui::test]
fn an_unlock_completes_without_anyone_holding_a_task(cx: &mut TestAppContext) {
    if !in_a_home_of_its_own("an_unlock_completes_without_anyone_holding_a_task") {
        return;
    }
    let dir = tempfile::tempdir().expect("a folder for the vault");
    let path = vault_file(dir.path());
    let vault = waiting(cx, &path, InMemoryBiometricStore::available());
    let answers = Answers::default();

    let done = noting(&answers, "unlock");
    let state = vault.state.clone();
    // Whatever comes back is dropped at once; nothing else holds the attempt.
    cx.update(|cx| {
        let _ = unlock_from_lock_screen(state, false, done, cx);
    });
    cx.run_until_parked();

    assert_eq!(
        *answers.borrow(),
        [("unlock", Answer::Opened(path.clone()))]
    );
    assert_eq!(open_path(cx, &vault.state), Some(path));
}

#[gpui::test]
fn a_stale_answer_opens_nothing(cx: &mut TestAppContext) {
    if !in_a_home_of_its_own("a_stale_answer_opens_nothing") {
        return;
    }
    let dir = tempfile::tempdir().expect("a folder for the vault");
    let path = vault_file(dir.path());
    let vault = waiting(cx, &path, InMemoryBiometricStore::available());
    let answers = Answers::default();

    // The first prompt is answered with the password, and before that answer
    // is applied a second attempt starts, which makes the first one stale.
    let second_state = vault.state.clone();
    let second_done = noting(&answers, "second");
    let then_start_another = Box::new(move |cx: &mut App| {
        unlock_from_lock_screen(second_state, false, second_done, cx)
            .expect("the second attempt starts");
    });
    let first_done = noting(&answers, "first");
    let state = vault.state.clone();
    cx.update(|cx| {
        run(
            UnlockOrigin::LockScreen,
            state,
            false,
            then_start_another,
            first_done,
            cx,
        )
    });
    cx.run_until_parked();

    // The stale answer ends the attempt it finds in flight, so the second
    // one is stale by the time its answer lands. Nothing opened.
    assert_eq!(
        *answers.borrow(),
        [("first", Answer::Stale), ("second", Answer::Stale)]
    );
    assert_eq!(vault.store.retrieves(), 2);
    assert_eq!(open_path(cx, &vault.state), None);
    vault.state.read_with(cx, |state, _| {
        assert_eq!(state.open_attempt(), 0, "a stale answer started an open");
        assert_eq!(*state.biometric_attempt(), BiometricAttempt::Idle);
        assert_eq!(state.pending_unlock_path(), Some(path.clone()));
    });

    // The next attempt starts fresh and opens the vault, once.
    let third_done = noting(&answers, "third");
    let state = vault.state.clone();
    cx.update(|cx| unlock_from_lock_screen(state, false, third_done, cx));
    cx.run_until_parked();
    assert_eq!(
        answers.borrow().last(),
        Some(&("third", Answer::Opened(path.clone())))
    );
    assert_eq!(open_path(cx, &vault.state), Some(path));
    vault
        .state
        .read_with(cx, |state, _| assert_eq!(state.open_attempt(), 1));
}

#[gpui::test]
fn a_failed_retrieve_reports_the_error(cx: &mut TestAppContext) {
    let vault = waiting(cx, Path::new(NOWHERE), InMemoryBiometricStore::available());
    vault.store.answering([BiometricError::AuthFailed]);
    let answers = Answers::default();

    let done = noting(&answers, "unlock");
    let state = vault.state.clone();
    cx.update(|cx| unlock_from_lock_screen(state, false, done, cx));
    cx.run_until_parked();

    assert_eq!(
        *answers.borrow(),
        [("unlock", Answer::Failed(BiometricError::AuthFailed))]
    );
    vault.state.read_with(cx, |state, _| {
        assert_eq!(
            *state.biometric_attempt(),
            BiometricAttempt::Error {
                path: vault.path.clone(),
                message: BiometricError::AuthFailed.to_string(),
            }
        );
        assert_eq!(state.pending_unlock_path(), Some(vault.path.clone()));
    });
}

#[gpui::test]
fn the_typed_query_never_reaches_last_used(cx: &mut TestAppContext) {
    cx.update(|cx| secure_input::fake::install(cx, FakeApi::new()));
    let mut database = Database::new();
    for title in ["NHQ 100 QAS", "Salesforce"] {
        database
            .root_mut()
            .add_entry()
            .set_unprotected(fields::TITLE, title);
    }
    let snapshot = snapshot_from_database(&database);
    let document = VaultDocument::new(database, snapshot, "pw".to_string(), None);
    let state = cx.new(|_| {
        let mut state = AppState::default();
        state.open_for_test(PathBuf::from(NOWHERE), document);
        state
    });
    let last_used = |cx: &mut TestAppContext| {
        state.read_with(cx, |state, _| match state.vault_status() {
            VaultStatus::Open { last_used, .. } => {
                let mut keys: Vec<String> = last_used.keys().cloned().collect();
                keys.sort();
                keys
            }
            _ => panic!("the vault is open"),
        })
    };

    let mut typed = cx.update(|cx| TypedAhead::begin(cx)).expect("begin");
    for c in "nhq".chars() {
        typed.push(c);
    }
    let query = typed.take_query(std::time::SystemTime::now());
    state.update(cx, |state, cx| state.set_search_query(query.as_str(), cx));
    assert!(
        last_used(cx).is_empty(),
        "a search marked something as recently used"
    );

    let shown = state.read_with(cx, |state, _| match state.vault_status() {
        VaultStatus::Open {
            selected_entry_id, ..
        } => selected_entry_id.clone().expect("the search found NHQ 100"),
        _ => panic!("the vault is open"),
    });
    state.update(cx, |state, _| state.mark_selected_used());
    assert_eq!(last_used(cx), [shown]);
    assert!(!last_used(cx).iter().any(|key| key.contains("nhq")));
}

#[test]
fn the_prompt_names_the_vault_without_its_extension() {
    assert_eq!(
        file_name_for_prompt(Path::new("/Vaults/Nordhafen.kdbx")),
        "Nordhafen"
    );
    assert_eq!(file_name_for_prompt(Path::new("/")), "vault");
}

/// Every trigger there is.
const EVERY_TRIGGER: [PanelTrigger; 8] = [
    PanelTrigger::ReturnKey,
    PanelTrigger::UnlockButtonClick,
    PanelTrigger::Opened,
    PanelTrigger::Typed,
    PanelTrigger::Activated,
    PanelTrigger::Timer,
    PanelTrigger::AfterInterrupted,
    PanelTrigger::AfterExpiry,
];

/// Whether `trigger` is the user asking to unlock. A new trigger does not
/// compile here until it is sorted in, and then belongs in
/// [`EVERY_TRIGGER`].
fn is_a_confirmation(trigger: PanelTrigger) -> bool {
    match trigger {
        PanelTrigger::ReturnKey | PanelTrigger::UnlockButtonClick => true,
        PanelTrigger::Opened
        | PanelTrigger::Typed
        | PanelTrigger::Activated
        | PanelTrigger::Timer
        | PanelTrigger::AfterInterrupted
        | PanelTrigger::AfterExpiry => false,
    }
}

/// The vault on the unlock screen has had its header read, and `cost` is
/// what it found.
fn header_read(cx: &mut TestAppContext, vault: &Vault, cost: Option<KdfCost>) {
    let path = vault.path.clone();
    vault.state.update(cx, |state, cx| {
        let check = state.begin_kdf_check(cx);
        state.finish_kdf_check(check, path, cost, cx);
    });
}

/// A vault the panel could unlock: on the unlock screen, enrolled, the
/// sensor in reach and the header read. Its file does not exist, so Touch ID
/// answers and nothing opens.
fn locked_panel(cx: &mut TestAppContext) -> Vault {
    let vault = waiting(cx, Path::new(NOWHERE), InMemoryBiometricStore::available());
    header_read(cx, &vault, None);
    vault
}

/// Stands in for the panel: whatever happens goes through `confirm`, and only
/// a token reaches `unlock_from_panel`. Says whether an attempt started.
fn panel_sees(cx: &mut TestAppContext, vault: &Vault, trigger: PanelTrigger) -> bool {
    let Some(token) = confirm(trigger) else {
        return false;
    };
    let state = vault.state.clone();
    let started = cx.update(|cx| {
        unlock_from_panel(
            token,
            state,
            false,
            Box::new(|_| {}),
            Box::new(|_, _| {}),
            cx,
        )
    });
    cx.run_until_parked();
    started.is_some()
}

#[test]
fn confirm_is_a_closed_table() {
    for trigger in EVERY_TRIGGER {
        assert_eq!(
            confirm(trigger).is_some(),
            is_a_confirmation(trigger),
            "{trigger:?}"
        );
    }
    assert!(confirm(PanelTrigger::ReturnKey).is_some());
    assert!(confirm(PanelTrigger::UnlockButtonClick).is_some());
}

#[gpui::test]
fn the_panel_starts_touch_id_only_with_a_token(cx: &mut TestAppContext) {
    cx.update(|cx| secure_input::fake::install(cx, FakeApi::new()));
    let vault = locked_panel(cx);

    // The panel opens, the user types, the panel comes back to the front.
    assert!(!panel_sees(cx, &vault, PanelTrigger::Opened));
    let mut typed = cx.update(|cx| TypedAhead::begin(cx)).expect("begin");
    for c in "nhq".chars() {
        typed.push(c);
        assert!(!panel_sees(cx, &vault, PanelTrigger::Typed));
    }
    assert!(!panel_sees(cx, &vault, PanelTrigger::Activated));

    // 31 seconds go by: the buffer expires, timers fire, and a prompt that
    // macOS withdrew would be reported.
    cx.executor().advance_clock(Duration::from_secs(31));
    cx.run_until_parked();
    assert!(typed.expire_if_due(std::time::SystemTime::now() + Duration::from_secs(31)));
    for trigger in [
        PanelTrigger::AfterExpiry,
        PanelTrigger::Timer,
        PanelTrigger::AfterInterrupted,
    ] {
        assert!(!panel_sees(cx, &vault, trigger), "{trigger:?}");
    }
    assert_eq!(
        vault.store.retrieves(),
        0,
        "Touch ID came up without a confirmation"
    );

    assert!(panel_sees(cx, &vault, PanelTrigger::ReturnKey));
    assert_eq!(vault.store.retrieves(), 1);
    assert!(panel_sees(cx, &vault, PanelTrigger::UnlockButtonClick));
    assert_eq!(vault.store.retrieves(), 2);
}

#[gpui::test]
fn an_interrupted_panel_prompt_is_not_retried(cx: &mut TestAppContext) {
    let vault = locked_panel(cx);
    vault.store.answering([BiometricError::Interrupted]);
    let answers = Answers::default();

    let token = confirm(PanelTrigger::ReturnKey).expect("Return confirms");
    let done = noting(&answers, "panel");
    let state = vault.state.clone();
    cx.update(|cx| unlock_from_panel(token, state, false, Box::new(|_| {}), done, cx))
        .expect("the attempt starts");
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();

    assert_eq!(*answers.borrow(), [("panel", Answer::Interrupted)]);
    assert_eq!(
        vault.store.retrieves(),
        1,
        "a withdrawn prompt in the panel was asked again"
    );
    vault.state.read_with(cx, |state, _| {
        assert_eq!(*state.biometric_attempt(), BiometricAttempt::Idle);
        assert_eq!(state.pending_unlock_path(), Some(vault.path.clone()));
        assert!(state.unlock_prompt().expect("still locked").error.is_none());
    });
}

/// The panel's side of the Touch ID dialog. The dialog takes key focus, and
/// macOS does not hand it back, so a Return typed before the panel takes it
/// back goes nowhere.
#[derive(Default)]
struct PanelWindow {
    key: bool,
    returns_taken: usize,
    events: Vec<&'static str>,
}

impl PanelWindow {
    fn press_return(&mut self) {
        if self.key {
            self.returns_taken += 1;
        }
    }
}

/// Whatever the dialog answers, the panel takes key focus back before the
/// answer is applied, and only then takes Return: after the password, a
/// cancel, a failed match, a prompt macOS withdrew, and an answer that comes
/// after the user moved on.
#[gpui::test]
fn the_panel_regains_key_before_it_takes_return(cx: &mut TestAppContext) {
    // (how the prompt ends, what the store answers instead of the password,
    // whether the user moves on before the answer comes, the answer)
    let cases = [
        (
            "the password, the vault stays shut",
            None,
            false,
            Answer::NotOpened,
        ),
        (
            "cancelled",
            Some(BiometricError::UserCancelled),
            false,
            Answer::Failed(BiometricError::UserCancelled),
        ),
        (
            "no match",
            Some(BiometricError::AuthFailed),
            false,
            Answer::Failed(BiometricError::AuthFailed),
        ),
        (
            "withdrawn by macOS",
            Some(BiometricError::Interrupted),
            false,
            Answer::Interrupted,
        ),
        ("the password, too late", None, true, Answer::Stale),
        (
            "cancelled, too late",
            Some(BiometricError::UserCancelled),
            true,
            Answer::Stale,
        ),
    ];
    let mut seen = Vec::new();
    let mut wanted = Vec::new();
    for (case, error, moved_on, answer) in cases {
        let vault = locked_panel(cx);
        vault.store.answering(error);
        let panel = Rc::new(RefCell::new(PanelWindow {
            key: true,
            ..PanelWindow::default()
        }));
        // The attempt as it stands just before the answer comes. If
        // `on_answered` finds it changed, the answer was applied first.
        let before = Rc::new(RefCell::new(BiometricAttempt::Idle));
        let answered = Rc::new(RefCell::new(None));

        let token = confirm(PanelTrigger::ReturnKey).expect("Return confirms");
        // The dialog goes up and takes key focus.
        panel.borrow_mut().key = false;
        let on_answered: OnAnswered = {
            let panel = panel.clone();
            let before = before.clone();
            let state = vault.state.clone();
            Box::new(move |cx: &mut App| {
                let applied = *state.read(cx).biometric_attempt() != *before.borrow();
                let mut panel = panel.borrow_mut();
                panel.key = true;
                panel.events.push(if applied {
                    "answered after the answer was applied"
                } else {
                    "answered"
                });
            })
        };
        let on_done: OnDone = {
            let panel = panel.clone();
            let answered = answered.clone();
            Box::new(move |answer, _| {
                let mut panel = panel.borrow_mut();
                let event = if panel.key {
                    "done"
                } else {
                    "done while not key"
                };
                panel.events.push(event);
                *answered.borrow_mut() = Some(answer);
            })
        };
        let state = vault.state.clone();
        cx.update(|cx| unlock_from_panel(token, state, false, on_answered, on_done, cx))
            .expect("the attempt starts");
        if moved_on {
            vault
                .state
                .update(cx, |state, _| state.clear_biometric_attempt_public());
        }
        *before.borrow_mut() = vault
            .state
            .read_with(cx, |state, _| state.biometric_attempt().clone());

        // Return while the dialog is up, and again once it has answered.
        panel.borrow_mut().press_return();
        cx.run_until_parked();
        panel.borrow_mut().press_return();

        let panel = panel.borrow();
        let answered = answered.borrow_mut().take();
        seen.push((case, panel.events.clone(), panel.returns_taken, answered));
        wanted.push((case, vec!["answered", "done"], 1_usize, Some(answer)));
    }
    assert_eq!(seen, wanted);
}

/// `on_answered` runs as soon as macOS has answered, before the vault is
/// opened: the open can take seconds with an expensive key derivation, and
/// the panel has to have key focus back before then. Here `on_answered`
/// writes the vault file that the open reads, so the vault opens only if
/// `on_answered` ran first. The flow runs once through the panel's way in
/// and once with the lock screen's origin. The lock screen's own entry hands
/// the flow an `on_answered` that does nothing, so that case calls the flow
/// directly.
#[gpui::test]
fn on_answered_runs_before_the_vault_is_opened(cx: &mut TestAppContext) {
    if !in_a_home_of_its_own("on_answered_runs_before_the_vault_is_opened") {
        return;
    }
    let mut seen = Vec::new();
    let mut wanted = Vec::new();
    for origin in [UnlockOrigin::Panel, UnlockOrigin::LockScreen] {
        let dir = tempfile::tempdir().expect("a folder for the vault");
        let path = dir.path().join("nordhafen.kdbx");
        let vault = waiting(cx, &path, InMemoryBiometricStore::available());
        header_read(cx, &vault, None);
        let answers = Answers::default();

        let folder = dir.path().to_path_buf();
        let on_answered: OnAnswered = Box::new(move |_| {
            vault_file(&folder);
        });
        let done = noting(&answers, "unlock");
        let state = vault.state.clone();
        cx.update(|cx| match origin {
            UnlockOrigin::Panel => {
                let token = confirm(PanelTrigger::ReturnKey).expect("Return confirms");
                unlock_from_panel(token, state, false, on_answered, done, cx)
            }
            UnlockOrigin::LockScreen => run(origin, state, false, on_answered, done, cx),
        })
        .expect("the attempt starts");
        cx.run_until_parked();

        let answers = answers.borrow().clone();
        seen.push((origin, answers, open_path(cx, &vault.state)));
        wanted.push((
            origin,
            vec![("unlock", Answer::Opened(path.clone()))],
            Some(path),
        ));
    }
    assert_eq!(
        seen, wanted,
        "the vault was opened before on_answered had run"
    );
}

#[gpui::test]
fn the_panel_never_answers_a_cost_question(cx: &mut TestAppContext) {
    let start = |cx: &mut TestAppContext, vault: &Vault| {
        let token = confirm(PanelTrigger::ReturnKey).expect("Return confirms");
        let state = vault.state.clone();
        let started = cx.update(|cx| {
            unlock_from_panel(
                token,
                state,
                false,
                Box::new(|_| {}),
                Box::new(|_, _| {}),
                cx,
            )
        });
        cx.run_until_parked();
        started.is_some()
    };

    let not_read = waiting(cx, Path::new(NOWHERE), InMemoryBiometricStore::available());
    assert!(!start(cx, &not_read), "started before the header was read");
    assert_eq!(not_read.store.retrieves(), 0);

    let costly = waiting(cx, Path::new(NOWHERE), InMemoryBiometricStore::available());
    header_read(
        cx,
        &costly,
        Some(KdfCost {
            variant: Argon2Variant::Argon2id,
            memory: 2 * 1024 * 1024 * 1024,
            iterations: 3,
            parallelism: 4,
        }),
    );
    assert!(!start(cx, &costly), "started with a cost question open");
    assert_eq!(costly.store.retrieves(), 0);

    header_read(cx, &costly, None);
    assert!(start(cx, &costly));
    assert_eq!(costly.store.retrieves(), 1);
}

/// The panel's prompt takes the Mac password only when `unlock_from_panel`
/// is told to allow it: first not, then yes.
#[gpui::test]
fn the_panel_prompt_takes_the_mac_password_only_when_allowed(cx: &mut TestAppContext) {
    let vault = locked_panel(cx);
    for allowed in [false, true] {
        let token = confirm(PanelTrigger::ReturnKey).expect("Return confirms");
        let state = vault.state.clone();
        cx.update(|cx| {
            unlock_from_panel(
                token,
                state,
                allowed,
                Box::new(|_| {}),
                Box::new(|_, _| {}),
                cx,
            )
        })
        .expect("the attempt starts");
        cx.run_until_parked();
    }

    let asked: Vec<bool> = vault
        .store
        .inner
        .retrieve_calls()
        .iter()
        .map(|(_, options)| options.allow_device_passcode)
        .collect();
    assert_eq!(
        asked,
        [false, true],
        "the panel's prompt did not take the Mac password as it was told, not and then yes"
    );
}

/// An `on_answered` that sets `flag`.
fn flagging(flag: &Rc<Cell<bool>>) -> OnAnswered {
    let flag = flag.clone();
    Box::new(move |_| flag.set(true))
}

/// Runs the executor one task at a time until `answered` is set. The step
/// that sets it is the one that took macOS's answer; an open it spawned has
/// not run yet when this returns.
fn tick_until_answered(cx: &mut TestAppContext, answered: &Cell<bool>) {
    while !answered.get() {
        assert!(cx.executor().tick(), "the prompt was never answered");
    }
}

/// A vault file on the unlock screen, enrolled, the sensor in reach and the
/// header read: the panel can unlock it, and Touch ID's password opens it.
fn openable(cx: &mut TestAppContext, dir: &Path) -> Vault {
    let vault = waiting(cx, &vault_file(dir), InMemoryBiometricStore::available());
    header_read(cx, &vault, None);
    vault
}

/// Starts the panel's attempt with Return. `on_answered` sets `answered`.
fn panel_unlocks(
    cx: &mut TestAppContext,
    vault: &Vault,
    answered: &Rc<Cell<bool>>,
    answers: &Answers,
) {
    let token = confirm(PanelTrigger::ReturnKey).expect("Return confirms");
    let state = vault.state.clone();
    let (on_answered, on_done) = (flagging(answered), noting(answers, "unlock"));
    cx.update(|cx| unlock_from_panel(token, state, false, on_answered, on_done, cx))
        .expect("the attempt starts");
}

/// While an attempt is under way, from the prompt until its vault is open,
/// neither Return nor the unlock button starts another one, and `kind` tells
/// the panel so. A second prompt would make the first attempt stale; the
/// first answer would then end the second attempt too, and the vault would
/// stay locked after two prompts answered with a finger. The open is held by
/// running the executor one task at a time: the step that takes macOS's
/// answer spawns the open, and the test confirms again before it runs.
#[gpui::test]
fn the_panel_starts_no_second_prompt_while_one_is_under_way(cx: &mut TestAppContext) {
    if !in_a_home_of_its_own("the_panel_starts_no_second_prompt_while_one_is_under_way") {
        return;
    }
    let mut seen = Vec::new();
    let mut wanted = Vec::new();
    for when in ["while the prompt is up", "while the vault opens"] {
        let dir = tempfile::tempdir().expect("a folder for the vault");
        let vault = openable(cx, dir.path());
        let answers = Answers::default();
        let answered = Rc::new(Cell::new(false));

        panel_unlocks(cx, &vault, &answered, &answers);
        if when == "while the vault opens" {
            tick_until_answered(cx, &answered);
        }
        let open_before = open_path(cx, &vault.state);
        let kind = vault
            .state
            .read_with(cx, |state, _| locked_panel::kind(state, false));
        let mut again = Vec::new();
        for trigger in [PanelTrigger::ReturnKey, PanelTrigger::UnlockButtonClick] {
            let token = confirm(trigger).expect("confirms");
            let state = vault.state.clone();
            let on_done = noting(&answers, "again");
            let started = cx
                .update(|cx| unlock_from_panel(token, state, false, Box::new(|_| {}), on_done, cx));
            again.push((trigger, started.is_some()));
        }
        cx.run_until_parked();

        seen.push((
            when,
            open_before,
            kind,
            again,
            vault.store.retrieves(),
            answers.borrow().clone(),
            open_path(cx, &vault.state),
        ));
        wanted.push((
            when,
            None,
            Kind::Unlocking,
            vec![
                (PanelTrigger::ReturnKey, false),
                (PanelTrigger::UnlockButtonClick, false),
            ],
            1,
            vec![("unlock", Answer::Opened(vault.path.clone()))],
            Some(vault.path.clone()),
        ));
    }
    assert_eq!(
        seen, wanted,
        "a second prompt went up while the first attempt was under way"
    );
}

/// How the user ends an attempt before its answer is applied.
#[derive(Clone, Copy, Debug, PartialEq)]
enum MoveOn {
    /// The vault is locked: the auto-lock, or the lock command.
    Lock,
    /// Another vault is asked for and waits on the unlock screen.
    AnotherVault,
}

impl MoveOn {
    fn other(vault: &Vault) -> PathBuf {
        vault.path.with_file_name("elbe.kdbx")
    }

    fn now(self, cx: &mut TestAppContext, vault: &Vault) {
        let other = Self::other(vault);
        vault.state.update(cx, |state, cx| match self {
            MoveOn::Lock => assert!(state.lock_vault(cx), "the lock waited for a save"),
            MoveOn::AnotherVault => state.request_password(other, cx),
        });
    }

    /// The vault on the unlock screen after that.
    fn waiting_after(self, vault: &Vault) -> Option<PathBuf> {
        match self {
            MoveOn::Lock => None,
            MoveOn::AnotherVault => Some(Self::other(vault)),
        }
    }
}

/// The open attempts so far and the vault on the unlock screen.
fn unlock_screen(cx: &mut TestAppContext, vault: &Vault) -> (u64, Option<PathBuf>) {
    vault.state.read_with(cx, |state, _| {
        (state.open_attempt(), state.pending_unlock_path())
    })
}

/// An attempt the user ended while the prompt was up, by locking the vault
/// or asking for another one, gets its answer as stale in the step that
/// takes it: the password never goes on to the open. Had it gone on,
/// `on_done` would come only after the open, in a later step.
#[gpui::test]
fn a_stale_password_never_goes_on_to_the_open(cx: &mut TestAppContext) {
    if !in_a_home_of_its_own("a_stale_password_never_goes_on_to_the_open") {
        return;
    }
    let mut seen = Vec::new();
    let mut wanted = Vec::new();
    for how in [MoveOn::Lock, MoveOn::AnotherVault] {
        let dir = tempfile::tempdir().expect("a folder for the vault");
        let vault = openable(cx, dir.path());
        let answers = Answers::default();
        let answered = Rc::new(Cell::new(false));

        panel_unlocks(cx, &vault, &answered, &answers);
        // The user moves on while the prompt is up, and then answers it.
        how.now(cx, &vault);
        tick_until_answered(cx, &answered);
        let in_that_step = answers.borrow().clone();
        cx.run_until_parked();

        seen.push((
            how,
            in_that_step,
            answers.borrow().clone(),
            open_path(cx, &vault.state),
            unlock_screen(cx, &vault),
        ));
        wanted.push((
            how,
            vec![("unlock", Answer::Stale)],
            vec![("unlock", Answer::Stale)],
            None,
            (0, how.waiting_after(&vault)),
        ));
    }
    assert_eq!(seen, wanted, "a stale password went on to the open");
}

/// An attempt the user ended while its vault was being opened, by locking
/// the vault or asking for another one, opens nothing once the open is done.
#[gpui::test]
fn an_attempt_ended_while_the_vault_opens_opens_nothing(cx: &mut TestAppContext) {
    if !in_a_home_of_its_own("an_attempt_ended_while_the_vault_opens_opens_nothing") {
        return;
    }
    let mut seen = Vec::new();
    let mut wanted = Vec::new();
    for how in [MoveOn::Lock, MoveOn::AnotherVault] {
        let dir = tempfile::tempdir().expect("a folder for the vault");
        let vault = openable(cx, dir.path());
        let answers = Answers::default();
        let answered = Rc::new(Cell::new(false));

        panel_unlocks(cx, &vault, &answered, &answers);
        // macOS answers with the password and the open is spawned. The user
        // moves on before it is done.
        tick_until_answered(cx, &answered);
        let held = answers.borrow().is_empty();
        how.now(cx, &vault);
        cx.run_until_parked();

        seen.push((
            how,
            held,
            answers.borrow().clone(),
            open_path(cx, &vault.state),
            unlock_screen(cx, &vault),
        ));
        wanted.push((
            how,
            true,
            vec![("unlock", Answer::Stale)],
            None,
            (0, how.waiting_after(&vault)),
        ));
    }
    assert_eq!(seen, wanted, "an attempt the user ended opened its vault");
}

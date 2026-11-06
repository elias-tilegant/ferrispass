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
use crate::app::unlock::typed_ahead::TypedAhead;
use crate::biometric::memory::InMemoryBiometricStore;
use crate::biometric::registry::{BiometricEnrollment, BiometricRegistry};
use crate::biometric::{BiometricResult, BiometricStore, EnrollmentId};
use crate::clipboard::{AccessBehavior, fake::FakePasteboard};
use crate::keepass::VaultDocument;
use crate::keepass::repository::snapshot_from_database;
use crate::platform::secure_input::{self, fake::FakeApi};
use crate::ui::AppShell;
use gpui::{AnyWindowHandle, TestAppContext, Window, WindowHandle};
use gpui_component::Root;
use keepass::config::{DatabaseConfig, KdfConfig};
use keepass::db::fields;
use keepass::{Database, DatabaseKey};
use std::cell::RefCell;
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
}

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
    assert_eq!(
        screen.vault.store.inner.retrieve_calls()[0]
            .1
            .allow_device_passcode,
        AppSettings::default().biometric_allow_passcode_fallback,
        "the Mac password setting did not reach the prompt"
    );
    assert_eq!(open_path(cx, &screen.vault.state), Some(path));
    screen.vault.state.read_with(cx, |state, _| {
        assert_eq!(*state.biometric_attempt(), BiometricAttempt::Idle);
    });
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

    // A withdrawn click is left for the user to click again.
    screen.vault.store.answering([BiometricError::Interrupted]);
    screen.in_window(cx, |shell, window, cx| {
        shell.submit_biometric_unlock(window, cx)
    });
    cx.run_until_parked();
    assert_eq!(screen.vault.store.retrieves(), 3);
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
    let query = typed.take_query();
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

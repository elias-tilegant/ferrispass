//! The locked panel as the app opens it: the main window on the unlock screen
//! of a vault enrolled for Touch ID, then the panel as a second window. Secure
//! event input is a fake that counts its calls, the store is the in-memory
//! one, and "another app" is a window of its own that the test brings to the
//! front.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::{AnyWindowHandle, AppContext as _, Entity, Keystroke, Role, TestAppContext};
use keepass::config::{DatabaseConfig, KdfConfig};
use keepass::db::fields;
use keepass::{Database, DatabaseKey};

use super::*;
use crate::app::actions;
use crate::app::services::clipboard::ClipboardService;
use crate::biometric::memory::InMemoryBiometricStore;
use crate::biometric::registry::{BiometricEnrollment, BiometricRegistry};
use crate::biometric::{BiometricError, BiometricStore, EnrollmentId};
use crate::clipboard::{AccessBehavior, fake::FakePasteboard};
use crate::platform::secure_input::fake::{self as secure, Fake, FakeApi};
use crate::platform::secure_input::{HoldError, OsStatus};
use crate::ui::ds::components::probe;
use crate::ui::vm::panel::NAME_MAX;
use crate::ui::windows::panel;

/// The master password of every vault here.
const PASSWORD: &str = "correct horse battery staple";

/// What the tests type, and look for in every node.
const SENTINEL: &str = "sp-typed-7Qx";

thread_local! {
    /// How far the tests moved the wall clock forward, on this thread.
    static AHEAD: Cell<Duration> = const { Cell::new(Duration::ZERO) };
    /// How often the panel took key status back, on this thread.
    static KEY_TAKEN: Cell<usize> = const { Cell::new(0) };
}

/// Called where the panel makes itself the key window again.
pub(super) fn note_key_taken() {
    KEY_TAKEN.with(|taken| taken.set(taken.get() + 1));
}

fn key_taken() -> usize {
    KEY_TAKEN.with(Cell::get)
}

/// How far the tests moved the panel's wall clock forward.
pub(super) fn wall_clock_ahead() -> Duration {
    AHEAD.with(Cell::get)
}

fn move_wall_clock(by: Duration) {
    AHEAD.with(|ahead| ahead.set(ahead.get() + by));
}

/// Puts the panel's wall clock back to the system's. A buffer begun after
/// the clock moved starts its 30 seconds at the system's time, so a test
/// that types again first puts the clock back.
fn reset_wall_clock() {
    AHEAD.with(|ahead| ahead.set(Duration::ZERO));
}

/// What waits for the panel when it opens.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Setup {
    /// A vault on the unlock screen, enrolled, the sensor in reach.
    TouchId,
    /// No vault at all.
    NothingYet,
    /// A vault on the unlock screen whose file is gone.
    Missing,
}

/// The app locked, with the main window open and the panel closed.
struct Locked {
    state: Entity<AppState>,
    store: Arc<InMemoryBiometricStore>,
    secure: Fake,
    path: PathBuf,
    /// A window that stands for another app.
    other: AnyWindowHandle,
    _dir: tempfile::TempDir,
}

/// A vault file in `dir` that opens with [`PASSWORD`], holding `titles`.
/// AES-KDF with few rounds keeps the open quick.
fn vault_file(dir: &Path, titles: &[&str]) -> PathBuf {
    let path = dir.join("team-beratung.kdbx");
    let mut database = Database::with_config(DatabaseConfig {
        kdf_config: KdfConfig::Aes { rounds: 1_000 },
        ..DatabaseConfig::default()
    });
    for title in titles {
        database
            .root_mut()
            .add_entry()
            .set_unprotected(fields::TITLE, *title);
    }
    let mut file = std::fs::File::create(&path).expect("create the vault file");
    database
        .save(&mut file, DatabaseKey::new().with_password(PASSWORD))
        .expect("write the vault file");
    path
}

fn locked_app(cx: &mut TestAppContext, setup: Setup) -> Locked {
    locked_app_with(cx, setup, FakeApi::new(), &["NHQ 100", "Salesforce"])
}

/// The app locked as `setup` says, secure input answered by `api`, the vault
/// holding `titles`.
fn locked_app_with(cx: &mut TestAppContext, setup: Setup, api: FakeApi, titles: &[&str]) -> Locked {
    AHEAD.with(|ahead| ahead.set(Duration::ZERO));
    KEY_TAKEN.with(|taken| taken.set(0));
    crate::autotype::hotkey::fake::reset();
    let secure = cx.update(|cx| {
        gpui_component::init(cx);
        actions::init(cx);
        ClipboardService::install_for_test(
            cx,
            Box::new(FakePasteboard::new(AccessBehavior::Default)),
        );
        secure::install(cx, api)
    });
    let dir = tempfile::tempdir().expect("a folder for the vault");
    let path = match setup {
        Setup::Missing => dir.path().join("gone").join("team-beratung.kdbx"),
        Setup::TouchId | Setup::NothingYet => vault_file(dir.path(), titles),
    };
    let store = Arc::new(InMemoryBiometricStore::available());
    let id = EnrollmentId::new_random();
    store.enroll(&id, PASSWORD).expect("enrol");
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
        if setup != Setup::NothingYet {
            state.request_password(waiting, cx);
        }
        state
    });
    // The main window reads the vault's header, as it does at launch.
    cx.update(|cx| crate::app::open_main_window_for_test(cx, state.clone()));
    cx.run_until_parked();
    let other = cx.add_window(|_, _| gpui::EmptyView).into();
    Locked {
        state,
        store,
        secure,
        path,
        other,
        _dir: dir,
    }
}

impl Locked {
    /// Opens the panel as the shortcut does. It is not the key window yet.
    fn open_panel(&self, cx: &mut TestAppContext) -> AnyWindowHandle {
        cx.update(panel::open);
        cx.run_until_parked();
        cx.update(|cx| panel::for_test(cx).0)
            .expect("the panel opened")
    }

    fn enables(&self) -> usize {
        self.secure.api.enables()
    }

    fn disables(&self) -> usize {
        self.secure.api.disables()
    }

    /// How often the Touch ID prompt was shown.
    fn retrieves(&self) -> usize {
        self.store.retrieve_calls().len()
    }

    /// The store gives `password` for the vault from now on.
    fn enrolled_password(&self, cx: &mut TestAppContext, password: &str) {
        let id = self.state.read_with(cx, |state, _| {
            state
                .biometric_registry()
                .get(&self.path)
                .expect("the vault is enrolled")
                .id
                .clone()
        });
        self.store.enroll(&id, password).expect("enrol");
    }

    /// The vault's state on the unlock screen, as the panel reads it.
    fn vault_is_open(&self, cx: &mut TestAppContext) -> bool {
        self.state.read_with(cx, |state, _| {
            matches!(state.vault_status(), VaultStatus::Open { .. })
        })
    }
}

/// Return, as the keyboard sends it, without running what it starts: the
/// prompt is up until the test lets the executor run.
fn press_return(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.dispatch_keystroke(window, Keystroke::parse("enter").expect("a keystroke"));
}

fn is_key(cx: &mut TestAppContext, window: AnyWindowHandle) -> bool {
    window
        .update(cx, |_, window, _| window.is_window_active())
        .expect("the window")
}

/// Return in the panel, and the Touch ID dialog taking key status from the
/// panel right after it, as on a Mac. The window of the other app stands
/// for the dialog: it becomes the key window once Return's handler is done,
/// and the panel's activation observer hears of it before macOS answers.
/// Returns while the prompt is up.
fn return_with_the_dialog(cx: &mut TestAppContext, app: &Locked, panel: AnyWindowHandle) {
    app.other
        .update(cx, |_, window, _| window.activate_window())
        .expect("the window for the dialog");
    press_return(cx, panel);
    while is_key(cx, panel) {
        assert!(cx.executor().tick(), "the dialog never took key status");
    }
    assert_eq!(key_taken(), 0, "macOS answered before the dialog was up");
}

/// A click on the unlock button, as its handler gets it.
fn click_unlock(cx: &mut TestAppContext, panel: AnyWindowHandle) {
    panel
        .update(cx, |_, window, cx| {
            let view = panel::view_for_test(cx).expect("the panel's view");
            view.update(cx, |view, cx| view.on_unlock_click(window, cx));
        })
        .expect("the panel");
    cx.run_until_parked();
}

/// What the panel searches for now.
fn search(cx: &mut TestAppContext) -> String {
    let vm = cx.update(|cx| panel::for_test(cx).1).expect("the model");
    vm.read_with(cx, |vm, _| vm.query().to_string())
}

/// What the panel's search field holds.
fn search_field(cx: &mut TestAppContext) -> String {
    view(cx).read_with(cx, |view, cx| view.search.read(cx).value().to_string())
}

/// Runs the calling test again in a child process of this test binary
/// whose home and temporary folder are a new, empty directory, and returns
/// `false` once that run has passed. In the child it returns `true`, and
/// the test goes on there. Opening a vault writes the list of recent vaults
/// under the home folder, so every test that opens one runs in a child.
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

fn view(cx: &mut TestAppContext) -> Entity<PanelView> {
    cx.update(|cx| panel::view_for_test(cx))
        .expect("the panel's view")
}

fn dots(cx: &mut TestAppContext) -> usize {
    view(cx).read_with(cx, |view, _| view.locked.dots_for_test())
}

fn has_buffer(cx: &mut TestAppContext) -> bool {
    view(cx).read_with(cx, |view, _| view.locked.has_buffer())
}

fn kind(cx: &mut TestAppContext) -> Kind {
    view(cx).read_with(cx, |view, _| view.locked.kind_for_test())
}

fn panel_is_open(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| panel::is_open(cx))
}

/// Brings `window` to the front, as a click on it does.
fn activate(cx: &mut TestAppContext, window: AnyWindowHandle) {
    window
        .update(cx, |_, window, _| window.activate_window())
        .expect("a window to activate");
    cx.run_until_parked();
}

fn type_into(cx: &mut TestAppContext, window: AnyWindowHandle, text: &str) {
    cx.simulate_input(window, text);
    cx.run_until_parked();
}

fn keys(cx: &mut TestAppContext, window: AnyWindowHandle, keystrokes: &str) {
    cx.simulate_keystrokes(window, keystrokes);
    cx.run_until_parked();
}

/// One tick of the locked panel's clock.
fn tick(cx: &mut TestAppContext) {
    cx.executor().advance_clock(TICK);
    cx.run_until_parked();
}

/// The parts `build` makes for `look`, read one by one as assistive
/// technology gets them. For the cases a test sets up by hand; the open
/// panel is read with `drawn_parts`.
fn parts_of(cx: &mut TestAppContext, look: Look) -> Vec<probe::ComponentPart> {
    probe::in_frame(cx, move |window, cx| {
        let focus = cx.focus_handle();
        // Every element by name, so one added to `Built` does not compile
        // until it is read here.
        // A design system button writes its node only once the design system
        // has turned it into GPUI's; its name is `button_name`, which the
        // tests read instead. The footer is read below, part by part.
        let Built {
            field,
            dots,
            sentence,
            button: _,
            footer: _,
        } = build(&look, &focus);
        let mut parts = vec![
            probe::part("field", field),
            probe::part("dots", dots),
            probe::part("sentence", sentence),
        ];
        parts.extend(crate::ui::ds::components::PanelFooter::parts(
            || build(&look, &focus).footer,
            window,
            cx,
        ));
        parts
    })
}

/// The parts of the open locked panel as its window draws them: what
/// `locked_parts` builds for the panel's own view as it is now, handlers
/// and text style included, read one by one as assistive technology gets
/// them. The button is not among them: only the design system turns it
/// into a node, so the tests read it by `button_name`.
fn drawn_parts(cx: &mut TestAppContext) -> Vec<probe::ComponentPart> {
    let view = view(cx);
    probe::in_frame(cx, move |window, cx| {
        // Every element by name, as in `parts_of`.
        let Built {
            field,
            dots,
            sentence,
            button: _,
            footer,
        } = view.update(cx, |view, cx| view.locked_parts(cx));
        // The footer's parts are read from two footers, one for the node
        // it writes and one for the elements inside it.
        let again = view.update(cx, |view, cx| view.locked_parts(cx)).footer;
        let footers = RefCell::new(vec![again, footer]);
        let mut parts = vec![
            probe::part("field", field),
            probe::part("dots", dots),
            probe::part("sentence", sentence),
        ];
        parts.extend(crate::ui::ds::components::PanelFooter::parts(
            || footers.borrow_mut().pop().expect("a footer for each read"),
            window,
            cx,
        ));
        parts
    })
}

/// Where `parts` hand `secret`, or two of its characters side by side, to
/// assistive technology more often than `quiet`, the same parts with nothing
/// typed: the part, the property and the text.
fn leaks(
    parts: &[probe::ComponentPart],
    quiet: &[probe::ComponentPart],
    secret: &str,
) -> Vec<String> {
    let chars: Vec<char> = secret.chars().collect();
    let mut pieces = vec![secret.to_string()];
    pieces.extend(chars.windows(2).map(|pair| pair.iter().collect::<String>()));
    let mut found = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let before = parts[..index]
            .iter()
            .filter(|other| other.name == part.name)
            .count();
        let same = quiet
            .iter()
            .filter(|other| other.name == part.name)
            .nth(before);
        for (property, said) in part.texts() {
            let Some(said) = said else {
                continue;
            };
            let plain = same
                .and_then(|same| same.texts().into_iter().find(|(name, _)| *name == property))
                .and_then(|(_, text)| text)
                .unwrap_or_default();
            for piece in &pieces {
                if said.matches(piece.as_str()).count() > plain.matches(piece.as_str()).count() {
                    found.push(format!(
                        "the {property} of the {} holds {piece:?}: {said:?}",
                        part.name
                    ));
                }
            }
        }
    }
    found
}

// ------------------------------------------------------- secure input --

/// Opened, the panel is not the key window yet: another app is still in
/// front, and the panel opens with `focus: true` all the same. Nothing turns
/// secure input on, and keys typed then go nowhere.
#[gpui::test]
fn an_open_but_inactive_locked_panel_holds_no_secure_input(cx: &mut TestAppContext) {
    let app = locked_app(cx, Setup::TouchId);
    let panel = app.open_panel(cx);
    let active = panel
        .update(cx, |_, window, _| window.is_window_active())
        .expect("the panel");
    assert!(!active, "the test needs a panel that is not key");
    assert_eq!(
        kind(cx),
        Kind::TouchId {
            device_password: false
        }
    );
    assert_eq!(app.enables(), 0, "secure input before the panel was key");
    assert!(!has_buffer(cx));

    type_into(cx, panel, "nh");
    assert_eq!(app.enables(), 0);
    assert_eq!(dots(cx), 0, "keys went in without secure input");
    assert!(!has_buffer(cx));
    assert!(
        app.store.retrieve_calls().is_empty(),
        "opening asked Touch ID"
    );
    assert!(
        app.state
            .read_with(cx, |state, _| state.pending_unlock_path().is_some()),
        "the vault still waits on the unlock screen"
    );
}

/// The panel becoming key turns secure input on once, before any key.
/// Another activation without a wipe in between takes no second Enable, and
/// the keys after it go into the same buffer.
#[gpui::test]
fn activation_takes_secure_input_once_before_the_first_key(cx: &mut TestAppContext) {
    let app = locked_app(cx, Setup::TouchId);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    assert_eq!(app.enables(), 1, "secure input as the panel became key");
    assert!(has_buffer(cx));
    assert_eq!(dots(cx), 0, "before the first key");

    // The observer reports the panel key once more.
    panel
        .update(cx, |_, window, cx| {
            let view = panel::view_for_test(cx).expect("the panel's view");
            view.update(cx, |view, cx| view.on_activation(true, window, cx));
        })
        .expect("the panel");
    cx.run_until_parked();
    type_into(cx, panel, "nhq");
    assert_eq!(
        app.enables(),
        1,
        "a second activation took secure input again"
    );
    assert_eq!(app.disables(), 0);
    assert_eq!(dots(cx), 3);
}

/// When the buffer is wiped while the panel stays key and locked, the next
/// one begins at once, under secure input: after the 30 seconds and after
/// Escape. After closing, and after losing the keyboard to another app, no
/// new one begins.
#[gpui::test]
fn a_wiped_buffer_is_replaced_while_the_panel_is_key_and_locked(cx: &mut TestAppContext) {
    if !in_a_home_of_its_own("a_wiped_buffer_is_replaced_while_the_panel_is_key_and_locked") {
        return;
    }
    let app = locked_app(cx, Setup::TouchId);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    type_into(cx, panel, "nhq");

    move_wall_clock(Duration::from_secs(31));
    tick(cx);
    reset_wall_clock();
    assert_eq!((app.enables(), app.disables()), (2, 1), "expired");
    assert!(has_buffer(cx), "no new buffer after it expired");
    assert_eq!(dots(cx), 0);
    type_into(cx, panel, "sales");
    assert_eq!(dots(cx), 5, "the new buffer takes keys");

    keys(cx, panel, "escape");
    assert!(panel_is_open(cx), "the first Escape only wipes");
    assert_eq!((app.enables(), app.disables()), (3, 2), "Escape");
    assert!(has_buffer(cx), "no new buffer after Escape");
    assert_eq!(dots(cx), 0);

    cx.update(panel::close);
    cx.run_until_parked();
    assert_eq!((app.enables(), app.disables()), (3, 3), "closed");

    let panel = app.open_panel(cx);
    activate(cx, panel);
    assert_eq!((app.enables(), app.disables()), (4, 3));
    activate(cx, app.other);
    assert!(!panel_is_open(cx));
    assert_eq!(
        (app.enables(), app.disables()),
        (4, 4),
        "losing the keyboard to another app began a new buffer"
    );

    // macOS refuses secure input when the panel takes it back after Touch
    // ID: the text is wiped, and as soon as the panel is key again a new
    // buffer begins under secure input, which macOS gives again, while the
    // vault still opens. The store's password does not open the vault, so
    // the panel stays locked afterwards.
    let panel = app.open_panel(cx);
    activate(cx, panel);
    type_into(cx, panel, "nhq");
    app.enrolled_password(cx, "not the password");
    return_with_the_dialog(cx, &app, panel);
    let refusing =
        cx.update(|cx| secure::install(cx, FakeApi::refusing(HoldError::Os(OsStatus(-50)))));
    while key_taken() == 0 {
        assert!(cx.executor().tick(), "macOS never answered");
    }
    assert_eq!(
        dots(cx),
        0,
        "the text stayed although secure input was refused"
    );
    assert!(!has_buffer(cx));
    assert!(!refusing.held());
    let again = cx.update(|cx| secure::install(cx, FakeApi::new()));
    while !has_buffer(cx) {
        assert!(
            cx.executor().tick(),
            "no new buffer after the refused rearm"
        );
    }
    let under_way = view(cx).read_with(cx, |view, _| view.locked.attempt_for_test().is_some());
    assert!(under_way, "the new buffer waited for the attempt to end");
    assert_eq!(again.api.enables(), 1);
    assert!(again.held());
    assert_eq!(dots(cx), 0);
    cx.run_until_parked();
    assert!(
        !app.vault_is_open(cx),
        "the test needs the vault to stay locked"
    );
    assert!(has_buffer(cx));
    assert_eq!(again.api.enables(), 1, "a second buffer after the attempt");
}

/// macOS refuses secure input: there is no buffer, nothing typed goes in,
/// and the panel sends you to the main window.
#[gpui::test]
fn a_refused_secure_input_sends_to_the_main_window(cx: &mut TestAppContext) {
    let app = locked_app_with(
        cx,
        Setup::TouchId,
        FakeApi::refusing(HoldError::Os(OsStatus(-50))),
        &["NHQ 100"],
    );
    let panel = app.open_panel(cx);
    activate(cx, panel);
    assert_eq!(kind(cx), Kind::NeedsMainWindow);
    assert!(!has_buffer(cx));
    type_into(cx, panel, "nhq");
    assert_eq!(dots(cx), 0, "keys went in without secure input");
    assert!(!app.secure.held());
}

// ---------------------------------------------- the buffer and its clock --

/// The tick wipes the buffer once its 30 seconds are up, and secure input
/// goes with it.
#[gpui::test]
fn the_tick_calls_expire_if_due(cx: &mut TestAppContext) {
    let app = locked_app(cx, Setup::TouchId);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    type_into(cx, panel, "nhq");
    move_wall_clock(Duration::from_secs(29));
    tick(cx);
    assert_eq!(dots(cx), 3, "wiped before the 30 seconds were up");
    assert_eq!(app.disables(), 0);

    move_wall_clock(Duration::from_secs(1));
    tick(cx);
    assert_eq!(dots(cx), 0, "the tick left the text after 30 seconds");
    assert_eq!(app.disables(), 1, "secure input stayed with the old buffer");
}

/// A key wipes the buffer too once its 30 seconds are up, before it does
/// anything, without a tick first: Backspace and a character each find the
/// old text gone with its secure input, and a new buffer under secure input
/// of its own, which the character is the first key of.
#[gpui::test]
fn a_key_calls_expire_if_due(cx: &mut TestAppContext) {
    let app = locked_app(cx, Setup::TouchId);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    // GPUI's clock stays where it is from here on, so no tick runs.
    for (key, left) in [("backspace", 0), ("s", 1)] {
        type_into(cx, panel, "nhq");
        move_wall_clock(Duration::from_secs(30));
        let (enables, disables) = (app.enables(), app.disables());
        keys(cx, panel, key);
        reset_wall_clock();
        assert_eq!(dots(cx), left, "{key}: the key found the old text");
        assert_eq!(
            (app.enables(), app.disables()),
            (enables + 1, disables + 1),
            "{key}: the old buffer's secure input, then the new one's"
        );
        assert!(app.secure.held(), "{key}");
    }
}

/// Every way of closing the locked panel wipes what was typed and lets go
/// of secure input: the shortcut, Cmd+W, and Escape with nothing typed.
#[gpui::test]
fn closing_the_locked_panel_wipes_the_buffer(cx: &mut TestAppContext) {
    let app = locked_app(cx, Setup::TouchId);
    for (how, close) in [
        ("the shortcut", None),
        ("Cmd+W", Some("cmd-w")),
        ("Escape, twice", Some("escape escape")),
    ] {
        let panel = app.open_panel(cx);
        activate(cx, panel);
        type_into(cx, panel, "nhq");
        let view = view(cx);
        let before = app.disables();
        match close {
            None => {
                cx.update(panel::toggle);
                cx.run_until_parked();
            }
            Some(keystrokes) => keys(cx, panel, keystrokes),
        }
        assert!(!panel_is_open(cx), "{how} left the panel open");
        assert!(
            app.disables() > before,
            "{how}: secure input stayed on after closing"
        );
        assert!(!app.secure.held(), "{how}");
        view.read_with(cx, |view, _| {
            assert!(
                !view.locked.has_buffer(),
                "{how}: the buffer outlived the panel"
            );
            assert_eq!(view.locked.dots_for_test(), 0, "{how}");
        });
    }
}

/// A click into another app closes the locked panel and wipes the buffer.
#[gpui::test]
fn a_click_outside_wipes_and_closes(cx: &mut TestAppContext) {
    let app = locked_app(cx, Setup::TouchId);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    type_into(cx, panel, SENTINEL);
    let view = view(cx);
    activate(cx, app.other);
    assert!(!panel_is_open(cx));
    assert_eq!(app.disables(), 1);
    assert!(!app.secure.held());
    view.read_with(cx, |view, _| assert!(!view.locked.has_buffer()));
}

/// Escape wipes what was typed and the panel stays; the next Escape closes
/// it.
#[gpui::test]
fn escape_wipes_and_closes(cx: &mut TestAppContext) {
    let app = locked_app(cx, Setup::TouchId);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    type_into(cx, panel, "nhq");
    keys(cx, panel, "escape");
    assert!(panel_is_open(cx));
    assert_eq!(dots(cx), 0, "Escape left the text");
    assert_eq!(app.disables(), 1);
    keys(cx, panel, "escape");
    assert!(!panel_is_open(cx), "the second Escape closes");
    assert_eq!(app.disables(), 2);
    assert!(!app.secure.held());
}

// --------------------------------------------------- what it shows --

/// The field shows a dot for each key and never the keys: its node is a
/// text input named "Search" without a value, the dots are a text without
/// an id and a role, so GPUI writes no node for them, and no part holds the
/// typed text, nor two of its characters side by side more often than with
/// nothing typed. The parts are read as the panel's window draws them, from
/// its own view, before and after typing. The button, which `drawn_parts`
/// does not read, is left to `only_the_dots_show_how_much_was_typed`.
#[gpui::test]
fn typed_keys_show_as_dots_and_never_as_a_value(cx: &mut TestAppContext) {
    let app = locked_app(cx, Setup::TouchId);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    let quiet = drawn_parts(cx);
    assert!(is_key(cx, panel), "reading the panel took its keyboard");
    type_into(cx, panel, "sp-t");
    assert_eq!(dots(cx), 4);
    type_into(cx, panel, &SENTINEL[4..]);
    assert_eq!(dots(cx), SENTINEL.chars().count());

    let parts = drawn_parts(cx);
    assert_eq!(
        dots(cx),
        SENTINEL.chars().count(),
        "reading the panel wiped the text"
    );
    let field = parts
        .iter()
        .find(|part| part.name == "field")
        .expect("the field");
    assert_eq!(field.role, Some(Role::TextInput));
    assert_eq!(field.node.label(), Some(tr!("a11y.search").as_ref()));
    assert_eq!(field.node.value(), None, "the field has a value");
    let dots = parts
        .iter()
        .find(|part| part.name == "dots")
        .expect("the dots");
    assert_eq!(dots.role, None, "the dots have a node");
    assert_eq!(dots.id, None, "the dots have a node");
    // What the dots draw, which GPUI writes into no node for a text without
    // an id.
    assert_eq!(
        dots.node.value(),
        Some("\u{2022}".repeat(SENTINEL.chars().count()).as_str()),
        "a dot per key"
    );
    let found = leaks(&parts, &quiet, SENTINEL);
    assert!(
        found.is_empty(),
        "the typed text reaches assistive technology:\n{}",
        found.join("\n")
    );
}

/// Each locked case says its sentence. The unlock button shows for Touch ID
/// only, and says "Mac password" when no sensor is in reach; setting up has
/// a button of its own.
#[gpui::test]
fn every_locked_case_renders_its_sentence(cx: &mut TestAppContext) {
    let vault: SharedString = "team-beratung".into();
    let missing: SharedString = "team-beratung.kdbx".into();
    let cases = [
        (
            Kind::TouchId {
                device_password: false,
            },
            tr!("quick.locked.typed_ahead"),
            Some(tr!("quick.locked.unlock_touch_id")),
        ),
        (
            Kind::TouchId {
                device_password: true,
            },
            tr!("quick.locked.typed_ahead"),
            Some(tr!("quick.locked.unlock_mac_password")),
        ),
        (
            Kind::NeedsMainWindow,
            tr!("quick.locked.needs_main_window"),
            None,
        ),
        (
            Kind::NotSetUp,
            tr!("quick.locked.not_set_up"),
            Some(tr!("quick.locked.set_up")),
        ),
        (
            Kind::FileMissing(PathBuf::from("/Volumes/x/team-beratung.kdbx")),
            tr!("quick.locked.file_missing", vault = missing),
            None,
        ),
        (
            Kind::Unlocking,
            tr!("quick.locked.still_unlocking", vault = vault),
            None,
        ),
    ];
    for (kind, said, button) in cases {
        let shown = match &kind {
            Kind::FileMissing(_) => missing.clone(),
            _ => vault.clone(),
        };
        let parts = parts_of(
            cx,
            Look {
                kind: kind.clone(),
                dots: 0,
                vault: shown,
            },
        );
        let sentence = parts
            .iter()
            .find(|part| part.name == "sentence")
            .expect("a sentence");
        assert_eq!(sentence.role, Some(Role::Label), "{kind:?}");
        assert_eq!(sentence.node.value(), Some(said.as_ref()), "{kind:?}");
        assert_eq!(button_name(&kind), button, "{kind:?}");
        let shown_as = |dots| Look {
            kind: kind.clone(),
            dots,
            vault: vault.clone(),
        };
        let footer = footer_model(&shown_as(0));
        assert_eq!(
            footer.main.map(|(keys, _)| keys),
            matches!(kind, Kind::NeedsMainWindow | Kind::FileMissing(_)).then_some("cmd-o"),
            "{kind:?}: Cmd+O opens the main window where only that helps"
        );
        assert_eq!(footer.note, None, "{kind:?}: nothing typed");
        assert_eq!(
            footer_model(&shown_as(3)).note,
            Some(tr!("quick.locked.search_pending")),
            "{kind:?}: the footer says the search is saved"
        );
    }
}

/// While locked, the panel shows nothing from a vault: a vault with a
/// favorite and an entry used recently was open, the panel listed both,
/// then the vault was locked. The panel lists no row, and no part
/// `drawn_parts` reads names an entry in its node. The button it does not
/// read is the one `button` makes from the locked case, which
/// `only_the_dots_show_how_much_was_typed` holds.
#[gpui::test]
fn the_locked_panel_shows_nothing_from_a_vault(cx: &mut TestAppContext) {
    let app = panel::tests::open_app(
        cx,
        &[
            panel::tests::entry("Salesforce").starred(),
            panel::tests::entry("Nordhafen VPN"),
        ],
    );
    let id = app.id(cx, "Nordhafen VPN");
    app.state.update(cx, |state, _| state.mark_entry_used(&id));
    app.open_panel(cx);
    let listed = cx
        .update(|cx| panel::for_test(cx).1)
        .expect("the model")
        .read_with(cx, |vm, _| vm.results().len());
    assert_eq!(listed, 2, "the test needs both entries listed while open");
    app.state.update(cx, |state, cx| {
        assert!(state.lock_vault(cx), "the lock waited for a save");
    });
    cx.run_until_parked();
    let panel = app.open_panel(cx);
    activate(cx, panel);
    let vm = cx.update(|cx| panel::for_test(cx).1).expect("the model");
    vm.read_with(cx, |vm, _| {
        assert!(vm.results().is_empty(), "rows while locked");
        assert_eq!(vm.vault_name(), None);
    });
    let parts = drawn_parts(cx);
    for part in &parts {
        for (property, text) in part.texts() {
            let Some(text) = text else {
                continue;
            };
            for title in ["Salesforce", "Nordhafen"] {
                assert!(
                    !text.contains(title),
                    "the {property} of the {} names {title}: {text:?}",
                    part.name
                );
            }
        }
    }
}

/// A missing vault is named by its file name alone, without its folder, and
/// cleaned up first: no control characters, no marks that turn the writing
/// direction around, and no more than sixty characters.
#[gpui::test]
fn the_file_name_in_file_missing_is_a_basename_through_display_untrusted(cx: &mut TestAppContext) {
    let app = locked_app(cx, Setup::Missing);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    let Kind::FileMissing(path) = kind(cx) else {
        panic!(
            "the test needs a missing vault file, the panel says {:?}",
            kind(cx)
        );
    };
    assert_eq!(path, app.path);
    let look = view(cx).read_with(cx, |view, cx| view.look(cx));
    assert_eq!(look.vault.as_ref(), "team-beratung.kdbx");
    let said = sentence(&look.kind, &look.vault);
    assert!(said.contains("team-beratung.kdbx"));
    let folder = app.path.parent().expect("a folder").display().to_string();
    assert!(!said.contains(&folder), "the folder is shown: {said}");

    let tricky = Path::new("/Users/n/Tresore/inv\u{202E}xbdk.ecioval\u{0007}.kdbx");
    assert_eq!(file_name(tricky).as_ref(), "invxbdk.ecioval.kdbx");
    let long = format!("/tmp/{}.kdbx", "a".repeat(80));
    let shown = file_name(Path::new(&long));
    assert_eq!(shown.chars().count(), NAME_MAX + 1);
    assert!(shown.ends_with('\u{2026}'));
}

// --------------------------------------------------------- Touch ID --

/// Touch ID starts from the panel on Return and on a click of the unlock
/// button, and on nothing else: not when the panel opens, nor for keys, nor
/// when it becomes key again, nor for a tick, nor when the buffer expires,
/// nor after macOS withdrew the prompt.
#[gpui::test]
fn return_and_the_button_are_the_only_ways_to_touch_id(cx: &mut TestAppContext) {
    let app = locked_app(cx, Setup::TouchId);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    type_into(cx, panel, "nhq");
    panel
        .update(cx, |_, window, cx| {
            let view = panel::view_for_test(cx).expect("the panel's view");
            view.update(cx, |view, cx| view.on_activation(true, window, cx));
        })
        .expect("the panel");
    tick(cx);
    move_wall_clock(Duration::from_secs(31));
    tick(cx);
    reset_wall_clock();
    assert_eq!(app.retrieves(), 0, "Touch ID without Return or a click");

    app.store.fail_next_retrieve(BiometricError::Interrupted);
    keys(cx, panel, "enter");
    assert_eq!(app.retrieves(), 1, "Return");
    tick(cx);
    activate(cx, app.other);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    tick(cx);
    assert_eq!(app.retrieves(), 1, "a withdrawn prompt was asked again");

    app.store.fail_next_retrieve(BiometricError::UserCancelled);
    click_unlock(cx, panel);
    assert_eq!(app.retrieves(), 2, "the unlock button");
}

/// After Touch ID answers, the panel takes the keyboard back before Return
/// counts again: a Return while the prompt is up starts nothing, the panel
/// is made key and its field focused once macOS has answered, and the next
/// Return starts the next attempt.
#[gpui::test]
fn the_panel_regains_key_and_focus_before_it_takes_return(cx: &mut TestAppContext) {
    let app = locked_app(cx, Setup::TouchId);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    type_into(cx, panel, "nhq");
    app.store.fail_next_retrieve(BiometricError::UserCancelled);
    return_with_the_dialog(cx, &app, panel);
    press_return(cx, panel);
    panel
        .update(cx, |_, window, cx| window.blur(cx))
        .expect("the panel");
    cx.run_until_parked();

    assert_eq!(
        app.retrieves(),
        1,
        "a Return while the prompt was up started one"
    );
    assert_eq!(key_taken(), 1, "the panel did not take key status back");
    assert!(is_key(cx, panel), "the panel is not the key window again");
    assert!(!is_key(cx, app.other));
    let focused = panel.update(cx, |_, window, cx| {
        let view = panel::view_for_test(cx).expect("the panel's view");
        view.read(cx).focus.is_focused(window)
    });
    assert_eq!(
        focused.ok(),
        Some(true),
        "the field did not get the keyboard back"
    );
    assert_eq!(dots(cx), 3);

    app.store.fail_next_retrieve(BiometricError::UserCancelled);
    keys(cx, panel, "enter");
    assert_eq!(
        app.retrieves(),
        2,
        "Return after the answer started nothing"
    );
}

/// While the Touch ID dialog has key status, secure input is off and keys
/// go nowhere; the panel stays open. Once macOS has answered, secure input
/// is on again, with the text and its clock as they were.
#[gpui::test]
fn secure_input_is_released_while_the_dialog_is_key_and_taken_again_after(cx: &mut TestAppContext) {
    let app = locked_app(cx, Setup::TouchId);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    type_into(cx, panel, "nhq");
    assert_eq!((app.enables(), app.disables()), (1, 0));

    app.store.fail_next_retrieve(BiometricError::UserCancelled);
    return_with_the_dialog(cx, &app, panel);
    assert_eq!(app.disables(), 1, "secure input stayed on under the dialog");
    assert!(!app.secure.held());
    assert!(panel_is_open(cx), "the dialog closed the panel");
    cx.dispatch_keystroke(panel, Keystroke::parse("x").expect("a key"));
    assert_eq!(dots(cx), 3, "a key went in without secure input");
    move_wall_clock(Duration::from_secs(20));
    cx.run_until_parked();

    assert_eq!(
        app.enables(),
        2,
        "secure input not taken again after the dialog"
    );
    assert!(app.secure.held());
    assert_eq!(dots(cx), 3, "the text changed through the dialog");
    // The clock ran on through the dialog: ten more seconds make thirty.
    move_wall_clock(Duration::from_secs(10));
    tick(cx);
    assert_eq!(dots(cx), 0, "the 30 seconds started again after the dialog");
}

/// macOS withdrew the prompt: nothing else is asked, the buffer stays, and
/// only Return or the button asks again.
#[gpui::test]
fn an_interrupted_prompt_is_not_asked_again_from_the_panel(cx: &mut TestAppContext) {
    let app = locked_app(cx, Setup::TouchId);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    type_into(cx, panel, "nhq");
    app.store.fail_next_retrieve(BiometricError::Interrupted);
    keys(cx, panel, "enter");
    for _ in 0..3 {
        tick(cx);
    }
    activate(cx, panel);
    assert_eq!(app.retrieves(), 1, "the panel asked again on its own");
    assert_eq!(
        kind(cx),
        Kind::TouchId {
            device_password: false
        }
    );
    assert_eq!(dots(cx), 3, "the buffer went with the withdrawn prompt");
    assert_eq!(search(cx), "");
}

/// What was typed becomes the search only once the panel's own unlock
/// opened the vault. A cancelled prompt, a withdrawn one, a password that
/// does not open the vault and an answer that came after the user moved on
/// leave the buffer and give no search. Opened, the buffer is used up and
/// its text is the search, in the model and in the field.
#[gpui::test]
fn the_typed_text_becomes_the_search_only_after_opened(cx: &mut TestAppContext) {
    if !in_a_home_of_its_own("the_typed_text_becomes_the_search_only_after_opened") {
        return;
    }
    let app = locked_app(cx, Setup::TouchId);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    type_into(cx, panel, "nhq");
    let other = app.path.with_file_name("elbe.kdbx");

    for answer in ["cancelled", "withdrawn", "not opened", "stale"] {
        match answer {
            "cancelled" => app.store.fail_next_retrieve(BiometricError::UserCancelled),
            "withdrawn" => app.store.fail_next_retrieve(BiometricError::Interrupted),
            "not opened" => app.enrolled_password(cx, "not the password"),
            _ => app.enrolled_password(cx, PASSWORD),
        }
        press_return(cx, panel);
        if answer == "stale" {
            // Another vault is asked for while the prompt is up.
            app.state
                .update(cx, |state, cx| state.request_password(other.clone(), cx));
        }
        cx.run_until_parked();
        assert!(app.retrieves() > 0, "{answer}: no prompt");
        assert!(!app.vault_is_open(cx), "{answer}: the vault opened");
        assert_eq!(search(cx), "", "{answer}: the typed text became the search");
        assert_eq!(dots(cx), 3, "{answer}: the buffer went");
        if answer == "stale" {
            app.state
                .update(cx, |state, cx| state.request_password(app.path.clone(), cx));
            cx.run_until_parked();
        }
    }

    app.enrolled_password(cx, PASSWORD);
    keys(cx, panel, "enter");
    assert!(app.vault_is_open(cx), "the test needs the vault open");
    assert_eq!(search(cx), "nhq");
    assert_eq!(search_field(cx), "nhq");
    assert!(!has_buffer(cx), "the buffer outlived the handover");
    assert!(
        !app.secure.held(),
        "secure input stayed with the handed-over text"
    );
}

/// The vault opens some other way while the locked panel is open, here with
/// the master password in the main window: what was typed is wiped as it
/// opens and secure input goes with it, although the panel stays open, and
/// none of it becomes the search.
#[gpui::test]
fn a_vault_opened_another_way_gets_nothing_of_what_was_typed(cx: &mut TestAppContext) {
    if !in_a_home_of_its_own("a_vault_opened_another_way_gets_nothing_of_what_was_typed") {
        return;
    }
    let app = locked_app(cx, Setup::TouchId);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    type_into(cx, panel, SENTINEL);
    let view = view(cx);
    assert!(app.secure.held());

    let document = crate::keepass::KeePassRepository::open(&app.path, PASSWORD, None)
        .expect("open the vault file");
    app.state.update(cx, |state, cx| {
        state.begin_open(app.path.clone(), cx);
        assert!(
            state.finish_open_attempt(app.path.clone(), Ok(document), cx),
            "the vault did not open"
        );
    });
    cx.run_until_parked();
    assert!(app.vault_is_open(cx), "the test needs the vault open");
    assert!(panel_is_open(cx), "the test needs the panel open");
    assert!(
        !app.secure.held(),
        "secure input stayed on after the vault opened another way"
    );
    view.read_with(cx, |view, _| {
        assert!(
            !view.locked.has_buffer(),
            "the text outlived the vault opening"
        );
        assert_eq!(view.locked.dots_for_test(), 0);
    });
    assert_eq!(app.retrieves(), 0, "the test needs no Touch ID");
    assert_eq!(search(cx), "", "the typed text became the search");
    assert_eq!(search_field(cx), "");
}

/// Several words without a single match look like a password typed into
/// the wrong window: they are dropped and the search stays empty. One word
/// with a match, and several words with a match, become the search.
#[gpui::test]
fn a_password_like_buffer_never_becomes_the_search(cx: &mut TestAppContext) {
    if !in_a_home_of_its_own("a_password_like_buffer_never_becomes_the_search") {
        return;
    }
    let app = locked_app(cx, Setup::TouchId);
    // Two words that match nothing in a vault with NHQ 100 and Salesforce,
    // then one word and two words that find NHQ 100.
    for (typed, searched) in [("zebra 747", ""), ("nhq", "nhq"), ("nhq 100", "nhq 100")] {
        let panel = app.open_panel(cx);
        activate(cx, panel);
        type_into(cx, panel, typed);
        keys(cx, panel, "enter");
        assert!(
            app.vault_is_open(cx),
            "{typed}: the test needs the vault open"
        );
        assert_eq!(search(cx), searched, "{typed}");
        assert_eq!(search_field(cx), searched, "{typed}");
        assert!(!has_buffer(cx), "{typed}");
        cx.update(panel::close);
        app.state.update(cx, |state, cx| {
            assert!(state.lock_vault(cx), "the lock waited for a save");
        });
        cx.run_until_parked();
    }
}

/// The 30 seconds run out while the Touch ID dialog is up: the vault opens
/// with an empty search, and nothing of the text is left.
#[gpui::test]
fn an_expired_buffer_opens_an_empty_search(cx: &mut TestAppContext) {
    if !in_a_home_of_its_own("an_expired_buffer_opens_an_empty_search") {
        return;
    }
    let app = locked_app(cx, Setup::TouchId);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    type_into(cx, panel, "nhq");
    return_with_the_dialog(cx, &app, panel);
    move_wall_clock(Duration::from_secs(31));
    cx.executor().advance_clock(TICK);
    cx.run_until_parked();
    assert!(app.vault_is_open(cx), "the test needs the vault open");
    assert_eq!(search(cx), "", "expired text became the search");
    assert_eq!(search_field(cx), "");
    assert!(!has_buffer(cx));
}

/// Return waits while an unlock is under way: a second Return starts no
/// second prompt, and the vault opens from the first. Once it is open,
/// Return acts on the results again.
#[gpui::test]
fn return_waits_while_an_unlock_is_under_way(cx: &mut TestAppContext) {
    if !in_a_home_of_its_own("return_waits_while_an_unlock_is_under_way") {
        return;
    }
    let app = locked_app(cx, Setup::TouchId);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    type_into(cx, panel, "sales");
    press_return(cx, panel);
    assert_eq!(kind(cx), Kind::Unlocking);
    let attempt = view(cx).read_with(cx, |view, _| view.locked.attempt_for_test());
    assert!(attempt.is_some(), "the attempt did not start");
    press_return(cx, panel);
    assert_eq!(
        view(cx).read_with(cx, |view, _| view.locked.attempt_for_test()),
        attempt,
        "a second Return went for another attempt"
    );
    click_unlock(cx, panel);
    assert_eq!(
        app.retrieves(),
        1,
        "a second prompt while one was under way"
    );
    assert!(
        app.vault_is_open(cx),
        "the first attempt did not open the vault"
    );
    assert_eq!(search(cx), "sales");

    // Open, Return runs the main action of the entry it found: Salesforce
    // has no address, so the footer says the website did not open.
    cx.executor().advance_clock(locked_panel::RETURN_SETTLE);
    cx.run_until_parked();
    keys(cx, panel, "enter");
    let vm = cx.update(|cx| panel::for_test(cx).1).expect("the model");
    let note = vm.read_with(cx, |vm, _| vm.footer().note);
    assert_eq!(note, Some(tr!("quick.error.website_failed")));
}

/// Should the panel be key again while the prompt is still up, it takes
/// secure input back at once, with the text as it was, and the answer
/// takes it no second time.
#[gpui::test]
fn the_panel_key_again_during_the_prompt_takes_secure_input_back(cx: &mut TestAppContext) {
    let app = locked_app(cx, Setup::TouchId);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    type_into(cx, panel, "nhq");
    app.store.fail_next_retrieve(BiometricError::UserCancelled);
    return_with_the_dialog(cx, &app, panel);
    assert!(!app.secure.held());

    panel
        .update(cx, |_, window, cx| {
            let view = panel::view_for_test(cx).expect("the panel's view");
            view.update(cx, |view, cx| view.on_activation(true, window, cx));
        })
        .expect("the panel");
    assert!(app.secure.held(), "the key panel holds no secure input");
    assert_eq!(app.enables(), 2);
    cx.dispatch_keystroke(panel, Keystroke::parse("x").expect("a key"));
    assert_eq!(dots(cx), 4, "a key did not go in under secure input");
    cx.run_until_parked();
    assert_eq!(
        app.enables(),
        2,
        "the answer took secure input a second time"
    );
    assert!(app.secure.held());
    assert_eq!(dots(cx), 4);
}

/// Once macOS has answered, the dialog is gone. Losing key status while
/// the vault still opens is a click into another app: the panel closes and
/// wipes what was typed, and the vault opens with nothing of it.
#[gpui::test]
fn a_click_outside_while_the_vault_opens_wipes_and_closes(cx: &mut TestAppContext) {
    if !in_a_home_of_its_own("a_click_outside_while_the_vault_opens_wipes_and_closes") {
        return;
    }
    let app = locked_app(cx, Setup::TouchId);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    type_into(cx, panel, SENTINEL);
    return_with_the_dialog(cx, &app, panel);
    while key_taken() == 0 {
        assert!(cx.executor().tick(), "macOS never answered");
    }
    let view = view(cx);
    assert_eq!(
        kind(cx),
        Kind::Unlocking,
        "the test needs the vault still opening"
    );
    panel
        .update(cx, |_, window, cx| {
            view.update(cx, |view, cx| view.on_activation(false, window, cx));
        })
        .expect("the panel");
    cx.run_until_parked();
    assert!(app.vault_is_open(cx), "the test needs the vault open");
    assert!(!panel_is_open(cx), "a click outside left the panel open");
    view.read_with(cx, |view, _| {
        assert!(
            !view.locked.has_buffer(),
            "the text outlived the click outside"
        );
        assert_eq!(view.locked.dots_for_test(), 0);
    });
    assert!(!app.secure.held());
    assert_eq!(
        search(cx),
        "",
        "the text typed before the click became the search"
    );
}

/// Nothing in the locked panel's file writes to a log, prints, or formats
/// a text, so nothing typed can reach one that way.
#[test]
fn the_locked_panel_never_logs() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui/windows/panel/locked.rs");
    let code = probe::code_only(&std::fs::read_to_string(&path).expect("read locked.rs"));
    let code = code
        .split("#[cfg(test)]\npub(crate) mod tests;")
        .next()
        .unwrap_or_default();
    let mut found = Vec::new();
    for pattern in [
        "log::",
        "println!",
        "eprintln!",
        "print!(",
        "eprint!(",
        "dbg!",
        "format!(",
        "format_args!(",
        "write!(",
        "writeln!(",
        "panic!(",
        "{:?}",
    ] {
        if code.contains(pattern) {
            found.push(pattern);
        }
    }
    assert!(found.is_empty(), "locked.rs writes or formats: {found:?}");
    assert!(
        code.contains("fn on_locked_key_down("),
        "the scan reads the panel"
    );
}

/// The code of `locked.rs` without its comments and its tests, line for
/// line as in the file.
fn locked_code() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui/windows/panel/locked.rs");
    let code = probe::code_only(&std::fs::read_to_string(&path).expect("read locked.rs"));
    code.split("#[cfg(test)]\npub(crate) mod tests;")
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Where the body of `function` is in `code`, braces included.
fn body_of(code: &str, function: &str) -> Range<usize> {
    let start = code
        .find(function)
        .unwrap_or_else(|| panic!("{function} in locked.rs"));
    let open = start + code[start..].find('{').expect("a body");
    open..open + probe::braced(&code[open..]).len()
}

/// Only `build` and `button` give an element of the locked panel an id, a
/// role, a name, a value or a text of its own. The rest of `locked.rs`,
/// `locked_parts` with the handlers and `assemble` with the layout among
/// it, gives none, so what the locked panel hands to assistive technology
/// is what those two make: the parts `drawn_parts` reads, and the button,
/// which the tests read by its name. Read through `code_only`, so comments
/// and the tests do not count; a call made through a macro or a function
/// in another file is past it.
#[test]
fn only_build_names_what_the_locked_panel_draws() {
    let mut code = locked_code();
    for function in ["fn build(", "fn button("] {
        let body = body_of(&code, function);
        // Blanked rather than cut, so a line found below is the line in
        // the file.
        let blank: String = code[body.clone()]
            .chars()
            .map(|c| if c == '\n' { '\n' } else { ' ' })
            .collect();
        code.replace_range(body, &blank);
    }
    for function in ["fn locked_parts(", "fn assemble(", "fn render_locked("] {
        assert!(code.contains(function), "the scan reads {function}");
    }
    let mut found = Vec::new();
    for (number, line) in code.lines().enumerate() {
        for call in [
            ".id(",
            ".role(",
            "Role::",
            "aria_",
            "a11y_",
            "accessibility_",
            "tooltip",
            "Text::new(",
            "text!(",
            "ElementId::",
            "with_id(",
            "InteractiveText",
            "Label",
            "Button::",
            ".description(",
        ] {
            if line.contains(call) {
                found.push(format!("locked.rs:{}: {}", number + 1, line.trim()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "an element of the locked panel gets a node or a text outside build:\n{}",
        found.join("\n")
    );
}

/// How many characters were typed reaches no node through `build`: it
/// reads the number once, for the dots, a text without an id. `build` and
/// `button` put nothing inside an element, so nothing inside the field
/// comes from them but the dots, which `assemble` puts there. And `build`
/// hands the button on as `button` makes it from the locked case, so the
/// button, whose node only the design system writes, gets nothing of what
/// was typed. `footer_model`, which `build` calls, reads only whether
/// anything was typed, and `drawn_parts` reads the footer's parts. Read
/// through `code_only`; the number read under another name is past it.
#[test]
fn only_the_dots_show_how_much_was_typed() {
    let code = locked_code();
    let build = body_of(&code, "fn build(");
    let bodies = [build.clone(), body_of(&code, "fn button(")];
    let mut reads = Vec::new();
    let mut inside = Vec::new();
    let mut start = 0;
    for (number, line) in code.split('\n').enumerate() {
        let end = start + line.len();
        if bodies
            .iter()
            .any(|body| start < body.end && body.start < end)
        {
            let at = format!("locked.rs:{}: {}", number + 1, line.trim());
            for _ in line.matches(".dots") {
                reads.push(at.clone());
            }
            if line.contains(".child(") || line.contains(".children(") {
                inside.push(at);
            }
        }
        start = end + 1;
    }
    assert!(
        reads.len() == 1 && reads[0].ends_with("dots: Text::new_inaccessible(dots(look.dots)),"),
        "build reads how much was typed for more than the dots, or draws the dots with a node:\n{}",
        reads.join("\n")
    );
    assert!(
        inside.is_empty(),
        "build puts an element inside another:\n{}",
        inside.join("\n")
    );
    assert!(
        code[build].contains("button: button(&look.kind),"),
        "build changes the button that `button` makes"
    );
}

//! The locked panel as the app opens it: the main window on the unlock screen
//! of a vault enrolled for Touch ID, then the panel as a second window. Secure
//! event input is a fake that counts its calls, the store is the in-memory
//! one, and "another app" is a window of its own that the test brings to the
//! front.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::{AnyWindowHandle, AppContext as _, Entity, Role, TestAppContext};
use keepass::config::{DatabaseConfig, KdfConfig};
use keepass::db::fields;
use keepass::{Database, DatabaseKey};

use super::*;
use crate::app::actions;
use crate::app::services::clipboard::ClipboardService;
use crate::biometric::memory::InMemoryBiometricStore;
use crate::biometric::registry::{BiometricEnrollment, BiometricRegistry};
use crate::biometric::{BiometricStore, EnrollmentId};
use crate::clipboard::{AccessBehavior, fake::FakePasteboard};
use crate::platform::secure_input::fake::{self as secure, Fake, FakeApi};
use crate::platform::secure_input::{HoldError, OsStatus};
use crate::ui::ds::components::probe;
use crate::ui::windows::panel;

/// The master password of every vault here.
const PASSWORD: &str = "correct horse battery staple";

/// What the tests type, and look for in every node.
const SENTINEL: &str = "sp-typed-7Qx";

thread_local! {
    /// How far the tests moved the wall clock forward, on this thread.
    static AHEAD: Cell<Duration> = const { Cell::new(Duration::ZERO) };
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

/// The parts of the locked panel for `look`, read one by one as assistive
/// technology gets them.
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
            sentence,
            button: _,
            footer: _,
        } = build(&look, &focus);
        let mut parts = vec![
            probe::part("field", field),
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
/// text input named "Search" without a value, and no node of the locked
/// panel holds the typed text, nor two of its characters side by side more
/// often than with nothing typed.
#[gpui::test]
fn typed_keys_show_as_dots_and_never_as_a_value(cx: &mut TestAppContext) {
    let app = locked_app(cx, Setup::TouchId);
    let panel = app.open_panel(cx);
    activate(cx, panel);
    type_into(cx, panel, "sp-t");
    assert_eq!(dots(cx), 4);
    type_into(cx, panel, &SENTINEL[4..]);
    assert_eq!(dots(cx), SENTINEL.chars().count());

    let look = view(cx).read_with(cx, |view, cx| view.look(cx));
    let quiet = Look {
        dots: 0,
        ..look.clone()
    };
    let parts = parts_of(cx, look);
    let quiet = parts_of(cx, quiet);
    let field = parts
        .iter()
        .find(|part| part.name == "field")
        .expect("the field");
    assert_eq!(field.role, Some(Role::TextInput));
    assert_eq!(field.node.label(), Some(tr!("a11y.search").as_ref()));
    assert_eq!(field.node.value(), None, "the field has a value");
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
/// favorite and an entry used recently was open, then locked. The panel
/// lists no row, and no node it draws names an entry.
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
    let look = view(cx).read_with(cx, |view, cx| view.look(cx));
    let parts = parts_of(cx, look);
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

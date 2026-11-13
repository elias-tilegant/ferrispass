//! The panel as the app opens it: the main window over a test vault, then the
//! panel as a second window, driven by its keys and actions.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use ::keepass::db::{Database, fields};
use gpui::{AnyWindowHandle, Entity, RenderOnce, Role, TestAppContext, WindowKind};
use gpui_component::WindowExt as _;

use super::view::{footer, rows_of, search_group};
use super::*;
use crate::app::actions::{
    self, ClosePanel, MinimizeWindow, NewEntry, PanelMoveDown, PanelMoveUp, PanelSelect,
};
use crate::app::registry::CommandId;
use crate::app::services::activity;
use crate::app::services::clipboard::ClipboardService;
use crate::app::{LibrarySelection, VaultStatus};
use crate::clipboard::{AccessBehavior, fake::FakePasteboard};
use crate::keepass::repository::snapshot_from_database;
use crate::keepass::{FAVORITE_TAG, VaultDocument};
use crate::tr;
use crate::ui::ds::components::{FooterTone, SearchGroup, probe};
use crate::ui::vm::panel::PanelVm;

/// The vault the tests open. Its folder does not exist, so nothing is ever
/// written there.
const VAULT: &str = "/nonexistent/ferrispass-panel-tests/team-beratung.kdbx";

thread_local! {
    static LAUNCHES: RefCell<Vec<(String, bool)>> = const { RefCell::new(Vec::new()) };
}

/// Called instead of starting SAP GUI, with the entry and whether a password
/// came along.
pub(super) fn note_launch(entry_id: String, with_password: bool) {
    LAUNCHES.with(|launches| launches.borrow_mut().push((entry_id, with_password)));
}

fn launches() -> Vec<(String, bool)> {
    LAUNCHES.with(|launches| launches.borrow().clone())
}

/// An entry of the test vault.
#[derive(Clone, Default)]
pub(crate) struct Fixture {
    title: &'static str,
    user: &'static str,
    password: &'static str,
    url: &'static str,
    starred: bool,
    fields: Vec<(&'static str, &'static str)>,
    notes: &'static str,
    protected: Vec<(&'static str, &'static str)>,
}

pub(crate) fn entry(title: &'static str) -> Fixture {
    Fixture {
        title,
        ..Fixture::default()
    }
}

impl Fixture {
    fn user(self, user: &'static str) -> Self {
        Self { user, ..self }
    }

    fn password(self, password: &'static str) -> Self {
        Self { password, ..self }
    }

    fn url(self, url: &'static str) -> Self {
        Self { url, ..self }
    }

    pub(crate) fn starred(self) -> Self {
        Self {
            starred: true,
            ..self
        }
    }

    fn field(mut self, key: &'static str, value: &'static str) -> Self {
        self.fields.push((key, value));
        self
    }

    fn notes(self, notes: &'static str) -> Self {
        Self { notes, ..self }
    }

    fn protected(mut self, key: &'static str, value: &'static str) -> Self {
        self.protected.push((key, value));
        self
    }
}

fn document(entries: &[Fixture]) -> VaultDocument {
    let mut database = Database::new();
    {
        let mut root = database.root_mut();
        for fixture in entries {
            let mut entry = root.add_entry();
            entry.set_unprotected(fields::TITLE, fixture.title);
            if !fixture.user.is_empty() {
                entry.set_unprotected(fields::USERNAME, fixture.user);
            }
            if !fixture.password.is_empty() {
                entry.set_protected(fields::PASSWORD, fixture.password);
            }
            if !fixture.url.is_empty() {
                entry.set_unprotected(fields::URL, fixture.url);
            }
            for (key, value) in &fixture.fields {
                entry.set_unprotected(*key, *value);
            }
            if !fixture.notes.is_empty() {
                entry.set_unprotected(fields::NOTES, fixture.notes);
            }
            for (key, value) in &fixture.protected {
                entry.set_protected(*key, *value);
            }
            if fixture.starred {
                entry.tags.push(FAVORITE_TAG.to_string());
            }
        }
    }
    let snapshot = snapshot_from_database(&database);
    VaultDocument::new(database, snapshot, "pw".to_string(), None)
}

/// The app with its main window open on a vault holding `entries`.
pub(crate) struct Harness {
    pub(crate) state: Entity<AppState>,
    pub(crate) main: AnyWindowHandle,
    pasteboard: FakePasteboard,
}

pub(crate) fn open_app(cx: &mut TestAppContext, entries: &[Fixture]) -> Harness {
    open_app_at(cx, VAULT, entries)
}

/// The same, with the vault at `path`, whose folder must not exist.
fn open_app_at(cx: &mut TestAppContext, path: &str, entries: &[Fixture]) -> Harness {
    crate::autotype::hotkey::fake::reset();
    LAUNCHES.with(|launches| launches.borrow_mut().clear());
    let pasteboard = FakePasteboard::new(AccessBehavior::Default);
    cx.update(|cx| {
        gpui_component::init(cx);
        actions::init(cx);
        ClipboardService::install_for_test(cx, Box::new(pasteboard.clone()));
    });
    let state = cx.new(|_| {
        let mut state = AppState::default();
        state.open_for_test(PathBuf::from(path), document(entries));
        state
    });
    cx.update(|cx| crate::app::open_main_window_for_test(cx, state.clone()));
    cx.run_until_parked();
    let main = cx
        .update(|cx| actions::main_window(cx))
        .expect("the main window");
    Harness {
        state,
        main,
        pasteboard,
    }
}

impl Harness {
    /// The id of the entry titled `title`.
    pub(crate) fn id(&self, cx: &mut TestAppContext, title: &str) -> String {
        self.state.read_with(cx, |state, _| {
            state
                .vault_browser()
                .expect("an open vault")
                .snapshot
                .live_entries()
                .into_iter()
                .find(|entry| entry.title == title)
                .map(|entry| entry.id.clone())
                .unwrap_or_else(|| panic!("no entry titled {title}"))
        })
    }

    /// Opens the panel as the shortcut does and returns its window.
    pub(crate) fn open_panel(&self, cx: &mut TestAppContext) -> AnyWindowHandle {
        cx.update(open);
        cx.run_until_parked();
        panel_window(cx).expect("the panel opened")
    }

    fn vm(&self, cx: &mut TestAppContext) -> Entity<PanelVm> {
        cx.update(|cx| for_test(cx).1).expect("the panel's model")
    }

    fn query(&self, cx: &mut TestAppContext) -> String {
        self.vm(cx).read_with(cx, |vm, _| vm.query().to_string())
    }

    fn titles(&self, cx: &mut TestAppContext) -> Vec<String> {
        let ids: Vec<String> = self.vm(cx).read_with(cx, |vm, _| {
            vm.results()
                .iter()
                .map(|hit| hit.entry_id.clone())
                .collect()
        });
        self.state.read_with(cx, |state, _| {
            let browser = state.vault_browser().expect("an open vault");
            ids.iter()
                .map(|id| {
                    browser
                        .snapshot
                        .find_entry(id)
                        .map(|entry| entry.title.clone())
                        .unwrap_or_default()
                })
                .collect()
        })
    }

    fn selected_title(&self, cx: &mut TestAppContext) -> Option<String> {
        let index = self.vm(cx).read_with(cx, |vm, _| vm.selection().index())?;
        self.titles(cx).get(index).cloned()
    }

    /// What the clipboard service wrote, and how often.
    fn copied(&self) -> (usize, Option<String>) {
        let ledger = self.pasteboard.ledger();
        (ledger.writes, ledger.text.clone())
    }

    fn vault_is_open(&self, cx: &mut TestAppContext) -> bool {
        self.state.read_with(cx, |state, _| {
            matches!(state.vault_status(), VaultStatus::Open { .. })
        })
    }

    /// The entries the main window lists as recently used.
    fn recently_used(&self, cx: &mut TestAppContext) -> Vec<String> {
        self.state.update(cx, |state, cx| {
            let before = state.vault_browser().expect("open").selection;
            state.select_library(LibrarySelection::RecentlyUsed, cx);
            let titles = state
                .vault_browser()
                .expect("open")
                .entries
                .iter()
                .map(|entry| entry.title.clone())
                .collect();
            state.select_library(before, cx);
            titles
        })
    }
}

fn panel_window(cx: &mut TestAppContext) -> Option<AnyWindowHandle> {
    cx.update(|cx| for_test(cx).0)
}

fn panel_is_open(cx: &mut TestAppContext) -> bool {
    cx.update(|cx| is_open(cx))
}

/// Makes `window` the key window, as clicking it or opening it with focus
/// does on macOS.
fn activate(cx: &mut TestAppContext, window: AnyWindowHandle) {
    window
        .update(cx, |_, window, _| window.activate_window())
        .expect("a window to activate");
    cx.run_until_parked();
}

fn type_into(cx: &mut TestAppContext, panel: AnyWindowHandle, text: &str) {
    cx.simulate_input(panel, text);
    cx.run_until_parked();
}

/// Presses `keystrokes` in the panel. Return counts only once the results
/// have been on screen for a moment, as for someone who reads them first,
/// so before a Return the clock moves on by that moment.
fn keys(cx: &mut TestAppContext, panel: AnyWindowHandle, keystrokes: &str) {
    if keystrokes.split(' ').any(|keystroke| keystroke == "enter") {
        cx.executor()
            .advance_clock(crate::app::unlock::locked_panel::RETURN_SETTLE);
        cx.run_until_parked();
    }
    cx.simulate_keystrokes(panel, keystrokes);
    cx.run_until_parked();
}

/// A vault with two SAP systems, NHQ 100 and NHP 100, both signing in to
/// client 100 as N.WEISS through SAP GUI, a favorite web login, a remote
/// access entry and a web login of their own.
fn sample() -> Vec<Fixture> {
    vec![
        entry("NHQ 100")
            .user("N.WEISS")
            .password("sp-nhq")
            .field("SAP_HOST", "nhq.example")
            .field("SAP_INSTANCE", "00")
            .field("SAP_CLIENT", "100"),
        entry("NHP 100")
            .user("N.WEISS")
            .password("sp-nhp")
            .field("SAP_HOST", "nhp.example")
            .field("SAP_INSTANCE", "00")
            .field("SAP_CLIENT", "100"),
        entry("Salesforce")
            .user("nora@example.com")
            .password("sp-sales")
            .url("https://login.salesforce.com")
            .starred(),
        entry("Nordhafen VPN").user("nweiss").password("sp-vpn"),
        entry("Webmail")
            .user("nora")
            .password("sp-mail")
            .url("mail.example.com/login"),
    ]
}

// ------------------------------------------------------------- the window --

#[gpui::test]
fn the_panel_opens_as_a_popup_that_takes_focus(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let options = cx.update(|cx| window_options(cx));
    assert_eq!(options.kind, WindowKind::PopUp, "a non-activating panel");
    assert!(options.focus, "it takes the keyboard");
    assert!(options.show);
    assert!(!options.is_movable);
    assert!(!options.is_resizable);
    assert!(options.titlebar.is_none());
    let display = gpui::Bounds {
        origin: gpui::point(px(0.), px(0.)),
        size: size(px(1512.), px(982.)),
    };
    let bounds = panel_bounds(display);
    assert_eq!(bounds.size, size(px(WIDTH), px(HEIGHT)));
    assert_eq!(bounds.origin.x, px((1512. - WIDTH) / 2.));
    assert!(
        bounds.origin.y < px(982. / 3.),
        "the panel starts in the upper third"
    );
    let small = gpui::Bounds {
        origin: gpui::point(px(100.), px(50.)),
        size: size(px(600.), px(400.)),
    };
    let bounds = panel_bounds(small);
    assert_eq!(bounds.size, size(px(600.), px(400.)), "never larger");
    assert_eq!(bounds.origin, gpui::point(px(100.), px(50.)));

    // Opened, the panel is a window of its own with the search field
    // focused, and the main window is the one it was.
    let panel = app.open_panel(cx);
    assert_ne!(panel, app.main);
    type_into(cx, panel, "nh");
    assert_eq!(app.query(cx), "nh", "what is typed reaches the search");
    assert_eq!(cx.update(|cx| actions::main_window(cx)), Some(app.main));
}

#[gpui::test]
fn the_quick_access_shortcut_toggles_the_panel(cx: &mut TestAppContext) {
    let _app = open_app(cx, &sample());
    cx.update(toggle);
    cx.run_until_parked();
    assert!(panel_is_open(cx));
    cx.update(toggle);
    cx.run_until_parked();
    assert!(!panel_is_open(cx));
    assert!(panel_window(cx).is_none());
}

#[gpui::test]
fn clicking_elsewhere_closes_the_panel(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let panel = app.open_panel(cx);
    activate(cx, panel);
    assert!(panel_is_open(cx), "taking the keyboard keeps it open");
    activate(cx, app.main);
    assert!(!panel_is_open(cx), "losing the keyboard closes it");
    assert!(app.vault_is_open(cx));
}

// ------------------------------------------------------ search and list --

#[gpui::test]
fn typing_ranks_the_open_vault(cx: &mut TestAppContext) {
    let mut entries = sample();
    entries.push(entry("NHD 100 old").password("sp-old"));
    let app = open_app(cx, &entries);
    let nhp = app.id(cx, "NHP 100");
    app.state.update(cx, |state, _| state.mark_entry_used(&nhp));
    let panel = app.open_panel(cx);
    type_into(cx, panel, "nh");
    // The recently used NHP 100 first, then the other matches by their text.
    let titles = app.titles(cx);
    assert_eq!(titles.first().map(String::as_str), Some("NHP 100"));
    assert!(titles.contains(&"NHQ 100".to_string()));
    assert!(titles.contains(&"NHD 100 old".to_string()));
    assert!(!titles.contains(&"Salesforce".to_string()));
    let expected: Vec<String> = app.state.read_with(cx, |state, _| {
        state
            .ranked_for_panel("nh", 9)
            .into_iter()
            .map(|(entry, _)| entry.title)
            .collect()
    });
    assert_eq!(titles, expected, "the order of the search everywhere else");

    // An exact identifier comes first, whatever was used.
    type_into(cx, panel, "q 100");
    assert_eq!(app.titles(cx).first().map(String::as_str), Some("NHQ 100"));
}

#[gpui::test]
fn the_panel_lists_at_most_nine_rows(cx: &mut TestAppContext) {
    let titles = [
        "Server 01",
        "Server 02",
        "Server 03",
        "Server 04",
        "Server 05",
        "Server 06",
        "Server 07",
        "Server 08",
        "Server 09",
        "Server 10",
        "Server 11",
    ];
    let entries: Vec<Fixture> = titles.iter().map(|title| entry(title)).collect();
    let app = open_app(cx, &entries);
    let panel = app.open_panel(cx);
    assert_eq!(app.titles(cx).len(), 9, "an empty search lists nine");
    type_into(cx, panel, "server");
    assert_eq!(app.titles(cx).len(), 9, "a search lists nine");
}

#[gpui::test]
fn every_new_query_selects_the_first_row_and_late_results_do_not_move_it(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let panel = app.open_panel(cx);
    type_into(cx, panel, "n");
    let first = app.titles(cx)[0].clone();
    assert_eq!(app.selected_title(cx), Some(first));
    keys(cx, panel, "down");
    let picked = app.selected_title(cx).expect("a selection");
    assert_eq!(picked, app.titles(cx)[1]);

    // Results that change under the same query, as after a sync: the
    // picked entry stays selected.
    let picked_id = app.id(cx, &picked);
    app.state.update(cx, |state, cx| {
        state.mark_entry_used(&picked_id);
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(app.selected_title(cx), Some(picked.clone()));
    assert!(app.vm(cx).read_with(cx, |vm, _| vm.is_explicit()));

    // A new query starts on its first row again, and nothing is picked.
    type_into(cx, panel, "h");
    assert_eq!(app.selected_title(cx), Some(app.titles(cx)[0].clone()));
    assert!(!app.vm(cx).read_with(cx, |vm, _| vm.is_explicit()));
}

#[gpui::test]
fn up_and_down_never_wrap_and_cmd_numbers_select(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let panel = app.open_panel(cx);
    type_into(cx, panel, "nh");
    let count = app.titles(cx).len();
    assert!((2..9).contains(&count), "{count} rows");
    let last = count - 1;
    let index = |cx: &mut TestAppContext| app.vm(cx).read_with(cx, |vm, _| vm.selection().index());

    cx.dispatch_action(panel, PanelMoveUp);
    cx.run_until_parked();
    assert_eq!(index(cx), Some(0), "up on the first row stays there");
    for _ in 0..count + 2 {
        cx.dispatch_action(panel, PanelMoveDown);
    }
    cx.run_until_parked();
    assert_eq!(index(cx), Some(last), "down on the last row stays there");
    cx.dispatch_action(panel, PanelSelect { number: 1 });
    cx.run_until_parked();
    assert_eq!(index(cx), Some(0));
    cx.dispatch_action(panel, PanelSelect { number: 2 });
    cx.run_until_parked();
    assert_eq!(index(cx), Some(1));
    cx.dispatch_action(panel, PanelSelect { number: 9 });
    cx.run_until_parked();
    assert_eq!(index(cx), Some(1), "a number past the list changes nothing");
    cx.dispatch_action(panel, PanelMoveUp);
    cx.dispatch_action(panel, PanelMoveUp);
    cx.run_until_parked();
    assert_eq!(index(cx), Some(0), "up on the first row stays there");
}

/// The GPUI Kit's search field keeps the arrow keys and Cmd+C to itself
/// unless the panel binds them in `Panel > Input`. The field has the focus,
/// as it always does in the panel.
#[gpui::test]
fn arrow_keys_and_cmd_c_reach_the_panel_while_the_search_field_is_focused(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let panel = app.open_panel(cx);
    type_into(cx, panel, "nh");
    let titles = app.titles(cx);
    assert_eq!(app.selected_title(cx), Some(titles[0].clone()));

    keys(cx, panel, "down");
    assert_eq!(
        app.selected_title(cx),
        Some(titles[1].clone()),
        "down moved"
    );
    keys(cx, panel, "up");
    assert_eq!(app.selected_title(cx), Some(titles[0].clone()), "up moved");
    keys(cx, panel, "cmd-2");
    assert_eq!(
        app.selected_title(cx),
        Some(titles[1].clone()),
        "cmd-2 picked"
    );

    let password = sample()
        .into_iter()
        .find(|fixture| fixture.title == titles[1])
        .map(|fixture| fixture.password)
        .expect("a sample entry");
    keys(cx, panel, "cmd-c");
    assert_eq!(
        app.copied(),
        (1, Some(password.to_string())),
        "cmd-c copied the selected entry's password, once"
    );
    // Even with the search text marked.
    keys(cx, panel, "cmd-a cmd-c");
    assert_eq!(app.copied(), (2, Some(password.to_string())));
    assert_eq!(app.query(cx), "nh", "the search is untouched");
}

#[gpui::test]
fn panel_actions_use_the_panel_selection_not_the_main_windows(cx: &mut TestAppContext) {
    let app = open_app(
        cx,
        &[
            entry("Alpha").user("main-user").password("sp-main-A"),
            entry("Bravo").user("panel-user").password("sp-panel-B"),
        ],
    );
    let alpha = app.id(cx, "Alpha");
    let bravo = app.id(cx, "Bravo");
    app.state.update(cx, |state, cx| {
        state.select_library(LibrarySelection::Favorites, cx);
        state.select_library(LibrarySelection::AllItems, cx);
        state.select_entry(alpha.clone(), cx);
    });
    let main_selection = |cx: &mut TestAppContext| {
        app.state.read_with(cx, |state, _| {
            state
                .vault_browser()
                .and_then(|browser| browser.selected_entry_id)
        })
    };
    assert_eq!(main_selection(cx), Some(alpha.clone()));

    let panel = app.open_panel(cx);
    type_into(cx, panel, "bravo");
    assert_eq!(app.selected_title(cx).as_deref(), Some("Bravo"));
    keys(cx, panel, "cmd-c");
    assert_eq!(app.copied(), (1, Some("sp-panel-B".to_string())));
    assert_eq!(
        main_selection(cx),
        Some(alpha.clone()),
        "the main window's selection moved"
    );
    assert_eq!(app.recently_used(cx), ["Bravo"], "Bravo counts as used");

    keys(cx, panel, "cmd-shift-c");
    assert_eq!(app.copied(), (2, Some("panel-user".to_string())));

    keys(cx, panel, "cmd-o");
    assert!(!panel_is_open(cx), "show in FerrisPass closes the panel");
    assert_eq!(
        main_selection(cx),
        Some(bravo),
        "the main window shows the panel's entry"
    );
    assert_eq!(app.recently_used(cx), ["Bravo"], "Alpha was not touched");
}

// ------------------------------------------------------- names and footer --

#[gpui::test]
fn a_row_is_named_after_the_pattern_and_never_by_the_letters_prd(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let panel = app.open_panel(cx);
    type_into(cx, panel, "nhp 100");
    let vm = app.vm(cx);
    let hit = vm.read_with(cx, |vm, _| vm.results()[0].clone());
    assert!(hit.production);
    let name = hit.parts.accessible_name();
    let production = tr!("glossary.role.production_system");
    assert!(
        name.contains(production.as_ref()),
        "the role in words: {name}"
    );
    assert!(
        !name.contains("PRD"),
        "the letters stay on the chip: {name}"
    );
    assert_eq!(
        name.to_string(),
        format!(
            "{} NHP 100, {production}, {} 100, N.WEISS, team-beratung",
            tr!("glossary.type.sap_system"),
            tr!("glossary.sap_client")
        )
    );
    let (tone, letters) = hit.chip.clone().expect("a chip");
    assert_eq!(tone, crate::ui::ds::components::ChipTone::Production);
    assert_eq!(letters.as_ref(), "PRD");

    // The row's node takes that name.
    let model = vm.read_with(cx, |vm, _| vm.clone());
    let label = probe::in_frame(cx, move |window, cx| {
        let row = rows_of(&model).into_iter().next().expect("a row");
        let (_, node) = probe::node(RenderOnce::render(row, window, cx));
        node.label().map(str::to_string)
    });
    assert_eq!(label.as_deref(), Some(name.as_ref()));
}

#[gpui::test]
fn the_selected_row_carries_the_footer_text_and_the_empty_group_carries_it_too(
    cx: &mut TestAppContext,
) {
    let app = open_app(cx, &sample());
    let panel = app.open_panel(cx);
    type_into(cx, panel, "n");
    keys(cx, panel, "cmd-2");
    let vm = app.vm(cx);
    let model = vm.read_with(cx, |vm, _| vm.clone());
    let descriptions: Vec<Option<String>> = probe::in_frame(cx, move |window, cx| {
        rows_of(&model)
            .into_iter()
            .map(|row| {
                let (_, node) = probe::node(RenderOnce::render(row, window, cx));
                node.description().map(str::to_string)
            })
            .collect()
    });
    let text = cx.update(|cx| footer(&vm.read(cx).footer()).description_text());
    assert!(!text.is_empty());
    assert!(descriptions.len() >= 2);
    for (index, description) in descriptions.iter().enumerate() {
        if index == 1 {
            assert_eq!(
                description.as_deref(),
                Some(text.as_ref()),
                "the selected row"
            );
        } else {
            assert_eq!(description, &None, "row {index} is not selected");
        }
    }

    // No row: the group carries it, here the note about the other vaults.
    type_into(cx, panel, "zzz");
    vm.update(cx, |vm, _| vm.assume_parked_vaults(1));
    assert!(app.titles(cx).is_empty());
    let model = vm.read_with(cx, |vm, _| vm.clone());
    let (group, expected) = probe::in_frame(cx, move |window, cx| {
        let input = cx.new(|cx| gpui_component::input::InputState::new(window, cx));
        let vault = model.vault_name().cloned().unwrap_or_default();
        let expected = footer(&model.footer()).description_text();
        let parts = SearchGroup::parts(|| search_group(&model, &input, &vault), window, cx);
        let group = parts
            .into_iter()
            .find(|part| part.name == "group")
            .expect("the group");
        (group.node.description().map(str::to_string), expected)
    });
    assert!(expected.contains(tr!("quick.footer.other_vaults").as_ref()));
    assert_eq!(group.as_deref(), Some(expected.as_ref()));
}

#[gpui::test]
fn an_empty_search_never_lets_return_act_on_a_production_system(cx: &mut TestAppContext) {
    let mut entries = sample();
    // A starred production system is the first row of an empty search.
    entries[1].starred = true;
    entries[2].starred = false;
    let app = open_app(cx, &entries);
    let panel = app.open_panel(cx);
    assert_eq!(app.selected_title(cx).as_deref(), Some("NHP 100"));
    let vm = app.vm(cx);
    let model = vm.read_with(cx, |vm, _| vm.footer());
    assert_eq!(model.tone, FooterTone::ProductionSelectFirst);
    assert_eq!(model.primary, None, "Return runs nothing");
    assert_eq!(
        model.main.as_ref().map(|(_, words)| words.clone()),
        Some(tr!("quick.footer.prd_select_first"))
    );
    keys(cx, panel, "enter");
    assert!(
        launches().is_empty(),
        "Return signed in to a production system"
    );
    assert!(panel_is_open(cx));

    // Picking it with Cmd+1 makes Return sign in.
    keys(cx, panel, "cmd-1");
    let model = vm.read_with(cx, |vm, _| vm.footer());
    assert_eq!(model.tone, FooterTone::Normal);
    assert_eq!(model.primary, Some(CommandId::ENTRY_SAP_LOGIN));
    let nhp = app.id(cx, "NHP 100");
    keys(cx, panel, "enter");
    assert_eq!(launches(), [(nhp, true)]);
}

/// Return in the open panel counts only for results that were on screen:
/// none before they are drawn, none within 300 ms of that, and none while a
/// vault is still being unlocked. One exact match in a vault that was open
/// before the search takes Return at once, unless it is a production
/// system.
#[gpui::test]
fn return_in_the_open_panel_goes_through_the_gate(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let panel = app.open_panel(cx);
    let vm = app.vm(cx);
    // Return as the keyboard sends it, without the moment `keys` waits.
    let enter = |cx: &mut TestAppContext, panel: AnyWindowHandle| {
        cx.simulate_keystrokes(panel, "enter");
        cx.run_until_parked();
    };
    let redraw = |cx: &mut TestAppContext| {
        vm.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
    };

    type_into(cx, panel, "webm");
    vm.read_with(cx, |vm, _| vm.drawn().clear());
    enter(cx, panel);
    assert_eq!(
        cx.opened_url(),
        None,
        "Return before the results were drawn"
    );
    redraw(cx);
    assert!(vm.read_with(cx, |vm, _| vm.drawn().at().is_some()));
    cx.executor().advance_clock(Duration::from_millis(299));
    enter(cx, panel);
    assert_eq!(cx.opened_url(), None, "Return 299 ms after drawing");
    cx.executor().advance_clock(Duration::from_millis(1));
    enter(cx, panel);
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://mail.example.com/login"),
        "Return 300 ms after drawing"
    );

    // An unlock under way holds Return back, however long the results show.
    // Each part starts from an empty search, not the one kept from before.
    vm.update(cx, |vm, _| vm.forget());
    let panel = app.open_panel(cx);
    type_into(cx, panel, "webm");
    cx.executor().advance_clock(Duration::from_secs(1));
    let now = cx.executor().now();
    let (held, free) = vm.read_with(cx, |vm, _| {
        (
            crate::app::unlock::locked_panel::return_accepted(&vm.return_gate(true), now),
            crate::app::unlock::locked_panel::return_accepted(&vm.return_gate(false), now),
        )
    });
    assert!(!held, "Return while a vault is being unlocked");
    assert!(free);
    cx.update(close);
    cx.run_until_parked();

    // One exact match: the whole title takes Return at once.
    vm.update(cx, |vm, _| vm.forget());
    let panel = app.open_panel(cx);
    type_into(cx, panel, "salesforce");
    enter(cx, panel);
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://login.salesforce.com"),
        "one exact match waited"
    );

    // A production system named exactly waits like any other row.
    vm.update(cx, |vm, _| vm.forget());
    let panel = app.open_panel(cx);
    type_into(cx, panel, "nhp 100");
    assert!(vm.read_with(cx, |vm, _| {
        vm.selected().is_some_and(|hit| hit.exact && hit.production)
    }));
    enter(cx, panel);
    assert!(
        launches().is_empty(),
        "Return signed in to a production system at once"
    );
    cx.executor().advance_clock(Duration::from_millis(300));
    enter(cx, panel);
    assert_eq!(launches().len(), 1, "Return 300 ms later signs in");
}

#[gpui::test]
fn typing_lifts_the_production_rule(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let panel = app.open_panel(cx);
    type_into(cx, panel, "nhp");
    let model = app.vm(cx).read_with(cx, |vm, _| vm.footer());
    assert_eq!(model.primary, Some(CommandId::ENTRY_SAP_LOGIN));
    let words = model.main.map(|(_, words)| words).unwrap_or_default();
    assert_eq!(
        words,
        tr!(
            "quick.footer.sap_login",
            sid = "NHP",
            client = "100",
            user = "N.WEISS"
        ),
        "the footer names the system, the client and the user"
    );
}

/// An entry is a production system in the panel when the fields the row
/// shows make it one, or when the whole entry, protected fields included,
/// does. NHX 100 has a SID without a production letter, so the fields the
/// row shows give it no role, and the role PRD in a protected field: in
/// the panel it is a production system. From an empty search Return signs
/// in to nothing, and typed in full, the one exact match waits like any
/// production system.
#[gpui::test]
fn a_production_role_in_a_protected_field_counts_in_the_panel(cx: &mut TestAppContext) {
    let app = open_app(
        cx,
        &[entry("NHX 100")
            .user("N.WEISS")
            .password("sp-nhx")
            .field("SAP_HOST", "nhx.example")
            .field("SAP_INSTANCE", "00")
            .field("SAP_CLIENT", "100")
            .protected("SAP_ROLE", "PRD")
            .starred()],
    );
    let panel = app.open_panel(cx);
    let vm = app.vm(cx);
    assert_eq!(app.selected_title(cx).as_deref(), Some("NHX 100"));
    keys(cx, panel, "enter");
    assert!(
        launches().is_empty(),
        "Return signed in to a production system from an empty search"
    );
    let model = vm.read_with(cx, |vm, _| vm.footer());
    assert_eq!(model.tone, FooterTone::ProductionSelectFirst);
    assert_eq!(model.primary, None, "Return runs nothing");

    // Typed in full, the one exact match does not take Return at once.
    type_into(cx, panel, "nhx 100");
    assert!(vm.read_with(cx, |vm, _| vm.selected().is_some_and(|hit| hit.exact)));
    cx.simulate_keystrokes(panel, "enter");
    cx.run_until_parked();
    assert!(
        launches().is_empty(),
        "Return signed in to a production system at once"
    );
    cx.executor()
        .advance_clock(crate::app::unlock::locked_panel::RETURN_SETTLE);
    cx.simulate_keystrokes(panel, "enter");
    cx.run_until_parked();
    assert_eq!(launches(), [(app.id(cx, "NHX 100"), true)]);

    // The row says so as a production system's row does, and that is all
    // it takes from the protected field.
    let hit = vm.read_with(cx, |vm, _| vm.results()[0].clone());
    assert!(hit.production);
    let (tone, letters) = hit.chip.clone().expect("a chip");
    assert_eq!(tone, crate::ui::ds::components::ChipTone::Production);
    assert_eq!(letters.as_ref(), "PRD");
    let name = hit.parts.accessible_name();
    assert!(
        name.contains(tr!("glossary.role.production_system").as_ref()),
        "the role in words: {name}"
    );
}

/// A protected field can make an entry a production system in the panel,
/// but never make a production system an ordinary one. NHP 100 has the
/// production letter in its SID and plain `SAP_HOST`, `SAP_INSTANCE` and
/// `SAP_CLIENT`, so the fields the row shows make it a production system
/// and its chip says PRD. Its protected `SAP_ROLE` of QAS would make it a
/// quality system, were the whole entry the only answer: then a row that
/// shows PRD would sign in from an empty search. It stays a production
/// system: from an empty search Return signs in to nothing, and the one
/// exact match waits like any production system.
#[gpui::test]
fn a_protected_role_never_takes_production_away_in_the_panel(cx: &mut TestAppContext) {
    let app = open_app(
        cx,
        &[entry("NHP 100")
            .user("N.WEISS")
            .password("sp-nhp")
            .field("SAP_HOST", "nhp.example")
            .field("SAP_INSTANCE", "00")
            .field("SAP_CLIENT", "100")
            .protected("SAP_ROLE", "QAS")
            .starred()],
    );
    let panel = app.open_panel(cx);
    let vm = app.vm(cx);
    assert_eq!(app.selected_title(cx).as_deref(), Some("NHP 100"));
    let hit = vm.read_with(cx, |vm, _| vm.results()[0].clone());
    let (tone, letters) = hit.chip.clone().expect("a chip");
    assert_eq!(tone, crate::ui::ds::components::ChipTone::Production);
    assert_eq!(letters.as_ref(), "PRD");

    keys(cx, panel, "enter");
    assert!(
        launches().is_empty(),
        "Return signed in to a production system from an empty search"
    );
    let model = vm.read_with(cx, |vm, _| vm.footer());
    assert_eq!(model.tone, FooterTone::ProductionSelectFirst);
    assert_eq!(model.primary, None, "Return runs nothing");

    // Typed in full, the one exact match does not take Return at once.
    type_into(cx, panel, "nhp 100");
    assert!(vm.read_with(cx, |vm, _| vm.selected().is_some_and(|hit| hit.exact)));
    cx.simulate_keystrokes(panel, "enter");
    cx.run_until_parked();
    assert!(
        launches().is_empty(),
        "Return signed in to a production system at once"
    );
    cx.executor()
        .advance_clock(crate::app::unlock::locked_panel::RETURN_SETTLE);
    cx.simulate_keystrokes(panel, "enter");
    cx.run_until_parked();
    assert_eq!(launches(), [(app.id(cx, "NHP 100"), true)]);
    assert!(vm.read_with(cx, |vm, _| vm.results()[0].production));
}

/// An entry whose SID is protected, under a title that names no system, is
/// a web login as the panel shows it, and a production system read over
/// the whole entry, its protected `SAP_SID` of NHP included. Return waits
/// for it as for any production system: from an empty search it opens
/// nothing, and typed in full, the one exact match does not take Return at
/// once, only 300 ms after the results are drawn.
#[gpui::test]
fn a_production_system_the_panel_shows_as_a_web_login_waits_too(cx: &mut TestAppContext) {
    let app = open_app(
        cx,
        &[entry("Nordhafen ERP")
            .user("N.WEISS")
            .password("sp-erp")
            .url("https://erp.example.com")
            .protected("SAP_SID", "NHP")
            .starred()],
    );
    let panel = app.open_panel(cx);
    let vm = app.vm(cx);
    let hit = vm.read_with(cx, |vm, _| vm.results()[0].clone());
    assert_eq!(
        hit.selected_kind(),
        crate::app::registry::SelectedKind::Web,
        "the test needs a row the panel shows as a web login"
    );
    keys(cx, panel, "enter");
    assert_eq!(
        cx.opened_url(),
        None,
        "Return acted on a production system from an empty search"
    );
    let model = vm.read_with(cx, |vm, _| vm.footer());
    assert_eq!(model.tone, FooterTone::ProductionSelectFirst);
    assert_eq!(model.primary, None, "Return runs nothing");

    type_into(cx, panel, "nordhafen erp");
    assert!(vm.read_with(cx, |vm, _| vm.selected().is_some_and(|hit| hit.exact)));
    cx.simulate_keystrokes(panel, "enter");
    cx.run_until_parked();
    assert_eq!(
        cx.opened_url(),
        None,
        "Return acted on a production system at once"
    );
    cx.executor()
        .advance_clock(crate::app::unlock::locked_panel::RETURN_SETTLE);
    cx.simulate_keystrokes(panel, "enter");
    cx.run_until_parked();
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://erp.example.com"),
        "Return 300 ms later opens the website"
    );
}

/// The footer names the client and the user only when signing in puts
/// exactly these into SAP GUI's launch file, which reads every field, the
/// protected ones too. NHP 100 signs in to a protected client as a
/// protected user, so the footer says only "Sign in", and no protected
/// value reaches a row, the footer or a node. NHQ 100 signs in to the
/// client and as the user its row shows, and the footer names them. NHD
/// 100 has its client in the title only, which the launch file does not
/// get, so the footer names neither.
#[gpui::test]
fn the_footer_names_client_and_user_only_as_signing_in_uses_them(cx: &mut TestAppContext) {
    const HOST: &str = "sp-host-5Tz.example";
    let app = open_app(
        cx,
        &[
            entry("NHP 100")
                .user("N.WEISS")
                .password("sp-nhp")
                .protected("SAP_HOST", HOST)
                .protected("SAP_INSTANCE", "00")
                .protected("SAP_USER", "ADMIN")
                .protected("SAP_CLIENT", "000"),
            entry("NHQ 100")
                .user("N.WEISS")
                .password("sp-nhq")
                .field("SAP_HOST", "nhq.example")
                .field("SAP_INSTANCE", "00")
                .field("SAP_CLIENT", "100"),
            entry("NHD 100")
                .user("N.WEISS")
                .password("sp-nhd")
                .field("SAP_HOST", "nhd.example")
                .field("SAP_INSTANCE", "00"),
        ],
    );
    let panel = app.open_panel(cx);
    let vm = app.vm(cx);
    let words = |cx: &mut TestAppContext| {
        vm.read_with(cx, |vm, _| vm.footer().main.map(|(_, words)| words))
            .unwrap_or_default()
    };
    let sign_in = tr!("action.sap_login.label");

    type_into(cx, panel, "nhp");
    assert_eq!(app.selected_title(cx).as_deref(), Some("NHP 100"));
    assert_eq!(
        words(cx),
        sign_in,
        "the footer names a client and a user the launch file does not get"
    );
    let model = vm.read_with(cx, |vm, _| vm.clone());
    let text = footer(&model.footer()).description_text().to_string();
    for shown in ["N.WEISS", "100"] {
        assert!(!text.contains(shown), "the footer names {shown}: {text}");
    }
    let mut seen = vec![text];
    for hit in model.results() {
        seen.push(hit.parts.accessible_name().to_string());
    }
    let nodes: Vec<String> = probe::in_frame(cx, move |window, cx| {
        rows_of(&model)
            .into_iter()
            .map(|row| {
                let (_, node) = probe::node(RenderOnce::render(row, window, cx));
                format!(
                    "{:?} {:?} {:?}",
                    node.label(),
                    node.description(),
                    node.value()
                )
            })
            .collect()
    });
    seen.extend(nodes);
    for text in &seen {
        for secret in ["ADMIN", "000", HOST] {
            assert!(!text.contains(secret), "{secret} reached the panel: {text}");
        }
    }
    let debug = vm.read_with(cx, |vm, _| format!("{vm:?}"));
    for secret in ["ADMIN", HOST] {
        assert!(!debug.contains(secret), "{secret} in {debug}");
    }

    keys(cx, panel, "escape");
    type_into(cx, panel, "nhq");
    assert_eq!(app.selected_title(cx).as_deref(), Some("NHQ 100"));
    assert_eq!(
        words(cx),
        tr!(
            "quick.footer.sap_login",
            sid = "NHQ",
            client = "100",
            user = "N.WEISS"
        ),
        "signing in uses what the row shows, and the footer names it"
    );

    keys(cx, panel, "escape");
    type_into(cx, panel, "nhd");
    assert_eq!(app.selected_title(cx).as_deref(), Some("NHD 100"));
    assert_eq!(
        words(cx),
        sign_in,
        "the footer names a client the launch file does not get"
    );
}

/// The search field's label is the header, "Searching team-beratung". The
/// model has the vault's name, and in the source `render` hands that name
/// to `search_group`, which labels the field with it: the test platform
/// builds no accessibility tree, and the field's node is the Kit input's,
/// which only the design system reads.
#[gpui::test]
fn the_header_names_the_vault_and_the_footer_mentions_parked_vaults(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let _panel = app.open_panel(cx);
    let vm = app.vm(cx);
    assert_eq!(
        vm.read_with(cx, |vm, _| vm.vault_name().cloned()),
        Some("team-beratung".into())
    );
    let header = tr!("quick.header.searching_vault", vault = "team-beratung");
    assert!(header.contains("team-beratung"));
    let view = squeezed(&source_of("src/ui/windows/panel/view.rs"));
    let group = function(&view, "fnsearch_group(");
    assert!(
        group.contains(
            r#"TextField::new("panel-search",search,tr!("quick.header.searching_vault",vault=vault),)"#
        ),
        "the search field is not labelled with the vault it searches: {group}"
    );
    let render = function(&view, "fnrender(");
    assert!(
        render.contains("letopen=self.vm.read(cx).vault_name().cloned();")
            && render.contains("Some(vault)=>")
            && render.contains("search_group(vm,&self.search,&vault)"),
        "render hands search_group another name than the model's: {render}"
    );
    let note = |cx: &mut TestAppContext| vm.read_with(cx, |vm, _| vm.footer().note);
    assert_eq!(note(cx), None, "no other vault, no note");
    vm.update(cx, |vm, _| vm.assume_parked_vaults(2));
    assert_eq!(note(cx), Some(tr!("quick.footer.other_vaults")));
}

/// The code of `file` without its comments.
fn source_of(file: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(file);
    probe::code_only(&std::fs::read_to_string(&path).expect("read the source"))
}

/// `code` without any whitespace, so a check does not depend on how the
/// code is formatted.
fn squeezed(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The body of the function whose signature starts with `start` in
/// `code`, braces included.
fn function<'a>(code: &'a str, start: &str) -> &'a str {
    let at = code
        .find(start)
        .unwrap_or_else(|| panic!("{start} in the source"));
    let open = at + code[at..].find('{').expect("a body");
    probe::braced(&code[open..])
}

/// The vault's name comes from its file name, which whatever put the file
/// on the disk chose. The open panel shows it as the locked panel does:
/// without control characters and the twelve marks that turn the writing
/// direction, U+202E and the Arabic letter mark U+061C among them, cut
/// after sixty characters. Read in the model and in every row's name, off
/// the row's node; all twelve marks are read through `vault_name`.
#[gpui::test]
fn the_open_panel_cleans_the_vault_name_as_the_locked_panel_does(cx: &mut TestAppContext) {
    let app = open_app_at(
        cx,
        "/nonexistent/ferrispass-panel-tests/team-\u{202E}bera\u{061C}tung\u{0007}.kdbx",
        &sample(),
    );
    let _panel = app.open_panel(cx);
    let vm = app.vm(cx);
    assert_eq!(
        vm.read_with(cx, |vm, _| vm.vault_name().cloned()),
        Some("team-beratung".into()),
        "the model did not clean the vault's name of control characters and direction marks"
    );
    let model = vm.read_with(cx, |vm, _| vm.clone());
    assert_eq!(model.results().len(), 5, "the test needs every row listed");
    let labels: Vec<Option<String>> = probe::in_frame(cx, move |window, cx| {
        rows_of(&model)
            .into_iter()
            .map(|row| {
                let (_, node) = probe::node(RenderOnce::render(row, window, cx));
                node.label().map(str::to_string)
            })
            .collect()
    });
    for label in &labels {
        let label = label.as_deref().expect("every row has a name");
        assert!(
            label.ends_with(", team-beratung"),
            "a row names the vault without cleaning its name of control characters and direction marks: {label:?}"
        );
    }

    let long = format!("/tmp/{}.kdbx", "a".repeat(80));
    let shown = crate::ui::vm::panel::vault_name(Path::new(&long));
    assert_eq!(shown.chars().count(), crate::ui::vm::panel::NAME_MAX + 1);
    assert!(shown.ends_with('\u{2026}'));

    let marks = [
        '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}',
        '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
    ];
    for mark in marks {
        let marked = format!("/tmp/te{mark}am.kdbx");
        assert_eq!(
            crate::ui::vm::panel::vault_name(Path::new(&marked)).as_ref(),
            "team",
            "the vault's name keeps the direction mark U+{:04X}",
            u32::from(mark)
        );
    }
}

#[gpui::test]
fn search_and_selection_are_forgotten_after_two_minutes(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let panel = app.open_panel(cx);
    type_into(cx, panel, "nh");
    keys(cx, panel, "down");
    let picked = app.selected_title(cx);
    cx.update(close);
    cx.run_until_parked();

    // Two minutes, written out: the rule, not whatever the constant says.
    cx.executor().advance_clock(Duration::from_secs(119));
    let _panel = app.open_panel(cx);
    assert_eq!(app.query(cx), "nh", "kept for two minutes");
    assert_eq!(app.selected_title(cx), picked);
    cx.update(close);
    cx.run_until_parked();

    cx.executor().advance_clock(Duration::from_secs(120));
    let _panel = app.open_panel(cx);
    assert_eq!(app.query(cx), "", "forgotten after two minutes");
    assert!(!app.vm(cx).read_with(cx, |vm, _| vm.is_explicit()));
}

#[gpui::test]
fn locking_closes_the_panel_and_empties_the_model(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let panel = app.open_panel(cx);
    type_into(cx, panel, "nh");
    app.state.update(cx, |state, cx| {
        let _ = state.lock_vault(cx);
    });
    cx.run_until_parked();
    assert!(!panel_is_open(cx), "locking closed the panel");
    let vm = app.vm(cx);
    vm.read_with(cx, |vm, _| {
        assert_eq!(vm.query(), "");
        assert!(vm.results().is_empty());
        assert_eq!(vm.vault_name(), None);
    });

    // Opened while locked, the panel says it needs the main window, and
    // Cmd+O opens that.
    let _panel = app.open_panel(cx);
    let model = vm.read_with(cx, |vm, _| vm.footer());
    assert_eq!(model.primary, None);
    assert_eq!(model.main, Some(("cmd-o", tr!("action.open.label"))));
}

// ---------------------------------------------------------------- closing --

#[gpui::test]
fn cmd_w_and_escape_close_only_the_panel(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());

    // Cmd+W in the panel. Hiding the app would panic on the test platform.
    let panel = app.open_panel(cx);
    activate(cx, panel);
    keys(cx, panel, "cmd-w");
    assert!(!panel_is_open(cx));
    assert!(app.vault_is_open(cx), "Cmd+W in the panel locked the vault");
    assert!(
        app.main.update(cx, |_, _, _| ()).is_ok(),
        "the main window lives"
    );

    // Escape with an empty search.
    let panel = app.open_panel(cx);
    activate(cx, panel);
    keys(cx, panel, "escape");
    assert!(!panel_is_open(cx));
    assert!(app.vault_is_open(cx));

    // Cmd+M with the panel in front minimizes nothing; minimizing panics on
    // the test platform. Minimize from the Window menu does nothing either.
    let panel = app.open_panel(cx);
    activate(cx, panel);
    keys(cx, panel, "cmd-m");
    cx.dispatch_action(panel, MinimizeWindow);
    cx.run_until_parked();
    cx.update(actions::minimize_main_window);
    cx.run_until_parked();
    assert!(panel_is_open(cx));
    assert!(app.vault_is_open(cx));

    // Close Window, from the menu or from Cmd+W while the field has no
    // focus, closes the panel in front and leaves the main window alone.
    cx.update(actions::close_main_window);
    cx.run_until_parked();
    assert!(!panel_is_open(cx));
    assert!(
        app.vault_is_open(cx),
        "Close Window from the panel locked the vault"
    );

    // The locked panel shows no search field; Cmd+W and Escape close it
    // all the same.
    app.state.update(cx, |state, cx| {
        let _ = state.lock_vault(cx);
    });
    cx.run_until_parked();
    for keystroke in ["cmd-w", "escape"] {
        let panel = app.open_panel(cx);
        activate(cx, panel);
        keys(cx, panel, keystroke);
        assert!(!panel_is_open(cx), "{keystroke} left the locked panel open");
    }
}

#[gpui::test]
fn escape_clears_the_search_before_it_closes(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let panel = app.open_panel(cx);
    type_into(cx, panel, "nh");
    keys(cx, panel, "escape");
    assert!(panel_is_open(cx), "the first Escape only clears");
    assert_eq!(app.query(cx), "");
    keys(cx, panel, "escape");
    assert!(!panel_is_open(cx), "the second Escape closes");

    // Cmd+W closes at once, search or not.
    let panel = app.open_panel(cx);
    type_into(cx, panel, "x");
    cx.dispatch_action(
        panel,
        ClosePanel {
            clear_search_first: false,
        },
    );
    cx.run_until_parked();
    assert!(!panel_is_open(cx));
}

#[gpui::test]
fn quit_and_notifications_from_the_panel_go_to_the_main_window(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    cx.update(|cx| {
        crate::platform::secure_input::fake::install(
            cx,
            crate::platform::secure_input::fake::FakeApi::new(),
        )
    });
    let notices = |cx: &mut TestAppContext, window: AnyWindowHandle| {
        window
            .update(cx, |_, window, cx| window.notifications(cx).len())
            .expect("the window")
    };

    // An unsaved entry in the main window holds Quit back with a notice.
    cx.dispatch_action(app.main, NewEntry);
    cx.run_until_parked();
    cx.simulate_input(app.main, "Bank");
    cx.run_until_parked();
    let panel = app.open_panel(cx);
    activate(cx, panel);
    let main_before = notices(cx, app.main);
    actions::stored_geometry::take();

    cx.update(actions::quit_from_anywhere);
    cx.run_until_parked();
    assert_eq!(
        actions::stored_geometry::take(),
        [app.main],
        "Quit from the panel stored the main window's geometry, and only that"
    );
    assert_eq!(
        notices(cx, app.main),
        main_before + 1,
        "the notice went to the main window"
    );
    assert_eq!(notices(cx, panel), 0, "no notice in the panel");

    // Without a main window nothing happens to it, and nothing panics.
    cx.update(|cx| cx.remove_global::<actions::MainWindow>());
    cx.update(actions::minimize_main_window);
    cx.update(actions::quit_from_anywhere);
    cx.run_until_parked();
    assert!(actions::stored_geometry::take().is_empty());
    assert_eq!(notices(cx, panel), 0);
    // Close Window still closes the panel in front.
    cx.update(actions::close_main_window);
    cx.run_until_parked();
    assert!(!panel_is_open(cx));
}

/// Cmd+W and Cmd+Q pressed in the main window reach the main window. GPUI
/// runs the app's handlers while the window the keys came from is busy
/// dispatching them. An unsaved entry holds both back with a notice, so the
/// app is neither hidden nor quit.
#[gpui::test]
fn close_window_and_quit_reach_the_main_window_from_inside_it(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let notices = |cx: &mut TestAppContext| {
        app.main
            .update(cx, |_, window, cx| window.notifications(cx).len())
            .expect("the main window")
    };
    cx.dispatch_action(app.main, NewEntry);
    cx.run_until_parked();
    cx.simulate_input(app.main, "Bank");
    cx.run_until_parked();
    activate(cx, app.main);
    let before = notices(cx);
    actions::stored_geometry::take();

    cx.simulate_keystrokes(app.main, "cmd-w");
    cx.run_until_parked();
    assert_eq!(
        notices(cx),
        before + 1,
        "Cmd+W in the main window did not reach it"
    );
    assert!(app.vault_is_open(cx));

    // Typing asks again before the next close.
    cx.simulate_input(app.main, "s");
    cx.run_until_parked();
    cx.simulate_keystrokes(app.main, "cmd-q");
    cx.run_until_parked();
    assert_eq!(
        actions::stored_geometry::take(),
        [app.main],
        "Cmd+Q in the main window did not store its geometry"
    );
    assert_eq!(
        notices(cx),
        before + 2,
        "Cmd+Q in the main window did not reach it"
    );
}

/// Cmd+M pressed in the main window reaches the main window: it asks the
/// platform to minimize it, which the test platform has not implemented.
#[gpui::test]
#[should_panic(expected = "not implemented")]
fn minimize_reaches_the_main_window_from_inside_it(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    activate(cx, app.main);
    cx.simulate_keystrokes(app.main, "cmd-m");
    cx.run_until_parked();
}

#[test]
fn no_handler_reads_the_active_window() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in ["src/app", "src/ui/legacy/shell"] {
        probe::rust_files(&root.join(dir), &mut files);
    }
    assert!(
        files
            .iter()
            .any(|file| file.ends_with("src/app/actions.rs"))
    );
    assert!(
        files
            .iter()
            .any(|file| file.ends_with("src/ui/legacy/shell/secrets.rs"))
    );
    let found: Vec<String> = files
        .iter()
        .filter_map(|file| {
            let code = probe::code_only(&std::fs::read_to_string(file).expect("read source"));
            code.contains("active_window(")
                .then(|| file.display().to_string())
        })
        .collect();
    assert!(
        found.is_empty(),
        "these read the key window, which can be the quick access panel; use \
         actions::main_window: {found:?}"
    );
}

// ------------------------------------------------- activity, ids and logs --

#[gpui::test]
fn keys_and_clicks_in_the_panel_count_as_activity(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let panel = app.open_panel(cx);
    assert!(app.vault_is_open(cx));
    let ten_minutes = Duration::from_secs(600);
    let idle = || activity::idle_for(SystemTime::now());

    activity::reset(SystemTime::now() - ten_minutes);
    keys(cx, panel, "down");
    assert!(idle() < Duration::from_secs(1), "a key in the panel");

    activity::reset(SystemTime::now() - ten_minutes);
    let mut window = gpui::VisualTestContext::from_window(panel, cx);
    window.simulate_click(gpui::point(px(40.), px(40.)), gpui::Modifiers::none());
    assert!(idle() < Duration::from_secs(1), "a click in the panel");

    // Without input nothing counts, the panel's open window included.
    activity::reset(SystemTime::now() - ten_minutes);
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert!(idle() >= ten_minutes, "something counted on its own");
}

#[gpui::test]
fn panel_ids_carry_no_title_user_or_host(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let _panel = app.open_panel(cx);
    let vm = app.vm(cx);
    assert!(panel_is_open(cx));
    let model = vm.read_with(cx, |vm, _| vm.clone());
    let ids: Vec<Option<String>> = probe::in_frame(cx, move |window, cx| {
        rows_of(&model)
            .into_iter()
            .map(|row| {
                let (_, node) = probe::node(RenderOnce::render(row, window, cx));
                node.author_id().map(str::to_string)
            })
            .collect()
    });
    assert_eq!(ids.len(), 5);
    let secrets = [
        "NHQ",
        "NHP",
        "Salesforce",
        "Nordhafen",
        "Webmail",
        "N.WEISS",
        "nora",
        "nweiss",
        "salesforce.com",
        "mail.example",
        "nhp.example",
    ];
    for (index, id) in ids.iter().enumerate() {
        let id = id.as_deref().expect("every row has an id");
        assert_eq!(id, format!("fp.panel.row.{index}"));
        for secret in secrets {
            assert!(!id.contains(secret), "{id} names {secret}");
        }
    }
}

/// The panel shows an entry without its notes and its protected custom
/// fields. An SAP system with a note, a protected SAP user and a protected
/// token of its own: neither the model, its `Debug` output included, nor a
/// row's node nor the footer holds any of them. The row names the user from
/// the entry's user name field instead; the footer names no user, since
/// signing in uses the protected one. What the panel takes from protected
/// fields, whether the entry is a production system and whether signing in
/// uses the login the row shows, is a yes or no each, held by
/// `a_production_role_in_a_protected_field_counts_in_the_panel` and
/// `the_footer_names_client_and_user_only_as_signing_in_uses_them`.
#[gpui::test]
fn the_panel_shows_no_notes_and_no_protected_fields(cx: &mut TestAppContext) {
    const NOTE: &str = "sp-note-4Kd";
    const USER: &str = "SP-PROT-9WZ";
    const TOKEN: &str = "sp-token-2Hm";
    let app = open_app(
        cx,
        &[entry("NHQ 100")
            .user("N.WEISS")
            .password("sp-nhq")
            .field("SAP_HOST", "nhq.example")
            .field("SAP_INSTANCE", "00")
            .notes(NOTE)
            .protected("SAP_USER", USER)
            .protected("Token", TOKEN)],
    );
    let panel = app.open_panel(cx);
    type_into(cx, panel, "nhq");
    let model = app.vm(cx).read_with(cx, |vm, _| vm.clone());
    assert_eq!(model.results().len(), 1, "the test needs the entry listed");
    let hit = &model.results()[0];
    let mut seen = vec![
        format!("{model:?}"),
        format!("{:?}", hit.kind),
        hit.parts.accessible_name().to_string(),
        footer(&model.footer()).description_text().to_string(),
    ];
    let row_model = model.clone();
    let nodes: Vec<String> = probe::in_frame(cx, move |window, cx| {
        rows_of(&row_model)
            .into_iter()
            .map(|row| {
                let (_, node) = probe::node(RenderOnce::render(row, window, cx));
                format!(
                    "{:?} {:?} {:?}",
                    node.label(),
                    node.description(),
                    node.value()
                )
            })
            .collect()
    });
    seen.extend(nodes);
    for text in &seen {
        for secret in [NOTE, USER, TOKEN] {
            assert!(!text.contains(secret), "{secret} reached the panel: {text}");
        }
    }
    assert!(
        hit.parts.accessible_name().contains("N.WEISS"),
        "the row names the user from the user name field"
    );
    assert_eq!(
        model.footer().main.map(|(_, words)| words),
        Some(tr!("action.sap_login.label")),
        "the footer names a user signing in does not use"
    );
}

#[gpui::test]
fn the_panel_model_debug_names_no_entry(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let panel = app.open_panel(cx);
    type_into(cx, panel, "nhp");
    let debug = app.vm(cx).read_with(cx, |vm, _| format!("{vm:?}"));
    for secret in ["nhp", "NHP", "N.WEISS", "nhp.example", "team-beratung"] {
        assert!(!debug.contains(secret), "{secret} in {debug}");
    }
}

#[test]
fn nothing_in_the_panel_logs_a_title() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in ["src/ui/windows", "src/ui/vm"] {
        probe::rust_files(&root.join(dir), &mut files);
    }
    assert!(files.len() >= 4, "the scan found {files:?}");
    let mut found = Vec::new();
    // The tests' own file names the patterns it looks for.
    for file in files.iter().filter(|file| !file.ends_with("tests.rs")) {
        let code = probe::code_only(&std::fs::read_to_string(file).expect("read source"));
        for pattern in [
            "log::",
            "eprintln!",
            "println!",
            "dbg!",
            "eprint!",
            "print!(",
        ] {
            if code.contains(pattern) {
                found.push(format!("{}: {pattern}", file.display()));
            }
        }
    }
    assert!(found.is_empty(), "the panel writes to a log: {found:?}");
}

// ------------------------------------------------------- Return and copies --

/// The panel's own files, without the tests, which name what they look
/// for.
fn panel_sources() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in ["src/ui/windows", "src/ui/vm"] {
        probe::rust_files(&root.join(dir), &mut files);
    }
    let sources: Vec<(String, String)> = files
        .into_iter()
        .filter(|file| !file.ends_with("tests.rs"))
        .map(|file| {
            let code = probe::code_only(&std::fs::read_to_string(&file).expect("read source"));
            (file.display().to_string(), code)
        })
        .collect();
    assert!(
        sources
            .iter()
            .any(|(file, _)| file.ends_with("panel/view.rs")),
        "the scan reads the panel's view"
    );
    sources
}

#[gpui::test]
fn return_runs_the_main_action_of_the_kind(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());

    // An SAP system: SAP GUI signs in with the entry the panel selected, and
    // the panel closes.
    let panel = app.open_panel(cx);
    type_into(cx, panel, "nhq");
    keys(cx, panel, "enter");
    assert_eq!(launches(), [(app.id(cx, "NHQ 100"), true)]);
    assert!(!panel_is_open(cx), "the panel closes after signing in");
    assert_eq!(app.recently_used(cx), ["NHQ 100"]);
    assert_eq!(app.copied().0, 0, "signing in copies nothing");

    // A web login: its website opens, an address without a scheme as https,
    // and the panel closes.
    let panel = app.open_panel(cx);
    keys(cx, panel, "escape");
    type_into(cx, panel, "webmail");
    keys(cx, panel, "enter");
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://mail.example.com/login")
    );
    assert!(
        !panel_is_open(cx),
        "the panel closes after opening the website"
    );

    // Remote access: Return copies the password, and the panel stays open.
    let panel = app.open_panel(cx);
    keys(cx, panel, "escape");
    type_into(cx, panel, "vpn");
    keys(cx, panel, "enter");
    assert_eq!(app.copied(), (1, Some("sp-vpn".to_string())));
    assert!(panel_is_open(cx));
    assert_eq!(launches().len(), 1, "nothing else was started");
}

#[gpui::test]
fn copies_go_through_the_clipboard_service_and_keep_the_panel_open(cx: &mut TestAppContext) {
    let mut entries = sample();
    entries.push(
        entry("Two factor")
            .password("sp-2fa")
            .field("otp", "otpauth://totp/X?secret=JBSWY3DPEHPK3PXP"),
    );
    let app = open_app(cx, &entries);
    let panel = app.open_panel(cx);
    let note = |cx: &mut TestAppContext| app.vm(cx).read_with(cx, |vm, _| vm.footer().note);

    type_into(cx, panel, "salesforce");
    keys(cx, panel, "cmd-c");
    assert_eq!(app.copied(), (1, Some("sp-sales".to_string())));
    assert!(
        cx.update(|cx| ClipboardService::handle(cx).read(cx).deadline())
            .is_some(),
        "the copy has a clear timer"
    );
    assert!(panel_is_open(cx), "a copy keeps the panel open");
    assert_eq!(note(cx), Some(tr!("quick.footer.copied.password")));

    keys(cx, panel, "cmd-shift-c");
    assert_eq!(app.copied(), (2, Some("nora@example.com".to_string())));
    assert_eq!(note(cx), Some(tr!("quick.footer.copied.user")));

    keys(cx, panel, "escape");
    type_into(cx, panel, "two factor");
    keys(cx, panel, "alt-cmd-c");
    let (writes, code) = app.copied();
    assert_eq!(writes, 3);
    let code = code.expect("a code");
    assert!(
        code.len() == 6 && code.chars().all(|c| c.is_ascii_digit()),
        "the code, without the space the display puts in: {code:?}"
    );
    assert_eq!(note(cx), Some(tr!("quick.footer.copied.one_time_code")));
    assert!(panel_is_open(cx));

    // Moving the selection takes the note away.
    keys(cx, panel, "escape");
    keys(cx, panel, "down");
    assert_eq!(note(cx), None);
}

/// Runs the test called `test` again in a child process whose home and
/// temporary folder are a new, empty directory, and returns `false` once
/// that run has passed. In the child it returns `true`, and the test goes
/// on there. A test that saves the settings runs in a child, so no other
/// test reads the file it writes.
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

/// A copy from the panel is cleared after the time the settings give, as a
/// copy from the main window is. The test saves 17 seconds, which no preset
/// offers, so the time can only come from the settings; saving them is why
/// it runs in a home of its own.
#[gpui::test]
fn a_copy_is_cleared_after_the_time_the_settings_give(cx: &mut TestAppContext) {
    if !in_a_home_of_its_own("a_copy_is_cleared_after_the_time_the_settings_give") {
        return;
    }
    let app = open_app(cx, &sample());
    cx.update(|cx| {
        crate::app::with_shared_shell(cx, |shell, cx| {
            let settings = crate::app::AppSettings {
                clipboard_clear_secs: Some(17),
                ..shell.settings().clone()
            };
            shell.update_settings(settings, cx);
        })
    })
    .expect("the main window's shell");
    cx.run_until_parked();
    let panel = app.open_panel(cx);
    type_into(cx, panel, "salesforce");
    keys(cx, panel, "cmd-c");
    assert_eq!(app.copied(), (1, Some("sp-sales".to_string())));
    let left = cx
        .update(|cx| ClipboardService::handle(cx).read(cx).deadline())
        .expect("the copy has a clear timer")
        .saturating_duration_since(std::time::Instant::now());
    assert!(
        left > Duration::from_secs(10) && left <= Duration::from_secs(17),
        "the copy is cleared in {left:?}, the settings say 17 s"
    );

    cx.executor().advance_clock(Duration::from_secs(16));
    cx.run_until_parked();
    assert_eq!(
        app.copied().1.as_deref(),
        Some("sp-sales"),
        "cleared before the 17 s were up"
    );
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(app.copied().1, None, "still on the pasteboard after 17 s");
    assert!(!cx.update(|cx| ClipboardService::handle(cx).read(cx).has_pending()));
}

/// The roles that make VoiceOver read a node out as soon as it changes.
fn announces(role: Option<Role>) -> bool {
    matches!(
        role,
        Some(Role::Alert | Role::Status | Role::Log | Role::Marquee | Role::Timer)
    )
}

/// What the panel has to say goes into its footer and nowhere else: no
/// copy, failed copy, refused website or sign-in puts up a notification, in
/// the panel or in the main window, and neither a row nor the group around
/// the list is a live region or has a role that announces itself.
#[gpui::test]
fn what_the_panel_says_stays_in_its_footer(cx: &mut TestAppContext) {
    let mut entries = sample();
    entries.push(entry("Locker").user("nobody"));
    entries.push(entry("Script portal").url("javascript:alert(1)"));
    let app = open_app(cx, &entries);
    let notices = |cx: &mut TestAppContext, window: AnyWindowHandle| {
        window
            .update(cx, |_, window, cx| window.notifications(cx).len())
            .expect("the window")
    };
    let in_main = notices(cx, app.main);
    let panel = app.open_panel(cx);
    let vm = app.vm(cx);
    for (query, keystrokes, said) in [
        ("salesforce", "cmd-c", tr!("quick.footer.copied.password")),
        ("salesforce", "cmd-shift-c", tr!("quick.footer.copied.user")),
        ("locker", "cmd-c", tr!("quick.error.copy_failed")),
        ("script portal", "enter", tr!("quick.error.website_failed")),
    ] {
        if !app.query(cx).is_empty() {
            keys(cx, panel, "escape");
        }
        type_into(cx, panel, query);
        keys(cx, panel, keystrokes);
        let step = format!("{query}, {keystrokes}");
        assert_eq!(
            vm.read_with(cx, |vm, _| vm.footer().note),
            Some(said),
            "{step}: the footer says it"
        );
        assert_eq!(notices(cx, panel), 0, "{step}: a notification in the panel");
        assert_eq!(
            notices(cx, app.main),
            in_main,
            "{step}: a notification in the main window"
        );

        // The rows and the group, drawn with that sentence in the footer.
        let model = vm.read_with(cx, |vm, _| vm.clone());
        let nodes: Vec<(String, Option<Role>, bool)> = probe::in_frame(cx, move |window, cx| {
            let mut nodes = Vec::new();
            for (index, row) in rows_of(&model).into_iter().enumerate() {
                let (role, node) = probe::node(RenderOnce::render(row, window, cx));
                nodes.push((format!("row {index}"), role, node.live().is_some()));
            }
            let input = cx.new(|cx| gpui_component::input::InputState::new(window, cx));
            let vault = model.vault_name().cloned().unwrap_or_default();
            for part in SearchGroup::parts(|| search_group(&model, &input, &vault), window, cx) {
                nodes.push((part.name.to_string(), part.role, part.node.live().is_some()));
            }
            nodes
        });
        assert!(nodes.iter().any(|(name, _, _)| name == "row 0"), "{step}");
        for (name, role, live) in nodes {
            assert!(!announces(role), "{step}: the {name} is {role:?}");
            assert!(!live, "{step}: the {name} is live");
        }
    }

    // Signing in hands over to SAP GUI and closes the panel, quietly too.
    keys(cx, panel, "escape");
    type_into(cx, panel, "nhq");
    keys(cx, panel, "enter");
    assert!(!panel_is_open(cx));
    assert_eq!(
        notices(cx, app.main),
        in_main,
        "signing in from the panel: a notification in the main window"
    );
}

/// No notification and no live region in the panel's code either, so a
/// path the tests above do not drive is held too. The one sentence the
/// panel has for an action stands in its footer.
#[test]
fn the_panel_builds_no_notification_and_no_live_region() {
    let mut found = Vec::new();
    for (file, code) in panel_sources() {
        for (at, line) in code.lines().enumerate() {
            for forbidden in [
                "notification",
                "Notification",
                "Role::Alert",
                "Role::Status",
                "Role::Log",
                "Role::Marquee",
                "Role::Timer",
                "set_live(",
                "Live::",
            ] {
                if line.contains(forbidden) {
                    found.push(format!("{file}:{}: {}", at + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "the panel announces something itself: {found:#?}"
    );
}

/// Two rows can stand for the same system, the same SID and client at two
/// customers. While such a row is selected, the footer says how many rows
/// do; while Return waits for a production system to be picked, the footer
/// says that instead. A row without a twin gets no such sentence.
#[gpui::test]
fn rows_for_the_same_system_are_counted_in_the_footer(cx: &mut TestAppContext) {
    let app = open_app(
        cx,
        &[
            entry("NHP 100")
                .user("N.WEISS")
                .password("sp-nhp-north")
                .field("SAP_HOST", "nhp-north.example")
                .field("SAP_INSTANCE", "00")
                .starred(),
            entry("NHP 100")
                .user("K.BRAUN")
                .password("sp-nhp-south")
                .field("SAP_HOST", "nhp-south.example")
                .field("SAP_INSTANCE", "00")
                .starred(),
            entry("NHQ 100")
                .user("N.WEISS")
                .password("sp-nhq")
                .field("SAP_HOST", "nhq.example")
                .field("SAP_INSTANCE", "00"),
        ],
    );
    let panel = app.open_panel(cx);
    let vm = app.vm(cx);
    let tone = |cx: &mut TestAppContext| vm.read_with(cx, |vm, _| vm.footer().tone);
    let two = FooterTone::Tie(crate::tp!("quick.results.same_matches", 2));

    // The empty search starts on a production system: Return waits, and the
    // footer says so rather than counting.
    assert_eq!(app.selected_title(cx).as_deref(), Some("NHP 100"));
    assert_eq!(tone(cx), FooterTone::ProductionSelectFirst);
    keys(cx, panel, "cmd-1");
    assert_eq!(tone(cx), two, "the picked row has a twin");
    let model = vm.read_with(cx, |vm, _| vm.footer());
    assert_eq!(
        model.primary,
        Some(CommandId::ENTRY_SAP_LOGIN),
        "Return signs in"
    );

    // Typed, both NHP 100 rows come first; each has the other as its twin.
    type_into(cx, panel, "nhp 100");
    assert_eq!(tone(cx), two);
    let text = cx.update(|cx| footer(&vm.read(cx).footer()).description_text());
    let words = crate::tp!("quick.results.same_matches", 2);
    assert!(
        text.contains(words.as_ref()),
        "the selected row's description says it: {text}"
    );
    keys(cx, panel, "down");
    assert_eq!(app.selected_title(cx).as_deref(), Some("NHP 100"));
    assert_eq!(tone(cx), two, "the second NHP 100");

    // NHQ 100 has no twin, also in a list whose first row has one.
    keys(cx, panel, "escape");
    type_into(cx, panel, "100");
    assert_eq!(tone(cx), two);
    keys(cx, panel, "cmd-3");
    assert_eq!(app.selected_title(cx).as_deref(), Some("NHQ 100"));
    assert_eq!(tone(cx), FooterTone::Normal);
}

/// The search field's own Copy and Cut, from a key, the Edit menu or the
/// field's edit menu, would write the search to the pasteboard past the
/// clipboard service. In the panel, Copy copies the password as Cmd+C does,
/// and Cut does nothing.
#[gpui::test]
fn the_search_field_never_writes_its_text_to_the_pasteboard(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let panel = app.open_panel(cx);
    type_into(cx, panel, "nhq");
    keys(cx, panel, "cmd-a cmd-x");
    assert_eq!(cx.read_from_clipboard(), None, "Cut wrote the search");
    assert_eq!(app.query(cx), "nhq", "Cut took the search away");

    // As the menus send them.
    cx.dispatch_action(panel, gpui_component::input::Cut);
    cx.run_until_parked();
    assert_eq!(cx.read_from_clipboard(), None, "Cut wrote the search");
    cx.dispatch_action(panel, gpui_component::input::Copy);
    cx.run_until_parked();
    assert_eq!(cx.read_from_clipboard(), None, "Copy wrote the search");
    assert_eq!(
        app.copied(),
        (1, Some("sp-nhq".to_string())),
        "Copy copies the password, through the clipboard service"
    );
    assert_eq!(app.query(cx), "nhq");
}

#[gpui::test]
fn a_copy_marks_the_entry_as_recently_used(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let panel = app.open_panel(cx);
    type_into(cx, panel, "webmail");
    keys(cx, panel, "cmd-c");
    assert_eq!(app.recently_used(cx), ["Webmail"]);
    keys(cx, panel, "escape");
    type_into(cx, panel, "vpn");
    keys(cx, panel, "cmd-shift-c");
    assert_eq!(app.recently_used(cx), ["Nordhafen VPN", "Webmail"]);
}

#[test]
fn the_panel_never_touches_the_pasteboard() {
    let import = "use crate::app::services::clipboard::ClipboardService;";
    let mut found = Vec::new();
    let mut secret_writes = 0;
    for (file, code) in panel_sources() {
        for (at, line) in code.lines().enumerate() {
            let line = line.trim();
            for forbidden in [
                "write_to_clipboard",
                "read_from_clipboard",
                "ClipboardItem",
                "Pasteboard",
                "write_text",
                "set_clipboard",
            ] {
                if line.contains(forbidden) {
                    found.push(format!("{file}:{}: {line}", at + 1));
                }
            }
            // The one way in is the clipboard service's import. The setting
            // for the clear timer is the only other name with the word.
            let rest = line.replace("clipboard_clear_secs", "");
            if rest.contains("clipboard") && line != import {
                found.push(format!("{file}:{}: {line}", at + 1));
            }
            secret_writes += line.matches("write_secret(").count();
        }
    }
    assert!(
        found.is_empty(),
        "the panel reaches the pasteboard: {found:#?}"
    );
    assert_eq!(secret_writes, 1, "every copy goes through one write_secret");
}

#[gpui::test]
fn only_http_and_https_open(cx: &mut TestAppContext) {
    let app = open_app(
        cx,
        &[
            entry("Script portal").url("javascript:alert(1)"),
            entry("Local file").url("file:///etc/passwd"),
            entry("Plain portal").url("https://portal.example/login"),
        ],
    );
    for title in ["Script portal", "Local file"] {
        let panel = app.open_panel(cx);
        if !app.query(cx).is_empty() {
            keys(cx, panel, "escape");
        }
        type_into(cx, panel, title);
        assert_eq!(app.selected_title(cx).as_deref(), Some(title));
        keys(cx, panel, "enter");
        assert_eq!(cx.opened_url(), None, "{title} opened");
        assert!(panel_is_open(cx), "{title}: the panel stays open");
        let note = app.vm(cx).read_with(cx, |vm, _| vm.footer().note);
        assert_eq!(note, Some(tr!("quick.error.website_failed")));
        cx.update(close);
        cx.run_until_parked();
    }
    let panel = app.open_panel(cx);
    keys(cx, panel, "escape");
    type_into(cx, panel, "plain portal");
    keys(cx, panel, "enter");
    assert_eq!(
        cx.opened_url().as_deref(),
        Some("https://portal.example/login")
    );
}

/// The panel never activates the app. It makes its own window key in one
/// place, after the Touch ID dialog took that from it (`take_key` in
/// `locked.rs`); that brings the panel forward, not the app.
#[test]
fn the_panel_never_activates_the_app() {
    let mut found = Vec::new();
    let mut shown = 0;
    let mut key_again = 0;
    for (file, code) in panel_sources() {
        let answered = code.find("fn take_key(").map(|start| {
            let body = probe::braced(&code[start..]);
            let at = body.as_ptr() as usize - code.as_ptr() as usize;
            at..at + body.len()
        });
        let mut offset = 0;
        for (at, line) in code.lines().enumerate() {
            let range = offset..offset + line.len();
            offset += line.len() + 1;
            for forbidden in [
                "activate(",
                "activate_window(",
                "request_window_close",
                "cx.hide(",
            ] {
                if !line.contains(forbidden) {
                    continue;
                }
                let in_answered = answered
                    .as_ref()
                    .is_some_and(|body| body.start <= range.start && range.end <= body.end);
                if forbidden == "activate_window(" && in_answered && file.ends_with("locked.rs") {
                    key_again += line.matches(forbidden).count();
                    continue;
                }
                found.push(format!("{file}:{}: {}", at + 1, line.trim()));
            }
        }
        let calls = code.matches("show_main_window").count();
        if calls > 0 {
            let start = code
                .find("fn show_in_app(")
                .unwrap_or_else(|| panic!("{file} shows the main window outside show_in_app"));
            let body = probe::braced(&code[start..]);
            assert_eq!(
                body.matches("show_main_window").count(),
                calls,
                "{file} shows the main window outside show_in_app"
            );
            shown += calls;
        }
    }
    assert!(found.is_empty(), "the panel activates the app: {found:#?}");
    assert_eq!(shown, 1, "Cmd+O is the one way to the main window");
    assert_eq!(
        key_again, 1,
        "the panel takes key status back once, after Touch ID"
    );
}

//! The panel as the app opens it: the main window over a test vault, then the
//! panel as a second window, driven by its keys and actions.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use ::keepass::db::{Database, fields};
use gpui::{AnyWindowHandle, Entity, RenderOnce, TestAppContext, WindowKind};
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

    fn starred(self) -> Self {
        Self {
            starred: true,
            ..self
        }
    }

    fn field(mut self, key: &'static str, value: &'static str) -> Self {
        self.fields.push((key, value));
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
        state.open_for_test(PathBuf::from(VAULT), document(entries));
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
    fn id(&self, cx: &mut TestAppContext, title: &str) -> String {
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

fn keys(cx: &mut TestAppContext, panel: AnyWindowHandle, keystrokes: &str) {
    cx.simulate_keystrokes(panel, keystrokes);
    cx.run_until_parked();
}

/// A vault with two SAP systems, NHQ 100 and NHP 100, both signing in as
/// N.WEISS through SAP GUI, a favorite web login, a remote access entry and
/// a web login of their own.
fn sample() -> Vec<Fixture> {
    vec![
        entry("NHQ 100")
            .user("N.WEISS")
            .password("sp-nhq")
            .field("SAP_HOST", "nhq.example")
            .field("SAP_INSTANCE", "00"),
        entry("NHP 100")
            .user("N.WEISS")
            .password("sp-nhp")
            .field("SAP_HOST", "nhp.example")
            .field("SAP_INSTANCE", "00"),
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
    let note = |cx: &mut TestAppContext| vm.read_with(cx, |vm, _| vm.footer().note);
    assert_eq!(note(cx), None, "no other vault, no note");
    vm.update(cx, |vm, _| vm.assume_parked_vaults(2));
    assert_eq!(note(cx), Some(tr!("quick.footer.other_vaults")));
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

/// The panel's own files, without this one, which names what it looks for.
fn panel_sources() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in ["src/ui/windows", "src/ui/vm"] {
        probe::rust_files(&root.join(dir), &mut files);
    }
    let sources: Vec<(String, String)> = files
        .into_iter()
        .filter(|file| !file.ends_with("panel/tests.rs"))
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
        cx.update(|cx| ClipboardService::handle(cx).read(cx).has_pending()),
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
    assert_eq!(model.primary, Some(CommandId::ENTRY_SAP_LOGIN), "Return signs in");

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

#[test]
fn the_panel_never_activates_the_app() {
    let mut found = Vec::new();
    let mut shown = 0;
    for (file, code) in panel_sources() {
        for (at, line) in code.lines().enumerate() {
            for forbidden in [
                "activate(",
                "activate_window(",
                "request_window_close",
                "cx.hide(",
            ] {
                if line.contains(forbidden) {
                    found.push(format!("{file}:{}: {}", at + 1, line.trim()));
                }
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
}

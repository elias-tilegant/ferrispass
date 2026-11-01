//! What the list selects on its own, and what the user picked by hand.
//!
//! Every new search starts on its first row. Only `select_entry` makes a
//! choice explicit, and only in the list it was made in: rebuilding the list,
//! parking the vault or opening it again makes the selection implicit.
use super::*;
use crate::app::kinds;
use crate::keepass::FAVORITE_TAG;
use crate::keepass::repository::snapshot_from_database;
use gpui::{Entity, TestAppContext};
use keepass::db::{GroupMut, fields};

/// The parent folder does not exist, so nothing could ever be written here.
const VAULT: &str = "/nonexistent/ferrispass-search-tests/sample.kdbx";
const OTHER_VAULT: &str = "/nonexistent/ferrispass-search-tests/other.kdbx";

fn add_entry(group: &mut GroupMut<'_>, title: &str, starred: bool) {
    let mut entry = group.add_entry();
    entry.set_unprotected(fields::TITLE, title);
    if starred {
        entry.tags.push(FAVORITE_TAG.to_string());
    }
}

/// Two customers with the same production system, NHP 100, and a favorite
/// of the user's own. NHP 100 PRD is the first row of the Nordhafen group.
fn sample_database() -> Database {
    let mut database = Database::new();
    {
        let mut root = database.root_mut();
        add_entry(&mut root, "Salesforce", true);
        {
            let mut nordhafen = root.add_group();
            nordhafen.name = "Nordhafen".into();
            add_entry(&mut nordhafen, "NHP 100 PRD", false);
            add_entry(&mut nordhafen, "NHQ 100 QAS", false);
            add_entry(&mut nordhafen, "Nordhafen alt (NHP 100)", true);
        }
        {
            let mut brenner = root.add_group();
            brenner.name = "Brenner".into();
            add_entry(&mut brenner, "NHP 100", false);
            add_entry(&mut brenner, "Brenner VPN", false);
        }
    }
    database
}

fn document(database: Database) -> VaultDocument {
    let snapshot = snapshot_from_database(&database);
    VaultDocument::new(database, snapshot, "pw".to_string(), None)
}

fn open(cx: &mut TestAppContext, database: Database) -> Entity<AppState> {
    cx.new(|_| {
        let mut state = AppState::default();
        state.open_for_test(PathBuf::from(VAULT), document(database));
        state
    })
}

fn snapshot(state: &AppState) -> &VaultSnapshot {
    let VaultStatus::Open { document, .. } = &state.vault else {
        panic!("no vault is open");
    };
    document.snapshot()
}

fn id_of(state: &AppState, title: &str) -> String {
    snapshot(state)
        .live_entries()
        .into_iter()
        .find(|entry| entry.title == title)
        .map(|entry| entry.id.clone())
        .unwrap_or_else(|| panic!("no entry titled {title:?}"))
}

fn group_id(state: &AppState, name: &str) -> String {
    snapshot(state)
        .root
        .groups
        .iter()
        .find(|group| group.name == name)
        .map(|group| group.id.clone())
        .unwrap_or_else(|| panic!("no group named {name:?}"))
}

fn title_of(state: &AppState, id: Option<&str>) -> Option<String> {
    id.and_then(|id| snapshot(state).find_entry(id))
        .map(|entry| entry.title.clone())
}

/// The row the list shows as selected.
fn shown(state: &AppState) -> Option<String> {
    title_of(state, state.shown_entry_id())
}

/// What `selected_entry_id` holds, which can differ from the row shown.
fn selected(state: &AppState) -> Option<String> {
    let VaultStatus::Open {
        selected_entry_id, ..
    } = &state.vault
    else {
        return None;
    };
    title_of(state, selected_entry_id.as_deref())
}

fn listed(state: &AppState) -> Vec<String> {
    state
        .vault_browser()
        .expect("a vault is open")
        .entries
        .iter()
        .map(|entry| entry.title.clone())
        .collect()
}

fn pick(state: &mut AppState, title: &str, cx: &mut Context<AppState>) {
    let id = id_of(state, title);
    state.select_entry(id, cx);
}

fn show_group(state: &mut AppState, name: &str, cx: &mut Context<AppState>) {
    let id = group_id(state, name);
    state.select_group(id, cx);
}

/// Edits save the vault in the background. Without a session id the save
/// has nowhere to go, so these tests write no file and leave alone the
/// process-wide save flags other tests read.
fn skip_saves(state: &mut AppState) {
    state.active_vault_session_id = None;
}

#[gpui::test]
fn new_query_selects_the_first_row(cx: &mut TestAppContext) {
    let state = open(cx, sample_database());
    state.update(cx, |state, cx| {
        show_group(state, "Nordhafen", cx);
        pick(state, "NHQ 100 QAS", cx);

        state.set_search_query("nhp 100", cx);
        assert_eq!(
            listed(state),
            ["NHP 100", "NHP 100 PRD", "Nordhafen alt (NHP 100)"]
        );
        assert_eq!(selected(state).as_deref(), Some("NHP 100"));
        assert_eq!(shown(state).as_deref(), Some("NHP 100"));
        assert!(!state.selection_is_explicit());

        // The same query again is no new query, and a hand-picked row stays.
        pick(state, "NHP 100 PRD", cx);
        state.set_search_query("nhp 100", cx);
        assert_eq!(selected(state).as_deref(), Some("NHP 100 PRD"));
        assert!(state.selection_is_explicit());
    });
}

#[gpui::test]
fn typing_more_moves_the_selection_to_row_one_even_if_the_old_entry_is_still_listed(
    cx: &mut TestAppContext,
) {
    let state = open(cx, sample_database());
    state.update(cx, |state, cx| {
        state.set_search_query("nhp", cx);
        assert_eq!(
            listed(state),
            ["Nordhafen alt (NHP 100)", "NHP 100", "NHP 100 PRD"]
        );
        pick(state, "NHP 100 PRD", cx);

        state.set_search_query("nhp 100", cx);
        assert!(listed(state).contains(&"NHP 100 PRD".to_string()));
        assert_eq!(
            selected(state).as_deref(),
            Some("NHP 100"),
            "the selection stayed on a row the old query was picked in"
        );
        assert!(!state.selection_is_explicit());
    });
}

#[gpui::test]
fn arrow_or_click_makes_the_selection_explicit(cx: &mut TestAppContext) {
    let state = open(cx, sample_database());
    state.update(cx, |state, cx| {
        state.set_search_query("nhp 100", cx);
        assert!(!state.selection_is_explicit());

        pick(state, "NHP 100 PRD", cx);
        assert_eq!(shown(state).as_deref(), Some("NHP 100 PRD"));
        assert!(state.selection_is_explicit());

        // Picking the row that was already selected counts as well.
        pick(state, "NHP 100", cx);
        assert_eq!(shown(state).as_deref(), Some("NHP 100"));
        assert!(state.selection_is_explicit());
    });
}

#[gpui::test]
fn clearing_the_search_is_implicit_again(cx: &mut TestAppContext) {
    let state = open(cx, sample_database());
    state.update(cx, |state, cx| {
        show_group(state, "Nordhafen", cx);
        state.set_search_query("nhp 100", cx);
        pick(state, "NHP 100 PRD", cx);
        assert!(state.selection_is_explicit());

        // Back in the group, the same production system is the first row,
        // and the app put it there.
        state.clear_search(cx);
        assert_eq!(shown(state).as_deref(), Some("NHP 100 PRD"));
        assert!(!state.selection_is_explicit());

        // Emptying the search field by hand is the same.
        state.set_search_query("nhp 100", cx);
        pick(state, "NHP 100 PRD", cx);
        state.set_search_query("", cx);
        assert_eq!(shown(state).as_deref(), Some("NHP 100 PRD"));
        assert!(!state.selection_is_explicit());
    });
}

#[gpui::test]
fn a_parked_and_restored_selection_is_implicit_again(cx: &mut TestAppContext) {
    let state = open(cx, sample_database());
    state.update(cx, |state, cx| {
        show_group(state, "Nordhafen", cx);
        pick(state, "NHP 100 PRD", cx);
        assert!(state.selection_is_explicit());
        let list_before = state.vault_browser().expect("open").entries;

        state.park_active();
        state.open_for_test(PathBuf::from(OTHER_VAULT), document(Database::new()));
        state.park_active();
        assert!(state.unpark(Path::new(VAULT)));

        // The vault comes back with the very same list and selection, so
        // only the reset on parking tells the two sessions apart.
        let list_after = state.vault_browser().expect("open").entries;
        assert!(Rc::ptr_eq(&list_before, &list_after));
        assert_eq!(shown(state).as_deref(), Some("NHP 100 PRD"));
        assert!(
            !state.selection_is_explicit(),
            "a choice made before parking counted after switching back"
        );
    });
}

/// What a successful unlock puts in the active slot, showing `group` with
/// its first row selected, without what `finish_open_attempt` does to files
/// on this machine (the recent vaults list, the launch folder, the sync
/// settings).
fn finish_open(state: &mut AppState, path: &str, document: VaultDocument, group: &str) {
    assert!(
        matches!(&state.vault, VaultStatus::Opening { path: opening } if opening == Path::new(path))
    );
    let snapshot = document.snapshot();
    let group = snapshot
        .root
        .groups
        .iter()
        .find(|candidate| candidate.name == group)
        .expect("group exists");
    let selection = LibrarySelection::Group(group.id.clone());
    let visible_entries = Rc::new(entries_for_selection(
        snapshot,
        &selection,
        "",
        &HashMap::new(),
    ));
    let selected_entry_id = visible_entries.first().map(|entry| entry.id.clone());
    state.vault = VaultStatus::Open {
        path: PathBuf::from(path),
        document: Box::new(document),
        selection,
        selected_entry_id,
        search_query: String::new(),
        visible_entries,
        selected_strength: None,
        last_used: HashMap::new(),
    };
    state.active_vault_session_id = Some(VaultSessionId::next());
}

#[gpui::test]
fn a_reopened_vault_starts_implicit(cx: &mut TestAppContext) {
    let database = sample_database();
    let state = open(cx, database.clone());
    state.update(cx, |state, cx| {
        show_group(state, "Nordhafen", cx);
        pick(state, "NHP 100 PRD", cx);
        assert!(state.selection_is_explicit());

        state.lock_vault_now();
        assert!(!state.selection_is_explicit());

        state.begin_open(PathBuf::from(VAULT), cx);
        finish_open(state, VAULT, document(database), "Nordhafen");
        assert_eq!(shown(state).as_deref(), Some("NHP 100 PRD"));
        assert!(!state.selection_is_explicit());
        assert!(
            state.explicit_selection.is_none(),
            "a choice from before the lock outlived it"
        );
    });
}

#[gpui::test]
fn a_hidden_explicit_choice_does_not_cover_the_shown_row(cx: &mut TestAppContext) {
    let is_production = |state: &AppState| {
        let id = state.shown_entry_id().expect("a row is shown");
        let entry = snapshot(state).find_entry(id).expect("shown entry exists");
        kinds::classify(entry, None).is_production()
    };
    let state = open(cx, sample_database());
    state.update(cx, |state, cx| {
        // Picked in one list, then a search the entry is not part of.
        show_group(state, "Brenner", cx);
        pick(state, "Brenner VPN", cx);
        assert!(state.selection_is_explicit());
        state.set_search_query("nhp 100", cx);
        assert_eq!(shown(state).as_deref(), Some("NHP 100"));
        assert!(is_production(state));
        assert!(!state.selection_is_explicit());

        // An entry selected while it is not in the list: the id is recorded
        // for this very list, but the first row is what the list shows.
        pick(state, "Brenner VPN", cx);
        assert_eq!(selected(state).as_deref(), Some("Brenner VPN"));
        assert_eq!(shown(state).as_deref(), Some("NHP 100"));
        assert!(is_production(state));
        assert!(
            !state.selection_is_explicit(),
            "the choice of a hidden entry made the production row explicit"
        );

        // Moving the picked entry out of the group leaves its id behind on
        // purpose, and the first row takes its place.
        state.clear_search(cx);
        show_group(state, "Nordhafen", cx);
        pick(state, "NHQ 100 QAS", cx);
        assert!(state.selection_is_explicit());
        let brenner = group_id(state, "Brenner");
        let qas = id_of(state, "NHQ 100 QAS");
        skip_saves(state);
        state.move_entry(&qas, &brenner, cx).expect("moved");
        assert_eq!(selected(state).as_deref(), Some("NHQ 100 QAS"));
        assert_eq!(shown(state).as_deref(), Some("NHP 100 PRD"));
        assert!(is_production(state));
        assert!(!state.selection_is_explicit());
    });
}

#[gpui::test]
fn returning_to_a_group_makes_the_old_choice_implicit(cx: &mut TestAppContext) {
    let state = open(cx, sample_database());
    state.update(cx, |state, cx| {
        show_group(state, "Nordhafen", cx);
        pick(state, "NHP 100 PRD", cx);
        assert!(state.selection_is_explicit());

        show_group(state, "Brenner", cx);
        show_group(state, "Nordhafen", cx);
        assert_eq!(shown(state).as_deref(), Some("NHP 100 PRD"));
        assert!(
            !state.selection_is_explicit(),
            "the first row of a group counted as picked because it was picked last time"
        );
    });
}

#[gpui::test]
fn a_rebuilt_list_makes_the_choice_implicit(cx: &mut TestAppContext) {
    let state = open(cx, sample_database());
    state.update(cx, |state, cx| {
        show_group(state, "Nordhafen", cx);

        // A sync merge rebuilds the list; the row stays where it was.
        pick(state, "NHP 100 PRD", cx);
        assert!(state.selection_is_explicit());
        let VaultStatus::Open { document, .. } = &state.vault else {
            unreachable!("open");
        };
        let merged = document.database().clone();
        let session = state.active_vault_session_id.expect("session");
        assert!(state.replace_database_for_session(Path::new(VAULT), session, merged, cx));
        assert_eq!(shown(state).as_deref(), Some("NHP 100 PRD"));
        assert!(
            !state.selection_is_explicit(),
            "a sync merge kept the choice"
        );

        // Deleting another entry rebuilds it as well.
        skip_saves(state);
        pick(state, "NHP 100 PRD", cx);
        assert!(state.selection_is_explicit());
        let qas = id_of(state, "NHQ 100 QAS");
        state.delete_entry(&qas, cx).expect("deleted");
        assert_eq!(shown(state).as_deref(), Some("NHP 100 PRD"));
        assert!(
            !state.selection_is_explicit(),
            "deleting an entry kept the choice"
        );

        // A new entry is selected by the app, not by the user.
        pick(state, "NHP 100 PRD", cx);
        let nordhafen = group_id(state, "Nordhafen");
        let draft = EntryDraft {
            title: "NHP 200 PRD".to_string(),
            ..EntryDraft::default()
        };
        state.create_entry(&nordhafen, draft, cx).expect("created");
        assert_eq!(shown(state).as_deref(), Some("NHP 200 PRD"));
        assert!(!state.selection_is_explicit());
    });
}

#[gpui::test]
fn two_nones_are_not_explicit(cx: &mut TestAppContext) {
    // No vault at all.
    let state = cx.new(|_| AppState::default());
    state.read_with(cx, |state, _| assert!(!state.selection_is_explicit()));

    // An empty vault: no row is shown and no choice was made.
    let state = open(cx, Database::new());
    state.read_with(cx, |state, _| {
        assert_eq!(state.shown_entry_id(), None);
        assert!(state.explicit_selection.is_none());
        assert!(!state.selection_is_explicit());
    });

    // A search that finds nothing.
    let state = open(cx, sample_database());
    state.update(cx, |state, cx| {
        state.set_search_query("nothing like this", cx);
        assert_eq!(state.shown_entry_id(), None);
        assert!(state.explicit_selection.is_none());
        assert!(!state.selection_is_explicit());
    });
}

#[gpui::test]
fn ranked_order_reaches_the_browser_model(cx: &mut TestAppContext) {
    let state = open(cx, sample_database());
    state.update(cx, |state, cx| {
        state.set_search_query("nhq", cx);
        assert_eq!(listed(state), ["NHQ 100 QAS"]);
        let model = state.vault_browser().expect("open");
        assert_eq!(
            title_of(state, model.selected_entry_id.as_deref()).as_deref(),
            Some("NHQ 100 QAS")
        );

        // Starred and recently used reach the ranking: the favorite first,
        // then the entry just used, then the rest by text.
        let prd = id_of(state, "NHP 100 PRD");
        state.mark_entry_used(&prd);
        state.set_search_query("nhp", cx);
        assert_eq!(
            listed(state),
            ["Nordhafen alt (NHP 100)", "NHP 100 PRD", "NHP 100"]
        );

        state.set_search_query("nhp 100", cx);
        assert_eq!(
            listed(state),
            ["NHP 100 PRD", "NHP 100", "Nordhafen alt (NHP 100)"]
        );
    });
}

/// Every place outside `select_entry` that sets `selected_entry_id` picks a
/// row for the user. Each has to build a new `visible_entries` list in the
/// same function, or a choice the user made in the old list would still
/// count as explicit (`selection_is_explicit` compares list instances). This
/// reads the source of `src/app/state/`, so a new place that forgets the new
/// list fails here.
#[test]
fn every_implicit_selection_site_rebuilds_the_list() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app/state");
    let mut setters: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("src/app/state is readable")
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "rs")
                && !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with("_tests.rs"))
        })
        .collect();
    files.sort();

    for file in &files {
        let source = std::fs::read_to_string(file).expect("source is readable");
        let file_name = file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        for function in functions(&source) {
            let sets_selection = function
                .lines
                .iter()
                .any(|line| assigned_value(line, "selected_entry_id").is_some());
            if !sets_selection {
                continue;
            }
            setters.push(function.name.clone());
            if function.name == "select_entry" {
                continue;
            }
            let rebuilds_list = function.lines.iter().any(|line| {
                assigned_value(line, "visible_entries")
                    .is_some_and(|value| value.starts_with("Rc::new("))
            });
            if !rebuilds_list {
                missing.push(format!("{file_name}: {}", function.name));
            }
        }
    }

    // The scan has to find the places it guards, or it proves nothing.
    for known in [
        "select_entry",
        "select_group",
        "select_library",
        "set_search_query",
        "clear_search",
        "create_entry",
        "create_group",
        "run_entry_mutation",
        "run_group_mutation",
        "replace_database_for_session",
        "finish_open_attempt",
    ] {
        assert!(
            setters.iter().any(|name| name == known),
            "the scan no longer sees `{known}` set selected_entry_id; found {setters:?}"
        );
    }
    assert!(
        missing.is_empty(),
        "these set selected_entry_id without building a new visible_entries list: {missing:?}"
    );
}

struct Function {
    name: String,
    lines: Vec<String>,
}

/// Splits Rust source at every `fn` item. Good enough for these files: a
/// function runs until the next `fn`, and none of them nests one.
fn functions(source: &str) -> Vec<Function> {
    let mut functions: Vec<Function> = Vec::new();
    for line in source.lines() {
        if let Some(name) = fn_name(line) {
            functions.push(Function {
                name,
                lines: Vec::new(),
            });
        } else if let Some(function) = functions.last_mut() {
            function.lines.push(line.to_string());
        }
    }
    functions
}

fn fn_name(line: &str) -> Option<String> {
    let mut rest = line.trim_start();
    if rest.starts_with("//") {
        return None;
    }
    if let Some(after) = rest.strip_prefix("pub") {
        rest = after;
        if rest.starts_with('(') {
            rest = &rest[rest.find(')')? + 1..];
        }
        rest = rest.trim_start();
    }
    for qualifier in ["const ", "async ", "unsafe "] {
        rest = rest.strip_prefix(qualifier).unwrap_or(rest);
    }
    let name: String = rest
        .strip_prefix("fn ")?
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    (!name.is_empty()).then_some(name)
}

/// The value assigned to `field` on this line, if the line assigns it:
/// `field = value`, `*field = value`, `x.field = value` or
/// `let field = value`. A comparison (`==`) is not an assignment.
fn assigned_value<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let code = line.split("//").next().unwrap_or("");
    code.match_indices(field).find_map(|(at, _)| {
        let before = code[..at].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            return None;
        }
        let after = code[at + field.len()..].trim_start();
        let value = after.strip_prefix('=')?;
        (!value.starts_with('=')).then(|| value.trim_start())
    })
}

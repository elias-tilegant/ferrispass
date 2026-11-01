//! `entries_for_selection` decides what every list in the app shows.
//! It had no coverage, and shipped for months returning deleted entries
//! from All items, Favorites, the tag filters and the root group.
use super::*;
use crate::domain::{VaultEntry, VaultGroup, VaultSnapshot};

/// Every entry shares the "acme" token so a search query reaches all
/// three and the filter is what decides the result, not the ranker.
fn entry(id: &str, trashed: bool) -> VaultEntry {
    VaultEntry {
        id: id.to_string(),
        title: format!("acme {id}"),
        starred: true,
        has_otp: true,
        tags: vec!["Work".to_string()],
        in_recycle_bin: trashed,
        ..VaultEntry::default()
    }
}

/// root -> [live entry "keep", bin -> [entry "direct", deleted group ->
/// entry "nested"]]. The nested entry is the one a `delete_group` produces.
fn snapshot() -> VaultSnapshot {
    let deleted_group = VaultGroup {
        id: "deleted-group".to_string(),
        name: "Deleted".to_string(),
        entries: vec![entry("nested", true)],
        in_recycle_bin: true,
        ..VaultGroup::default()
    };
    let bin = VaultGroup {
        id: "bin".to_string(),
        name: "Recycle Bin".to_string(),
        groups: vec![deleted_group],
        entries: vec![entry("direct", true)],
        in_recycle_bin: true,
        ..VaultGroup::default()
    };
    let root = VaultGroup {
        id: "root".to_string(),
        name: "Root".to_string(),
        groups: vec![bin],
        entries: vec![entry("keep", false)],
        ..VaultGroup::default()
    };
    VaultSnapshot {
        recycle_bin_id: Some("bin".to_string()),
        ..VaultSnapshot::new(root)
    }
}

fn ids(selection: &LibrarySelection) -> Vec<String> {
    let last_used = HashMap::new();
    entries_for_selection(&snapshot(), selection, "", &last_used)
        .into_iter()
        .map(|entry| entry.id)
        .collect()
}

#[test]
fn every_live_selection_hides_deleted_entries() {
    for selection in [
        LibrarySelection::Group("root".to_string()),
        LibrarySelection::AllItems,
        LibrarySelection::Favorites,
        LibrarySelection::TotpEnabled,
        LibrarySelection::Tag("Work".to_string()),
    ] {
        assert_eq!(
            ids(&selection),
            vec!["keep".to_string()],
            "{selection:?} must not list entries from the Recycle Bin"
        );
    }
}

#[test]
fn trash_lists_the_whole_bin_subtree() {
    let mut listed = ids(&LibrarySelection::Trash);
    listed.sort();
    assert_eq!(
        listed,
        vec!["direct".to_string(), "nested".to_string()],
        "an entry inside a deleted group must be reachable from Trash"
    );
}

#[test]
fn search_does_not_reach_into_the_bin() {
    let last_used = HashMap::new();
    let hits = entries_for_selection(&snapshot(), &LibrarySelection::AllItems, "acme", &last_used);
    let ids: Vec<&str> = hits.iter().map(|entry| entry.id.as_str()).collect();
    assert_eq!(ids, vec!["keep"], "search is a live-only list");
}

#[test]
fn recently_used_hides_a_deleted_entry_it_remembers() {
    // Copy the password, then delete the entry: the session log still
    // holds the id, but the row must not come back.
    let mut last_used = HashMap::new();
    last_used.insert("nested".to_string(), Local::now());
    last_used.insert("keep".to_string(), Local::now());
    let listed =
        entries_for_selection(&snapshot(), &LibrarySelection::RecentlyUsed, "", &last_used);
    let ids: Vec<&str> = listed.iter().map(|entry| entry.id.as_str()).collect();
    assert_eq!(ids, vec!["keep"]);
}

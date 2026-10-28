//! A vault this app wrote, against the same vault after a KeePassXC save.
//!
//! KeePassXC changes a few things on every save without moving a
//! modification time: it adds `_LAST_MODIFIED` to the plugin data of entries
//! and groups, drops the item times there, and writes its default Auto-Type
//! block onto entries that had none. Read as edits, they turned every entry
//! and group that nobody had touched into a conflict.
//!
//! `fixtures/keepassxc-save/ferrispass.kdbx` was written by this app. Each
//! `keepassxc-<version>.kdbx` is that file after `keepassxc-cli edit` of
//! that release changed the username of "Live/Kept Entry" to "carol" and
//! saved. Nothing else was touched.

use std::collections::HashMap;

use ferrispass::keepass::merge::{self, Resolutions, Side};
use keepass::{
    Database, DatabaseKey,
    db::{CustomDataItem, fields},
};

/// Generated for these fixtures and used nowhere else.
const PASSWORD: &str = "cmTWKQxX2yP1mqh/xwQZqM39";

const KEEPASSXC_SAVES: [&str; 3] = [
    "keepassxc-2.6.6.kdbx",
    "keepassxc-2.7.12.kdbx",
    "keepassxc-2.8.0-beta1.kdbx",
];

fn open(name: &str) -> Database {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/keepassxc-save")
        .join(name);
    let mut file =
        std::fs::File::open(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    Database::open(&mut file, DatabaseKey::new().with_password(PASSWORD))
        .unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn entry<'a>(db: &'a Database, title: &str) -> keepass::db::EntryRef<'a> {
    db.iter_all_entries()
        .find(|entry| entry.get(fields::TITLE) == Some(title))
        .unwrap_or_else(|| panic!("no entry {title}"))
}

#[test]
fn a_keepassxc_save_asks_nothing_about_what_it_did_not_change() {
    let ours = open("ferrispass.kdbx");
    for name in KEEPASSXC_SAVES {
        let theirs = open(name);
        for (local, remote, edited) in [
            (&ours, &theirs, Side::Remote),
            (&theirs, &ours, Side::Local),
        ] {
            let report = merge::diff(local, remote);
            assert!(
                report.conflicts.is_empty(),
                "{name}: {:?}",
                report.conflicts
            );
            assert!(
                report.group_conflicts.is_empty(),
                "{name}: {:?}",
                report.group_conflicts
            );
            assert!(
                report.metadata_conflict.is_none(),
                "{name}: {:?}",
                report.metadata_conflict
            );
            assert_eq!(
                report
                    .auto_resolved
                    .iter()
                    .map(|resolved| (resolved.remote.title.as_str(), resolved.winner))
                    .collect::<Vec<_>>(),
                vec![("Kept Entry", edited)],
                "{name}: the one entry KeePassXC changed is taken from where it changed"
            );

            let merged = merge::apply_picks(local, remote, &Resolutions::default(), &report)
                .unwrap_or_else(|err| panic!("{name}: {err}"));
            assert_eq!(
                entry(&merged, "Kept Entry").get(fields::USERNAME),
                Some("carol"),
                "{name}: the edit lands"
            );

            // The edited entry comes from the save. Every other entry and
            // group kept its modification time, so the two copies were
            // united: nothing either of them wrote is lost.
            for ours_entry in ours.iter_all_entries() {
                let title = ours_entry.get(fields::TITLE).unwrap_or_default();
                let what = format!("{name}: entry {title}");
                let theirs_entry = theirs.entry(ours_entry.id()).expect("in both");
                let merged_entry = merged.entry(ours_entry.id()).expect("merged");
                assert_united(
                    &what,
                    &merged_entry.custom_data,
                    &ours_entry.custom_data,
                    &theirs_entry.custom_data,
                );
                assert_eq!(
                    merged_entry.autotype, theirs_entry.autotype,
                    "{what}: the Auto-Type block stays"
                );
                assert_eq!(
                    merged_entry.times.last_modification, theirs_entry.times.last_modification,
                    "{what}: no modification time moves"
                );
            }
            for ours_group in ours.iter_all_groups() {
                let what = format!("{name}: group {:?}", ours_group.name);
                let theirs_group = theirs.group(ours_group.id()).expect("in both");
                let merged_group = merged.group(ours_group.id()).expect("merged");
                assert_united(
                    &what,
                    &merged_group.custom_data,
                    &ours_group.custom_data,
                    &theirs_group.custom_data,
                );
            }
        }
    }
}

/// Every plugin data item this app wrote, item time included, and every key
/// KeePassXC added, with its value.
fn assert_united(
    what: &str,
    merged: &HashMap<String, CustomDataItem>,
    ours: &HashMap<String, CustomDataItem>,
    theirs: &HashMap<String, CustomDataItem>,
) {
    for (key, item) in ours {
        assert_eq!(
            merged.get(key),
            Some(item),
            "{what}: {key} as this app wrote it"
        );
    }
    for (key, item) in theirs {
        assert_eq!(
            merged.get(key).map(|merged| &merged.value),
            Some(&item.value),
            "{what}: {key} as KeePassXC wrote it"
        );
    }
}

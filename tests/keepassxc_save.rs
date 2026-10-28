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

use ferrispass::keepass::merge::{self, Resolutions, Side};
use keepass::{Database, DatabaseKey, db::fields};

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
            let untouched = entry(&merged, "Karte");
            assert!(
                untouched.custom_data.contains_key("_LAST_MODIFIED"),
                "{name}: what KeePassXC added is kept"
            );
            assert!(
                untouched
                    .custom_data
                    .values()
                    .any(|item| item.last_modification_time.is_some()),
                "{name}: and so is the item time this app wrote"
            );
            assert!(
                untouched.autotype.is_some(),
                "{name}: and the Auto-Type block"
            );
        }
    }
}

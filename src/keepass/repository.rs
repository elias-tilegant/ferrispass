use crate::domain::{VaultEntry, VaultGroup, VaultSnapshot};
use keepass::{
    Database, DatabaseKey,
    db::{DatabaseOpenError, EntryRef, GroupRef},
};
use std::{fs::File, path::Path};

pub struct KeePassRepository;

impl KeePassRepository {
    pub fn open_with_password(
        path: impl AsRef<Path>,
        password: &str,
    ) -> Result<VaultSnapshot, DatabaseOpenError> {
        let mut file = File::open(path)?;
        let key = DatabaseKey::new().with_password(password);
        let database = Database::open(&mut file, key)?;

        Ok(snapshot_from_database(&database))
    }
}

fn snapshot_from_database(database: &Database) -> VaultSnapshot {
    VaultSnapshot::new(group_from_ref(&database.root()))
}

fn group_from_ref(group: &GroupRef<'_>) -> VaultGroup {
    let mut groups = group
        .groups()
        .map(|child| group_from_ref(&child))
        .collect::<Vec<_>>();
    groups.sort_by_key(|child| child.name.to_lowercase());

    let mut entries = group
        .entries()
        .map(|entry| entry_from_ref(&entry))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.title.to_lowercase());

    VaultGroup::new(
        group.id().to_string(),
        non_empty(&group.name, "Root"),
        groups,
        entries,
    )
}

fn entry_from_ref(entry: &EntryRef<'_>) -> VaultEntry {
    VaultEntry::new(
        entry.id().to_string(),
        non_empty(entry.get_title().unwrap_or_default(), "Untitled"),
        entry.get_username().unwrap_or_default(),
        entry.get_url().unwrap_or_default(),
        entry
            .get_password()
            .is_some_and(|password| !password.is_empty()),
    )
}

fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

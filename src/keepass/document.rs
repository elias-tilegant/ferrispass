use crate::domain::VaultSnapshot;
use keepass::Database;
use std::fmt;

pub struct VaultDocument {
    database: Database,
    snapshot: VaultSnapshot,
}

impl VaultDocument {
    pub fn new(database: Database, snapshot: VaultSnapshot) -> Self {
        Self { database, snapshot }
    }

    pub fn snapshot(&self) -> &VaultSnapshot {
        &self.snapshot
    }

    pub fn password_for_entry(&self, entry_id: &str) -> Option<String> {
        self.database
            .iter_all_entries()
            .find(|entry| entry.id().to_string() == entry_id)
            .and_then(|entry| entry.get_password().map(ToOwned::to_owned))
            .filter(|password| !password.is_empty())
    }
}

impl fmt::Debug for VaultDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VaultDocument")
            .field("snapshot", &self.snapshot)
            .finish_non_exhaustive()
    }
}

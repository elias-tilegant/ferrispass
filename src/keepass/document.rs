use crate::domain::VaultSnapshot;
use keepass::Database;
use std::{fmt, sync::Arc};

pub struct VaultDocument {
    database: Database,
    snapshot: Arc<VaultSnapshot>,
}

impl VaultDocument {
    pub fn new(database: Database, snapshot: VaultSnapshot) -> Self {
        Self {
            database,
            snapshot: Arc::new(snapshot),
        }
    }

    pub fn snapshot(&self) -> &VaultSnapshot {
        &self.snapshot
    }

    /// Cheap O(1) clone of the snapshot — used by hot render paths to avoid the
    /// expensive deep-clone of the group tree + every entry. `Arc` (not `Rc`) so
    /// the document can be built on a background thread before being handed to UI.
    pub fn snapshot_rc(&self) -> Arc<VaultSnapshot> {
        Arc::clone(&self.snapshot)
    }

    pub fn password_for_entry(&self, entry_id: &str) -> Option<String> {
        self.database
            .iter_all_entries()
            .find(|entry| entry.id().to_string() == entry_id)
            .and_then(|entry| entry.get_password().map(ToOwned::to_owned))
            .filter(|password| !password.is_empty())
    }

    /// Run the real zxcvbn estimator against the entry's stored password.
    /// Returns `None` when the entry has no password (or doesn't exist).
    /// Computed lazily — typically ~1-5 ms for a 12-24 char password — so we only
    /// call it for the currently-selected entry rather than during snapshot build.
    pub fn strength_for_entry(&self, entry_id: &str) -> Option<StrengthReport> {
        let password = self.password_for_entry(entry_id)?;
        let length = password.chars().count();
        let entropy = zxcvbn::zxcvbn(&password, &[]);
        let score = entropy.score() as u8;
        let bits = (entropy.guesses() as f64).log2().round().max(0.0) as u32;
        let strength = match score {
            0 | 1 => crate::domain::Strength::Weak,
            2 => crate::domain::Strength::Fair,
            _ => crate::domain::Strength::Strong,
        };
        Some(StrengthReport {
            strength,
            length,
            bits,
            score,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StrengthReport {
    pub strength: crate::domain::Strength,
    pub length: usize,
    pub bits: u32,
    pub score: u8,
}

impl fmt::Debug for VaultDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VaultDocument")
            .field("snapshot", &self.snapshot)
            .finish_non_exhaustive()
    }
}

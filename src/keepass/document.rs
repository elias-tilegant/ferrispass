use crate::domain::VaultSnapshot;
use crate::keepass::repository::{find_entry_id, find_group_id, snapshot_from_database};
use keepass::{
    Database, DatabaseKey,
    db::fields,
};
use std::{
    fmt, fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;

pub struct VaultDocument {
    database: Database,
    snapshot: Arc<VaultSnapshot>,
    /// Master password kept in memory so we can rebuild the `DatabaseKey` for
    /// every save. The decrypted entries are already in memory anyway, so
    /// holding the password here is no worse than the existing exposure.
    password: String,
    /// Optional key-file path. We re-read the file on each save (rather than
    /// caching its contents) so that if the user rotates the key file outside
    /// the app, the next save uses the current bytes.
    keyfile_path: Option<PathBuf>,
}

impl VaultDocument {
    pub fn new(
        database: Database,
        snapshot: VaultSnapshot,
        password: String,
        keyfile_path: Option<PathBuf>,
    ) -> Self {
        Self {
            database,
            snapshot: Arc::new(snapshot),
            password,
            keyfile_path,
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

    /// Build a self-contained payload that can be sent to a background thread
    /// for an atomic save. We clone the database here (cheap relative to the
    /// Argon2 KDF that runs inside `Database::save`) so the foreground keeps
    /// full ownership of the live document and stays responsive while save runs.
    pub fn save_payload(&self) -> SavePayload {
        SavePayload {
            database: self.database.clone(),
            password: self.password.clone(),
            keyfile_path: self.keyfile_path.clone(),
        }
    }

    /// Create a new entry under `group_id` (the stringified `GroupId`), apply
    /// the draft fields, refresh the cached snapshot, and return the new
    /// entry's id. Caller is expected to schedule a background save afterwards.
    pub fn create_entry(
        &mut self,
        group_id_str: &str,
        draft: &EntryDraft,
    ) -> Result<String, MutationError> {
        let group_id = find_group_id(&self.database, group_id_str)
            .ok_or(MutationError::GroupNotFound)?;
        let mut group = self
            .database
            .group_mut(group_id)
            .ok_or(MutationError::GroupNotFound)?;
        let mut entry = group.add_entry();
        apply_draft_to_entry(&mut entry, draft);
        let id = entry.id().to_string();
        // Force the borrows to drop before we touch `self` again.
        drop(entry);
        drop(group);
        self.refresh_snapshot();
        Ok(id)
    }

    pub fn update_entry(
        &mut self,
        entry_id_str: &str,
        draft: &EntryDraft,
    ) -> Result<(), MutationError> {
        let entry_id = find_entry_id(&self.database, entry_id_str)
            .ok_or(MutationError::EntryNotFound)?;
        let mut entry = self
            .database
            .entry_mut(entry_id)
            .ok_or(MutationError::EntryNotFound)?;
        apply_draft_to_entry(&mut entry, draft);
        drop(entry);
        self.refresh_snapshot();
        Ok(())
    }

    /// Move an entry to the database's Recycle Bin (creating one if missing).
    /// We deliberately don't expose hard-delete from this API yet — that lives
    /// behind the future "Empty trash" affordance in the Trash sidebar view.
    pub fn delete_entry(&mut self, entry_id_str: &str) -> Result<(), MutationError> {
        let entry_id = find_entry_id(&self.database, entry_id_str)
            .ok_or(MutationError::EntryNotFound)?;
        let recycle_bin_id = self.ensure_recycle_bin();
        let mut entry = self
            .database
            .entry_mut(entry_id)
            .ok_or(MutationError::EntryNotFound)?;
        entry
            .move_to(recycle_bin_id)
            .map_err(|_| MutationError::RecycleBinUnavailable)?;
        drop(entry);
        self.refresh_snapshot();
        Ok(())
    }

    /// Returns the recycle-bin group id, creating one under the root if the
    /// database doesn't already have one set in `meta.recyclebin_uuid`.
    fn ensure_recycle_bin(&mut self) -> keepass::db::GroupId {
        if let Some(g) = self.database.recycle_bin() {
            return g.id();
        }
        let mut root = self.database.root_mut();
        let mut bin = root.add_group();
        bin.name = "Recycle Bin".to_string();
        let id = bin.id();
        drop(bin);
        drop(root);
        self.database.meta.recyclebin_uuid = Some(id.uuid());
        id
    }

    fn refresh_snapshot(&mut self) {
        self.snapshot = Arc::new(snapshot_from_database(&self.database));
    }
}

/// Field bundle for `create_entry` / `update_entry`. Empty fields are skipped
/// so the entry doesn't accumulate empty-string values for never-touched keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryDraft {
    pub title: String,
    pub username: String,
    pub password: String,
    pub url: String,
    pub notes: String,
    pub tags: Vec<String>,
}

fn apply_draft_to_entry<E>(entry: &mut E, draft: &EntryDraft)
where
    E: std::ops::DerefMut<Target = keepass::db::Entry>,
{
    set_or_clear_unprotected(entry, fields::TITLE, &draft.title);
    set_or_clear_unprotected(entry, fields::USERNAME, &draft.username);
    set_or_clear_unprotected(entry, fields::URL, &draft.url);
    set_or_clear_unprotected(entry, "Notes", &draft.notes);
    if draft.password.is_empty() {
        // Clear by writing empty protected value.
        entry.set_protected(fields::PASSWORD, "");
    } else {
        entry.set_protected(fields::PASSWORD, draft.password.clone());
    }
    entry.tags = draft.tags.clone();
    entry.times.last_modification = Some(keepass::db::Times::now());
}

fn set_or_clear_unprotected<E>(entry: &mut E, key: &str, value: &str)
where
    E: std::ops::DerefMut<Target = keepass::db::Entry>,
{
    if value.is_empty() {
        entry.set_unprotected(key, "");
    } else {
        entry.set_unprotected(key, value.to_string());
    }
}

#[derive(Debug, Error)]
pub enum MutationError {
    #[error("group not found")]
    GroupNotFound,
    #[error("entry not found")]
    EntryNotFound,
    #[error("recycle bin is unavailable in this database")]
    RecycleBinUnavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StrengthReport {
    pub strength: crate::domain::Strength,
    pub length: usize,
    pub bits: u32,
    pub score: u8,
}

/// A snapshot of the document's state suitable for an off-thread save.
/// Holds enough material (cloned `Database` + key sources) to rebuild the
/// `DatabaseKey` and serialize without touching the live document.
pub struct SavePayload {
    database: Database,
    password: String,
    keyfile_path: Option<PathBuf>,
}

impl SavePayload {
    /// Atomically write the database to `target_path`. Writes to `<target>.tmp`,
    /// fsyncs, then renames over the target so a crash mid-write can never
    /// leave a half-written `.kdbx`.
    pub fn save_to(self, target_path: &Path) -> Result<(), SaveError> {
        let mut key = DatabaseKey::new();
        if !self.password.is_empty() {
            key = key.with_password(&self.password);
        }
        if let Some(kf) = &self.keyfile_path {
            let mut kf_handle = fs::File::open(kf).map_err(SaveError::ReadKeyfile)?;
            key = key.with_keyfile(&mut kf_handle).map_err(SaveError::Keyfile)?;
        }

        let tmp_path = temp_path_for(target_path);
        // Scope the file handle so it's flushed + dropped before rename.
        {
            let mut tmp = fs::File::create(&tmp_path).map_err(SaveError::CreateTemp)?;
            self.database.save(&mut tmp, key).map_err(|e| {
                // Best-effort cleanup: leave no orphaned .tmp behind on failure.
                let _ = fs::remove_file(&tmp_path);
                SaveError::Encode(e.to_string())
            })?;
            tmp.flush().map_err(SaveError::WriteTemp)?;
            tmp.sync_all().map_err(SaveError::WriteTemp)?;
        }
        fs::rename(&tmp_path, target_path).map_err(SaveError::Rename)?;
        Ok(())
    }
}

fn temp_path_for(target: &Path) -> PathBuf {
    // `target.kdbx.tmp` keeps the temp file next to the destination so the
    // rename is on the same filesystem (atomic on POSIX/macOS).
    let mut buf = target.as_os_str().to_owned();
    buf.push(".tmp");
    PathBuf::from(buf)
}

#[derive(Debug, Error)]
pub enum SaveError {
    #[error("could not open key file: {0}")]
    ReadKeyfile(#[source] io::Error),

    #[error("could not read key file: {0}")]
    Keyfile(#[source] io::Error),

    #[error("could not create temp file for save: {0}")]
    CreateTemp(#[source] io::Error),

    #[error("could not write temp file: {0}")]
    WriteTemp(#[source] io::Error),

    #[error("could not rename temp file over target: {0}")]
    Rename(#[source] io::Error),

    #[error("could not encode database: {0}")]
    Encode(String),
}

impl fmt::Debug for VaultDocument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VaultDocument")
            .field("snapshot", &self.snapshot)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::VaultGroup;
    use keepass::{Database, db::fields};
    use tempfile::TempDir;

    /// Open → save → re-open round-trip on a freshly-built in-memory database.
    /// Verifies that:
    /// 1. `Database::save` actually writes a valid kdbx (i.e. the
    ///    `save_kdbx4` feature is enabled in keepass-rs).
    /// 2. The atomic temp+rename leaves the target intact and parseable.
    /// 3. The password we cache reconstructs into a working `DatabaseKey`.
    #[test]
    fn save_payload_round_trip() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("roundtrip.kdbx");

        // Build a tiny database in memory with one entry so we have something
        // recognisable on the other side of the round-trip.
        let mut db = Database::new();
        let mut root = db.root_mut();
        let mut entry = root.add_entry();
        entry.set_unprotected(fields::TITLE, "Roundtrip");
        entry.set_unprotected(fields::USERNAME, "alice");
        entry.set_protected(fields::PASSWORD, "hunter2");
        let entry_id = entry.id().to_string();

        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let doc = VaultDocument::new(db, snapshot, "vault-pw".to_string(), None);

        // Use the public payload API exactly the way `save_async` does on the
        // real path — so this test catches regressions in the same code path.
        doc.save_payload()
            .save_to(&path)
            .expect("first save succeeds");

        // No leftover .tmp from a successful save.
        let tmp_path = path.with_extension("kdbx.tmp");
        assert!(!tmp_path.exists(), "temp file must be cleaned up");

        // Reopen and verify the entry survived.
        let reopened = crate::keepass::KeePassRepository::open(&path, "vault-pw", None)
            .expect("reopen succeeds");
        let restored = reopened
            .password_for_entry(&entry_id)
            .expect("entry survives roundtrip");
        assert_eq!(restored, "hunter2");

        // Idempotent save: writing twice in a row should still produce a
        // readable file (this is what auto-save does on every mutation).
        reopened
            .save_payload()
            .save_to(&path)
            .expect("second save succeeds");
        let _ = crate::keepass::KeePassRepository::open(&path, "vault-pw", None)
            .expect("reopen after second save");
    }

    #[test]
    fn create_entry_mutation_round_trip() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("crud.kdbx");

        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "vault-pw".to_string(), None);

        // Find the root group's id from the live database (root_mut creates one
        // on demand if missing) and use it as the destination for the new entry.
        let root_id = doc.database.root().id().to_string();

        let draft = EntryDraft {
            title: "GitHub".to_string(),
            username: "elias".to_string(),
            password: "S3cret!".to_string(),
            url: "github.com".to_string(),
            notes: "Personal account".to_string(),
            tags: vec!["Work".to_string(), "2FA".to_string()],
        };
        let new_id = doc
            .create_entry(&root_id, &draft)
            .expect("create_entry succeeds");

        // Snapshot must reflect the new entry immediately (renderers need it).
        assert!(
            doc.snapshot()
                .root
                .entries
                .iter()
                .any(|e| e.id == new_id && e.title == "GitHub"),
            "new entry visible in snapshot",
        );

        // Round-trip: save, reopen, verify the password came back.
        doc.save_payload().save_to(&path).expect("save");
        let reopened =
            crate::keepass::KeePassRepository::open(&path, "vault-pw", None).expect("reopen");
        let pw = reopened.password_for_entry(&new_id).expect("password back");
        assert_eq!(pw, "S3cret!");
    }

    #[test]
    fn delete_entry_moves_to_recycle_bin() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);

        let root_id = doc.database.root().id().to_string();
        let id = doc
            .create_entry(&root_id, &EntryDraft { title: "Deletable".into(), ..Default::default() })
            .expect("create");

        // Recycle-bin should not exist yet — delete must lazily create one.
        assert!(doc.database.recycle_bin().is_none(), "no recycle bin initially");

        doc.delete_entry(&id).expect("delete");

        // Entry should no longer be in the root group.
        assert!(
            !doc.snapshot().root.entries.iter().any(|e| e.id == id),
            "deleted entry removed from root"
        );

        // Recycle bin must now exist and contain the entry.
        let bin = doc.database.recycle_bin().expect("recycle bin created");
        let bin_id_str = bin.id().to_string();
        assert!(
            doc.snapshot()
                .find_group(&bin_id_str)
                .map(|g| g.entries.iter().any(|e| e.id == id))
                .unwrap_or(false),
            "deleted entry is now in the recycle bin"
        );
    }

    #[test]
    fn update_entry_changes_password() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);

        let root_id = doc.database.root().id().to_string();
        let id = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "Original".into(),
                    password: "old".into(),
                    ..Default::default()
                },
            )
            .expect("create");

        doc.update_entry(
            &id,
            &EntryDraft {
                title: "Renamed".into(),
                password: "new".into(),
                ..Default::default()
            },
        )
        .expect("update");

        let entry = doc.snapshot().find_entry(&id).expect("entry exists");
        assert_eq!(entry.title, "Renamed");
        assert_eq!(doc.password_for_entry(&id).as_deref(), Some("new"));
    }

    #[test]
    fn save_to_wrong_password_path_still_writes_then_fails_to_reopen() {
        // Sanity check: ensure the password we save with is what's required to
        // open. Catches accidental key-source mixups (e.g. using a default key).
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("locked.kdbx");

        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let doc = VaultDocument::new(db, snapshot, "secret".to_string(), None);
        doc.save_payload().save_to(&path).expect("save succeeds");

        // Wrong password → open must fail.
        assert!(crate::keepass::KeePassRepository::open(&path, "wrong", None).is_err());
        // Correct password → open succeeds.
        assert!(crate::keepass::KeePassRepository::open(&path, "secret", None).is_ok());
    }
}

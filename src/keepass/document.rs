use crate::domain::{CustomField, VaultSnapshot};
use crate::keepass::repository::{
    STANDARD_FIELDS, find_entry_id, find_group_id, snapshot_from_database,
};
use keepass::{Database, DatabaseKey, db::fields};
use std::{
    fmt, fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
    sync::Arc,
    sync::atomic::{AtomicU64, Ordering},
};
use thiserror::Error;

/// Tag we use to mark favourites. Compared case-insensitively on read so
/// vaults that already use "favorite" / "FAVORITE" / etc. just work.
/// Single canonical casing on write keeps the database tidy.
pub(crate) const FAVORITE_TAG: &str = "Favorite";

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
    /// Monotonic mutation counter. Every mutator funnels through
    /// `refresh_snapshot`, which bumps it. The sync merge flow snapshots
    /// this alongside the database clone it diffs against and compares it
    /// again when the merged result is ready to install — a mismatch means
    /// the user edited the document mid-merge, and installing the merged
    /// copy would silently discard that edit.
    generation: u64,
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
            generation: 0,
        }
    }

    /// Current mutation generation — see the field docs. Compare two reads
    /// for equality only; the absolute value carries no meaning.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn snapshot(&self) -> &VaultSnapshot {
        &self.snapshot
    }

    /// The master password used to unlock this vault. Required by the sync
    /// flow when a 412 conflict happens — we need to decrypt the remote
    /// bytes against the same key, then re-encrypt the merged result.
    /// Lifetime-bound to `&self` so callers don't accidentally store it
    /// outside the document's scope.
    pub fn password(&self) -> &str {
        &self.password
    }

    /// Optional keyfile path — same reason as `password()`. `None` for
    /// password-only vaults.
    pub fn keyfile_path(&self) -> Option<&Path> {
        self.keyfile_path.as_deref()
    }

    /// Borrow the live `Database` for diff / read-only operations (e.g. by
    /// the sync conflict-resolution path).
    pub fn database(&self) -> &Database {
        &self.database
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

    /// Read a single custom-field (non-standard string) value off an
    /// entry. Used by the launcher path to look up `SAP_CONN`, etc.
    /// without snapshotting all custom fields. Returns `None` when the
    /// entry doesn't exist or the field isn't set.
    pub fn custom_field_value(&self, entry_id: &str, key: &str) -> Option<String> {
        let entry = self
            .database
            .iter_all_entries()
            .find(|e| e.id().to_string() == entry_id)?;
        let value = entry.fields.get(key)?;
        Some(value.get().clone())
    }

    /// Raw `otp` field of an entry — `otpauth://...` URL or bare secret.
    /// Used to prefill the Edit modal so the user can change/remove it.
    /// Returns `None` if the entry has no OTP set.
    pub fn otp_url_for_entry(&self, entry_id: &str) -> Option<String> {
        let entry = self
            .database
            .iter_all_entries()
            .find(|e| e.id().to_string() == entry_id)?;
        entry
            .get_raw_otp_value()
            .map(|s| s.to_string())
            .filter(|s| !s.trim().is_empty())
    }

    /// Compute the current TOTP code for an entry, if one is configured.
    /// Returns `None` when the entry has no `otp` field, the field is malformed,
    /// or the system clock is unreadable. Cheap (~microseconds), so we call it
    /// from the per-second UI tick rather than caching.
    pub fn totp_for_entry(&self, entry_id: &str) -> Option<OtpDisplay> {
        let entry = self
            .database
            .iter_all_entries()
            .find(|e| e.id().to_string() == entry_id)?;
        let raw = entry.get_raw_otp_value()?;
        let totp = parse_otp_value(raw)?;
        let code = totp.value_now().ok()?;
        // The spec uses 6 digits in 99% of issuers; insert a thin space mid-code
        // for readability (`123 456`) when we get an even count.
        let display_code = format_code(&code.code);
        Some(OtpDisplay {
            code: display_code,
            remaining_secs: code.valid_for.as_secs() as u32,
            period_secs: code.period.as_secs() as u32,
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
        let group_id =
            find_group_id(&self.database, group_id_str).ok_or(MutationError::GroupNotFound)?;
        let mut group = self
            .database
            .group_mut(group_id)
            .ok_or(MutationError::GroupNotFound)?;
        let mut entry = group.add_entry();
        apply_draft_to_entry(&mut entry, draft);
        // Tags are an *initial* set on create — `apply_draft_to_entry`
        // intentionally leaves them alone so updates don't wipe out
        // tags the user entered in another KeePass client. Custom
        // fields go through `apply_draft_to_entry` directly because
        // the editor produces authoritative drafts on every save.
        entry.tags = draft.tags.clone();
        let id = entry.id().to_string();
        // Force the borrows to drop before we touch `self` again.
        drop(entry);
        drop(group);
        self.refresh_snapshot();
        Ok(id)
    }

    /// Add or remove the favorite-marker tag on an entry. The convention
    /// is a single tag named `Favorite` (case-insensitive on read), which
    /// KeePassXC users already commonly use to flag favourites — this
    /// keeps our "Favorites" view in sync with what the user sees in
    /// other clients. Returns the new starred state. Caller is expected
    /// to schedule a background save.
    pub fn toggle_starred(&mut self, entry_id_str: &str) -> Result<bool, MutationError> {
        let entry_id =
            find_entry_id(&self.database, entry_id_str).ok_or(MutationError::EntryNotFound)?;
        let mut entry = self
            .database
            .entry_mut(entry_id)
            .ok_or(MutationError::EntryNotFound)?;

        let was_starred = entry
            .tags
            .iter()
            .any(|t| t.eq_ignore_ascii_case(FAVORITE_TAG));
        let changed_at = next_change_time(entry.times.last_modification);
        let mut entry = entry.track_changes();
        entry.edit(|entry| {
            if was_starred {
                entry
                    .tags
                    .retain(|tag| !tag.eq_ignore_ascii_case(FAVORITE_TAG));
            } else {
                entry.tags.push(FAVORITE_TAG.to_string());
            }
        });
        entry.times.last_modification = Some(changed_at);
        drop(entry);
        self.refresh_snapshot();
        Ok(!was_starred)
    }

    pub fn update_entry(
        &mut self,
        entry_id_str: &str,
        draft: &EntryDraft,
    ) -> Result<(), MutationError> {
        let entry_id =
            find_entry_id(&self.database, entry_id_str).ok_or(MutationError::EntryNotFound)?;
        let mut entry = self
            .database
            .entry_mut(entry_id)
            .ok_or(MutationError::EntryNotFound)?;
        let changed_at = next_change_time(entry.times.last_modification);
        let mut entry = entry.track_changes();
        entry.edit(|entry| apply_draft_to_entry(entry, draft));
        entry.times.last_modification = Some(changed_at);
        drop(entry);
        self.refresh_snapshot();
        Ok(())
    }

    /// Toggle the KeePass `IsExpanded` flag on a group. Persisting via
    /// the standard save flow keeps the user's sidebar collapse state
    /// across sessions and across other clients (KeePassXC and KeePass2
    /// honour the same flag, so dipping in from another app doesn't
    /// scramble what's open here). Returns `Ok` even when no flip is
    /// needed — idempotent.
    pub fn set_group_expanded(
        &mut self,
        group_id_str: &str,
        expanded: bool,
    ) -> Result<(), MutationError> {
        let group_id =
            find_group_id(&self.database, group_id_str).ok_or(MutationError::GroupNotFound)?;
        let mut group = self
            .database
            .group_mut(group_id)
            .ok_or(MutationError::GroupNotFound)?;
        if group.is_expanded == expanded {
            return Ok(());
        }
        let changed_at = next_change_time(group.times.last_modification);
        let mut group = group.track_changes();
        group.edit(|group| group.is_expanded = expanded);
        group.times.last_modification = Some(changed_at);
        drop(group);
        self.refresh_snapshot();
        Ok(())
    }

    /// Replace the entry's icon with a custom-icon blob. `bytes` is the raw
    /// PNG/JPEG/ICO/etc — keepass-rs stores it verbatim and our
    /// repository-side magic-byte sniffer figures out the format on the
    /// next read. Used by the favicon downloader; safe to call repeatedly
    /// (each call replaces any previous icon, including a previously-
    /// downloaded one).
    pub fn set_entry_custom_icon(
        &mut self,
        entry_id_str: &str,
        bytes: Vec<u8>,
    ) -> Result<(), MutationError> {
        let entry_id =
            find_entry_id(&self.database, entry_id_str).ok_or(MutationError::EntryNotFound)?;
        let mut entry = self
            .database
            .entry_mut(entry_id)
            .ok_or(MutationError::EntryNotFound)?;
        if entry
            .as_ref()
            .custom_icon()
            .is_some_and(|icon| icon.data.as_slice() == bytes.as_slice())
        {
            return Ok(());
        }
        let changed_at = next_change_time(entry.times.last_modification);
        let mut entry = entry.track_changes();
        // `set_icon_custom_new` drops any previous icon (built-in or
        // custom) and registers a fresh `CustomIconId`. We don't try to
        // dedupe identical blobs across entries — the typical vault has
        // distinct icons per site, and the dedup bookkeeping isn't worth
        // it for an explicit user action.
        let mut current = entry.as_mut();
        let mut icon = current.set_icon_custom_new(bytes);
        icon.last_modification_time = Some(changed_at);
        drop(icon);
        drop(current);
        entry.times.last_modification = Some(changed_at);
        drop(entry);
        self.refresh_snapshot();
        Ok(())
    }

    /// Move an entry into `target_group_id`. Used by the drag-and-drop UI
    /// to relocate entries between groups; trash-drop and the explicit
    /// delete buttons go through `delete_entry` instead so the recycle
    /// bin gets lazily created when needed.
    pub fn move_entry(
        &mut self,
        entry_id_str: &str,
        target_group_id_str: &str,
    ) -> Result<(), MutationError> {
        let entry_id =
            find_entry_id(&self.database, entry_id_str).ok_or(MutationError::EntryNotFound)?;
        let target_id = find_group_id(&self.database, target_group_id_str)
            .ok_or(MutationError::GroupNotFound)?;
        let mut entry = self
            .database
            .entry_mut(entry_id)
            .ok_or(MutationError::EntryNotFound)?;
        if entry.as_ref().parent().id() == target_id {
            return Ok(());
        }
        let changed_at = next_change_time(entry.times.location_changed);
        let mut entry = entry.track_changes();
        entry
            .move_to(target_id)
            .map_err(|_| MutationError::GroupNotFound)?;
        entry.times.location_changed = Some(changed_at);
        drop(entry);
        self.refresh_snapshot();
        Ok(())
    }

    /// Move an entry to the database's Recycle Bin (creating one if missing).
    /// We deliberately don't expose hard-delete from this API yet — that lives
    /// behind the future "Empty trash" affordance in the Trash sidebar view.
    pub fn delete_entry(&mut self, entry_id_str: &str) -> Result<(), MutationError> {
        let entry_id =
            find_entry_id(&self.database, entry_id_str).ok_or(MutationError::EntryNotFound)?;
        let recycle_bin_id = self.ensure_recycle_bin();
        let entry = self
            .database
            .entry(entry_id)
            .ok_or(MutationError::EntryNotFound)?;
        let previous_parent = entry.parent().id();
        if group_is_within(&self.database, previous_parent, recycle_bin_id) {
            return Ok(());
        }
        let changed_at = next_change_time(
            entry
                .times
                .last_modification
                .max(entry.times.location_changed),
        );
        let mut entry = self
            .database
            .entry_mut(entry_id)
            .ok_or(MutationError::EntryNotFound)?;
        let mut entry = entry.track_changes();
        entry.previous_parent_group = Some(previous_parent);
        entry
            .move_to(recycle_bin_id)
            .map_err(|_| MutationError::RecycleBinUnavailable)?;
        entry.times.last_modification = Some(changed_at);
        entry.times.location_changed = Some(changed_at);
        drop(entry);
        self.refresh_snapshot();
        Ok(())
    }

    /// Permanently remove an entry from the database. Bypasses the Recycle
    /// Bin — call this only after explicit user confirmation; the data is
    /// unrecoverable once `save_async` flushes the result to disk.
    pub fn delete_entry_permanent(&mut self, entry_id_str: &str) -> Result<(), MutationError> {
        let entry_id =
            find_entry_id(&self.database, entry_id_str).ok_or(MutationError::EntryNotFound)?;
        let mut entry = self
            .database
            .entry_mut(entry_id)
            .ok_or(MutationError::EntryNotFound)?;
        let deleted_at = next_change_time(
            entry
                .times
                .last_modification
                .max(entry.times.location_changed),
        );
        entry.track_changes().remove();
        // `EntryTrack::remove` uses wall-clock seconds directly. A rapid edit
        // may already have advanced the entry timestamp to preserve ordering,
        // so make the tombstone monotonic as well or native merge can mistake
        // the deletion for an older change and resurrect the entry.
        self.database
            .deleted_objects
            .insert(entry_id.uuid(), Some(deleted_at));
        self.refresh_snapshot();
        Ok(())
    }

    /// Move an entry out of the Recycle Bin. KeePass' `PreviousParentGroup`
    /// points it back to its original group; root is the safe fallback when
    /// that group no longer exists or is itself inside the Recycle Bin.
    pub fn restore_entry(&mut self, entry_id_str: &str) -> Result<(), MutationError> {
        let entry_id =
            find_entry_id(&self.database, entry_id_str).ok_or(MutationError::EntryNotFound)?;
        let root_id = self.database.root().id();
        let recycle_bin_id = self.database.recycle_bin().map(|group| group.id());
        let entry = self
            .database
            .entry(entry_id)
            .ok_or(MutationError::EntryNotFound)?;
        let current_parent = entry.parent().id();
        if !recycle_bin_id.is_some_and(|recycle_bin_id| {
            group_is_within(&self.database, current_parent, recycle_bin_id)
        }) {
            return Ok(());
        }
        let target_id = entry
            .previous_parent_group
            .filter(|candidate| {
                self.database.group(*candidate).is_some()
                    && !recycle_bin_id.is_some_and(|recycle_bin_id| {
                        group_is_within(&self.database, *candidate, recycle_bin_id)
                    })
            })
            .unwrap_or(root_id);
        if current_parent == target_id && entry.previous_parent_group.is_none() {
            return Ok(());
        }
        let changed_at = next_change_time(
            entry
                .times
                .last_modification
                .max(entry.times.location_changed),
        );
        let mut entry = self
            .database
            .entry_mut(entry_id)
            .ok_or(MutationError::EntryNotFound)?;
        let mut entry = entry.track_changes();
        entry.previous_parent_group = None;
        entry
            .move_to(target_id)
            .map_err(|_| MutationError::RecycleBinUnavailable)?;
        entry.times.last_modification = Some(changed_at);
        entry.times.location_changed = Some(changed_at);
        drop(entry);
        self.refresh_snapshot();
        Ok(())
    }

    /// Create a new group under `parent_id_str` and return its stringified id.
    /// Trims the name and rejects empty values up front so we don't end up
    /// with anonymous rows in the sidebar. Caller is expected to schedule a
    /// background save afterwards.
    pub fn create_group(
        &mut self,
        parent_id_str: &str,
        name: &str,
    ) -> Result<String, MutationError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(MutationError::GroupNameEmpty);
        }
        let parent_id =
            find_group_id(&self.database, parent_id_str).ok_or(MutationError::GroupNotFound)?;
        let mut parent = self
            .database
            .group_mut(parent_id)
            .ok_or(MutationError::GroupNotFound)?;
        let mut new_group = parent.add_group();
        new_group.name = name.to_string();
        new_group.times.last_modification = Some(keepass::db::Times::now());
        let id = new_group.id().to_string();
        drop(new_group);
        drop(parent);
        self.refresh_snapshot();
        Ok(id)
    }

    /// Rename an existing group. Empty names are rejected (would leave a
    /// blank row in the sidebar). Bumps `last_modification` so other
    /// KeePass clients see the change on the next sync.
    pub fn rename_group(
        &mut self,
        group_id_str: &str,
        new_name: &str,
    ) -> Result<(), MutationError> {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            return Err(MutationError::GroupNameEmpty);
        }
        let group_id =
            find_group_id(&self.database, group_id_str).ok_or(MutationError::GroupNotFound)?;
        let mut group = self
            .database
            .group_mut(group_id)
            .ok_or(MutationError::GroupNotFound)?;
        if group.name == new_name {
            return Ok(());
        }
        let changed_at = next_change_time(group.times.last_modification);
        let mut group = group.track_changes();
        group.edit(|group| group.name = new_name.to_string());
        group.times.last_modification = Some(changed_at);
        drop(group);
        self.refresh_snapshot();
        Ok(())
    }

    /// Soft-delete a group: move the entire subtree to the Recycle Bin.
    /// Mirrors `delete_entry`'s contract — reversible via the Trash view.
    /// Refuses to delete the root, the Recycle Bin itself, or any group
    /// whose subtree contains the Recycle Bin (the latter only happens
    /// when another client moved RB under a sub-group; `move_to` would
    /// otherwise return `WouldCreateCycle` which we surface as a clearer
    /// error message).
    pub fn delete_group(&mut self, group_id_str: &str) -> Result<(), MutationError> {
        let root_id = self.database.root().id();
        if root_id.to_string() == group_id_str {
            return Err(MutationError::CannotDeleteRoot);
        }
        if let Some(rb) = self.database.recycle_bin() {
            if rb.id().to_string() == group_id_str {
                return Err(MutationError::CannotDeleteRecycleBin);
            }
        }
        let group_id =
            find_group_id(&self.database, group_id_str).ok_or(MutationError::GroupNotFound)?;
        if let Some(rb_id) = self.database.recycle_bin().map(|g| g.id())
            && let Some(target) = self.database.root().group(group_id)
            && subtree_contains(&target, rb_id)
        {
            return Err(MutationError::CannotDeleteRecycleBin);
        }
        let recycle_bin_id = self.ensure_recycle_bin();
        let group = self
            .database
            .group(group_id)
            .ok_or(MutationError::GroupNotFound)?;
        let previous_parent = group.parent().ok_or(MutationError::CannotDeleteRoot)?.id();
        if group_is_within(&self.database, previous_parent, recycle_bin_id) {
            return Ok(());
        }
        let changed_at = next_change_time(
            group
                .times
                .last_modification
                .max(group.times.location_changed),
        );
        let mut group = self
            .database
            .group_mut(group_id)
            .ok_or(MutationError::GroupNotFound)?;
        let mut group = group.track_changes();
        group.previous_parent_group = Some(previous_parent);
        group.move_to(recycle_bin_id).map_err(|e| match e {
            keepass::db::MoveGroupError::CannotMoveRoot => MutationError::CannotDeleteRoot,
            _ => MutationError::RecycleBinUnavailable,
        })?;
        group.times.last_modification = Some(changed_at);
        group.times.location_changed = Some(changed_at);
        drop(group);
        self.refresh_snapshot();
        Ok(())
    }

    /// Returns the recycle-bin group id, creating one under the root if the
    /// database doesn't already have one set in `meta.recyclebin_uuid`.
    fn ensure_recycle_bin(&mut self) -> keepass::db::GroupId {
        if let Some(id) = self.database.recycle_bin().map(|group| group.id()) {
            if self.database.meta.recyclebin_enabled != Some(true) {
                self.database.meta.recyclebin_enabled = Some(true);
                self.database.meta.recyclebin_changed = Some(keepass::db::Times::now());
            }
            return id;
        }
        let mut root = self.database.root_mut();
        let mut bin = root.add_group();
        bin.name = "Recycle Bin".to_string();
        let id = bin.id();
        drop(bin);
        drop(root);
        self.database.meta.recyclebin_enabled = Some(true);
        self.database.meta.recyclebin_uuid = Some(id.uuid());
        self.database.meta.recyclebin_changed = Some(keepass::db::Times::now());
        id
    }

    fn refresh_snapshot(&mut self) {
        self.generation = self.generation.wrapping_add(1);
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
    /// 2FA secret. Either a raw `otpauth://...` URL (preferred — keeps
    /// algorithm/digits/period/issuer config) or just the base32 secret. Empty
    /// = no OTP. Stored as a *protected* field because the value is the seed
    /// that generates every future code.
    pub otp: String,
    /// Non-standard string fields (KeePassXC's "Additional attributes").
    /// Drives our launcher detection (`SAP_CONN`, etc.) and round-trips
    /// through other clients. Entries with empty `key` are skipped on
    /// write, so the editor can keep blank rows around without polluting
    /// the saved database.
    pub custom_fields: Vec<CustomField>,
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
    if draft.otp.trim().is_empty() {
        entry.set_protected(fields::OTP, "");
    } else {
        // Store as protected — the value contains the TOTP seed.
        entry.set_protected(fields::OTP, draft.otp.trim().to_string());
    }
    // Tags deliberately not assigned here. The edit form doesn't expose a
    // tag input, so doing `entry.tags = draft.tags` would silently wipe
    // tags the user maintains in KeePassXC every time they re-saved an
    // entry. `create_entry` initialises tags explicitly; updates leave
    // them untouched.
    //
    // Custom fields, by contrast, *are* under the editor's control —
    // the AddEntry/EditEntry modal populates `draft.custom_fields`
    // from its row state on every save (including blank rows, which
    // `apply_custom_fields` filters out). Removing a row in the
    // editor must therefore propagate to the database, which is why
    // we run the rewrite here on both create and update paths.
    apply_custom_fields(entry, &draft.custom_fields);
    entry.times.last_modification = Some(keepass::db::Times::now());
}

/// Replace the non-standard fields on `entry` with `draft_fields`, in
/// two passes: drop everything that's currently outside `STANDARD_FIELDS`
/// (so removing a row in the editor actually removes it from the DB),
/// then re-write the draft. Standard fields untouched.
///
/// Empty keys are skipped — the editor leaves blank rows around for the
/// "+" button to fill, and we don't want those polluting the save.
fn apply_custom_fields<E>(entry: &mut E, draft_fields: &[CustomField])
where
    E: std::ops::DerefMut<Target = keepass::db::Entry>,
{
    let drop: Vec<String> = entry
        .fields
        .keys()
        .filter(|k| !STANDARD_FIELDS.contains(&k.as_str()))
        .cloned()
        .collect();
    for key in drop {
        entry.fields.remove(&key);
    }
    for cf in draft_fields {
        let key = cf.key.trim();
        if key.is_empty() {
            continue;
        }
        if cf.protected {
            entry.set_protected(key, cf.value.clone());
        } else {
            entry.set_unprotected(key, cf.value.clone());
        }
    }
}

fn subtree_contains(root: &keepass::db::GroupRef<'_>, target: keepass::db::GroupId) -> bool {
    if root.id() == target {
        return true;
    }
    root.groups().any(|child| subtree_contains(&child, target))
}

fn group_is_within(
    database: &Database,
    candidate: keepass::db::GroupId,
    ancestor: keepass::db::GroupId,
) -> bool {
    let mut current = Some(candidate);
    while let Some(group_id) = current {
        if group_id == ancestor {
            return true;
        }
        current = database
            .group(group_id)
            .and_then(|group| group.parent().map(|parent| parent.id()));
    }
    false
}

/// KDBX timestamps have one-second precision. Advancing past an existing
/// timestamp avoids creating two divergent revisions that native merge cannot
/// order when a user performs multiple mutations during the same second.
fn next_change_time(previous: Option<chrono::NaiveDateTime>) -> chrono::NaiveDateTime {
    let now = keepass::db::Times::now();
    match previous {
        Some(previous) if now <= previous => previous
            .checked_add_signed(chrono::Duration::seconds(1))
            .unwrap_or(previous),
        _ => now,
    }
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
    #[error("group name must not be empty")]
    GroupNameEmpty,
    #[error("the root group cannot be deleted")]
    CannotDeleteRoot,
    #[error("the Recycle Bin cannot be deleted")]
    CannotDeleteRecycleBin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StrengthReport {
    pub strength: crate::domain::Strength,
    pub length: usize,
    pub bits: u32,
    pub score: u8,
}

/// A formatted, ready-to-display TOTP code with its remaining validity window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OtpDisplay {
    pub code: String,
    pub remaining_secs: u32,
    pub period_secs: u32,
}

/// Parse the raw OTP field of a KeePass entry into a `TOTP`.
///
/// The keepass crate's `Entry::get_otp` only accepts `otpauth://totp/...`
/// URLs, but our UI advertises "otpauth URL or secret" and the KeePassXC
/// import path also accepts bare base32 secrets that authenticator apps
/// often hand out unwrapped. When the stored value isn't a URL, we wrap
/// it in a default-parameter otpauth URL so the same downstream parser
/// handles both shapes. Whitespace inside the secret (some apps render
/// it grouped: `JBSW Y3DP …`) is stripped, and the alphabet is
/// upper-cased so lower-case input doesn't fail base32 decode.
///
/// Also force `digits=6` when the URL doesn't pin a value: keepass-rs's
/// missing-param default is 8, which contradicts RFC 6238 and every
/// mainstream authenticator (Google, Authy, Microsoft) — leaving it on
/// 8 produced codes that just don't match the server's expectation.
fn parse_otp_value(raw: &str) -> Option<keepass::db::TOTP> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let base = if trimmed.starts_with("otpauth://") {
        trimmed.to_string()
    } else {
        let normalized: String = trimmed
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .flat_map(char::to_uppercase)
            .collect();
        format!("otpauth://totp/?secret={normalized}")
    };
    let url = if query_has_param(&base, "digits") {
        base
    } else {
        let sep = if base.contains('?') { '&' } else { '?' };
        format!("{base}{sep}digits=6")
    };
    url.parse().ok()
}

/// Cheap "is `key=` set in this URL's query string?" check. Avoids
/// pulling in a full URL parser just to inspect one parameter — we
/// already trust the input to be either an `otpauth://` URL or a base32
/// secret we just wrapped, so the query split is unambiguous.
fn query_has_param(url: &str, key: &str) -> bool {
    let Some((_, query)) = url.split_once('?') else {
        return false;
    };
    query
        .split('&')
        .any(|pair| pair.split_once('=').is_some_and(|(k, _)| k == key))
}

/// Inserts a thin space in the middle of even-length codes (`123456` → `123 456`).
/// Improves readability for the common 6/8-digit cases without breaking parsers
/// that strip whitespace; copy-to-clipboard should pass the raw digits.
fn format_code(raw: &str) -> String {
    let n = raw.chars().count();
    if n % 2 == 0 && (4..=10).contains(&n) {
        let half = n / 2;
        let first: String = raw.chars().take(half).collect();
        let second: String = raw.chars().skip(half).collect();
        format!("{first} {second}")
    } else {
        raw.to_string()
    }
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
    /// Build a payload from arbitrary inputs — used by the sync conflict
    /// flow to encrypt a freshly-merged Database without going through the
    /// live VaultDocument (which already holds the *un*-merged state).
    pub fn for_merged(database: Database, password: String, keyfile_path: Option<PathBuf>) -> Self {
        SavePayload {
            database,
            password,
            keyfile_path,
        }
    }

    /// Atomically write the database to `target_path`. Writes to a uniquely
    /// named `<target>.<pid>-<seq>.tmp` sibling, fsyncs, then renames over
    /// the target so a crash mid-write can never leave a half-written
    /// `.kdbx`.
    pub fn save_to(self, target_path: &Path) -> Result<(), SaveError> {
        let mut key = DatabaseKey::new();
        if !self.password.is_empty() {
            key = key.with_password(&self.password);
        }
        if let Some(kf) = &self.keyfile_path {
            let mut kf_handle = fs::File::open(kf).map_err(SaveError::ReadKeyfile)?;
            key = key
                .with_keyfile(&mut kf_handle)
                .map_err(SaveError::Keyfile)?;
        }

        let tmp_path = temp_path_for(target_path);
        // Scope the file handle so it's flushed + dropped before rename.
        let write_result = (|| {
            let mut tmp = fs::File::create(&tmp_path).map_err(SaveError::CreateTemp)?;
            self.database
                .save(&mut tmp, key)
                .map_err(|e| SaveError::Encode(e.to_string()))?;
            tmp.flush().map_err(SaveError::WriteTemp)?;
            tmp.sync_all().map_err(SaveError::WriteTemp)?;
            Ok(())
        })();
        // Best-effort cleanup on every failure arm: with per-save unique
        // names, a stranded temp file would otherwise accumulate forever.
        if let Err(e) = write_result {
            let _ = fs::remove_file(&tmp_path);
            return Err(e);
        }
        fs::rename(&tmp_path, target_path).map_err(|e| {
            let _ = fs::remove_file(&tmp_path);
            SaveError::Rename(e)
        })?;
        Ok(())
    }
}

fn temp_path_for(target: &Path) -> PathBuf {
    // Keep the temp file next to the destination so the rename is on the
    // same filesystem (atomic on POSIX/macOS). The name embeds the pid plus
    // a process-wide counter so two concurrent saves — a second one of ours
    // that slipped past serialization, or another app instance's — can never
    // truncate each other's temp file and publish interleaved bytes via the
    // final rename.
    static SAVE_SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SAVE_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut buf = target.as_os_str().to_owned();
    buf.push(format!(".{}-{}.tmp", std::process::id(), seq));
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

    fn merge_clean(mut destination: Database, source: &Database) -> Database {
        let log = destination.merge(source).expect("native merge succeeds");
        assert!(
            log.warnings.is_empty(),
            "native merge produced warnings: {:?}",
            log.warnings,
        );
        destination
    }

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

        // No leftover temp file from a successful save — the target must be
        // the only thing in the directory (temp names are per-save unique,
        // so scan instead of probing one fixed name).
        let leftovers: Vec<_> = fs::read_dir(tmp.path())
            .expect("read tempdir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .filter(|n| n != "roundtrip.kdbx")
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );

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
            username: "alice".to_string(),
            password: "S3cret!".to_string(),
            url: "github.com".to_string(),
            notes: "Personal account".to_string(),
            tags: vec!["Work".to_string(), "2FA".to_string()],
            otp: String::new(),
            custom_fields: Vec::new(),
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
    fn entry_updates_keep_history_and_merge_without_timestamp_sleep() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);
        let root_id = doc.database.root().id().to_string();
        let id = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "Before".into(),
                    password: "old-secret".into(),
                    ..Default::default()
                },
            )
            .expect("create");
        let entry_id = find_entry_id(&doc.database, &id).expect("entry id");
        let original_time = doc
            .database
            .entry(entry_id)
            .and_then(|entry| entry.times.last_modification)
            .expect("original timestamp");
        let base = doc.database.clone();

        doc.update_entry(
            &id,
            &EntryDraft {
                title: "After".into(),
                password: "new-secret".into(),
                ..Default::default()
            },
        )
        .expect("update");
        doc.toggle_starred(&id).expect("favorite");

        let changed = doc.database.entry(entry_id).expect("changed entry");
        assert!(
            changed.times.last_modification.expect("changed timestamp") > original_time,
            "rapid edits must advance beyond KDBX's one-second timestamp precision",
        );
        let history = changed.history.as_ref().expect("history");
        assert_eq!(history.get_entries().len(), 2);
        assert_eq!(history.get_entries()[0].get_title(), Some("After"));
        assert_eq!(history.get_entries()[1].get_title(), Some("Before"));

        let merged = merge_clean(base, doc.database());
        let merged_entry = merged.entry(entry_id).expect("merged entry");
        assert_eq!(merged_entry.get_title(), Some("After"));
        assert_eq!(merged_entry.get_password(), Some("new-secret"));
        assert!(merged_entry.tags.iter().any(|tag| tag == FAVORITE_TAG));
    }

    #[test]
    fn entry_move_tracks_location_without_claiming_a_content_edit() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);
        let root_id = doc.database.root().id().to_string();
        let source_id = doc.create_group(&root_id, "Source").expect("source");
        let target_id = doc.create_group(&root_id, "Target").expect("target");
        let id = doc
            .create_entry(
                &source_id,
                &EntryDraft {
                    title: "Movable".into(),
                    ..Default::default()
                },
            )
            .expect("entry");
        let entry_id = find_entry_id(&doc.database, &id).expect("entry id");
        let before = doc.database.entry(entry_id).expect("entry before move");
        let old_location = before.times.location_changed.expect("old location time");
        let old_modification = before.times.last_modification;
        let base = doc.database.clone();

        doc.move_entry(&id, &target_id).expect("move");

        let moved = doc.database.entry(entry_id).expect("moved entry");
        assert!(moved.times.location_changed.expect("new location time") > old_location);
        assert_eq!(
            moved.times.last_modification, old_modification,
            "a pure move must not win an unrelated concurrent content edit",
        );
        let merged = merge_clean(base, doc.database());
        assert_eq!(
            merged.entry(entry_id).expect("merged entry").parent().id(),
            find_group_id(&merged, &target_id).expect("target id"),
        );
    }

    #[test]
    fn recycle_and_restore_preserve_parent_and_merge_cleanly() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);
        let root_id = doc.database.root().id().to_string();
        let recycle_bin_id = doc.ensure_recycle_bin();
        let original_group = doc.create_group(&root_id, "Original").expect("group");
        let original_group_id = find_group_id(&doc.database, &original_group).expect("group id");
        let id = doc
            .create_entry(
                &original_group,
                &EntryDraft {
                    title: "Recoverable".into(),
                    ..Default::default()
                },
            )
            .expect("entry");
        let entry_id = find_entry_id(&doc.database, &id).expect("entry id");
        let base = doc.database.clone();

        doc.delete_entry(&id).expect("trash");
        let trashed = doc.database.entry(entry_id).expect("trashed entry");
        assert_eq!(trashed.parent().id(), recycle_bin_id);
        assert_eq!(trashed.previous_parent_group, Some(original_group_id));
        assert!(!doc.database.deleted_objects.contains_key(&entry_id.uuid()));
        let merged_trash = merge_clean(base, doc.database());
        assert_eq!(
            merged_trash
                .entry(entry_id)
                .expect("merged trashed entry")
                .parent()
                .id(),
            recycle_bin_id,
        );

        let trashed_base = doc.database.clone();
        doc.restore_entry(&id).expect("restore");
        let restored = doc.database.entry(entry_id).expect("restored entry");
        assert_eq!(restored.parent().id(), original_group_id);
        assert_eq!(restored.previous_parent_group, None);
        let merged_restore = merge_clean(trashed_base, doc.database());
        assert_eq!(
            merged_restore
                .entry(entry_id)
                .expect("merged restored entry")
                .parent()
                .id(),
            original_group_id,
        );
    }

    #[test]
    fn permanent_entry_delete_records_a_mergeable_tombstone() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);
        let root_id = doc.database.root().id().to_string();
        let id = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "Delete permanently".into(),
                    ..Default::default()
                },
            )
            .expect("entry");
        let entry_id = find_entry_id(&doc.database, &id).expect("entry id");
        doc.delete_entry(&id).expect("trash first");
        let last_entry_change = doc
            .database
            .entry(entry_id)
            .and_then(|entry| {
                entry
                    .times
                    .last_modification
                    .max(entry.times.location_changed)
            })
            .expect("trashed entry timestamp");
        let base = doc.database.clone();

        doc.delete_entry_permanent(&id).expect("permanent delete");

        assert!(doc.database.entry(entry_id).is_none());
        assert!(
            doc.database.deleted_objects[&entry_id.uuid()].expect("tombstone timestamp")
                > last_entry_change,
        );
        let merged = merge_clean(base, doc.database());
        assert!(merged.entry(entry_id).is_none());
        assert!(merged.deleted_objects.contains_key(&entry_id.uuid()));
    }

    #[test]
    fn group_updates_and_recycle_move_are_native_mergeable() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);
        let root_id = doc.database.root().id().to_string();
        let recycle_bin_id = doc.ensure_recycle_bin();
        let id = doc.create_group(&root_id, "Before").expect("group");
        let group_id = find_group_id(&doc.database, &id).expect("group id");
        let original_time = doc
            .database
            .group(group_id)
            .and_then(|group| group.times.last_modification)
            .expect("group timestamp");
        let base = doc.database.clone();

        doc.rename_group(&id, "After").expect("rename");
        doc.set_group_expanded(&id, false).expect("collapse");
        doc.delete_group(&id).expect("trash group");

        let changed = doc.database.group(group_id).expect("changed group");
        assert_eq!(changed.name, "After");
        assert!(!changed.is_expanded);
        assert_eq!(changed.parent().expect("parent").id(), recycle_bin_id);
        assert_eq!(
            changed.previous_parent_group,
            Some(doc.database.root().id())
        );
        assert!(changed.times.last_modification.expect("changed timestamp") > original_time);

        let merged = merge_clean(base, doc.database());
        let merged_group = merged.group(group_id).expect("merged group");
        assert_eq!(merged_group.name, "After");
        assert!(!merged_group.is_expanded);
        assert_eq!(merged_group.parent().expect("parent").id(), recycle_bin_id);
    }

    #[test]
    fn custom_icon_update_tracks_entry_history_and_is_idempotent() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);
        let root_id = doc.database.root().id().to_string();
        let id = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "Icon".into(),
                    ..Default::default()
                },
            )
            .expect("entry");
        let entry_id = find_entry_id(&doc.database, &id).expect("entry id");
        let before = doc
            .database
            .entry(entry_id)
            .and_then(|entry| entry.times.last_modification)
            .expect("timestamp");

        doc.set_entry_custom_icon(&id, vec![1, 2, 3])
            .expect("set icon");
        let changed = doc.database.entry(entry_id).expect("changed entry");
        assert!(changed.times.last_modification.expect("changed timestamp") > before);
        assert_eq!(
            changed
                .history
                .as_ref()
                .expect("history")
                .get_entries()
                .len(),
            1,
        );
        let changed_at = changed.times.last_modification;
        let icon_count = doc.database.num_custom_icons();

        doc.set_entry_custom_icon(&id, vec![1, 2, 3])
            .expect("same icon is a no-op");
        let unchanged = doc.database.entry(entry_id).expect("unchanged entry");
        assert_eq!(unchanged.times.last_modification, changed_at);
        assert_eq!(doc.database.num_custom_icons(), icon_count);
    }

    #[test]
    fn delete_entry_moves_to_recycle_bin() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);

        let root_id = doc.database.root().id().to_string();
        let id = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "Deletable".into(),
                    ..Default::default()
                },
            )
            .expect("create");

        // Recycle-bin should not exist yet — delete must lazily create one.
        assert!(
            doc.database.recycle_bin().is_none(),
            "no recycle bin initially"
        );

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
    fn create_group_adds_under_parent() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);
        let root_id = doc.database.root().id().to_string();

        let id = doc
            .create_group(&root_id, "  Work  ")
            .expect("create_group");

        let group = doc.snapshot().find_group(&id).expect("new group visible");
        assert_eq!(group.name, "Work", "name is trimmed");
        assert!(
            doc.snapshot().root.groups.iter().any(|g| g.id == id),
            "child of root"
        );
    }

    #[test]
    fn rename_group_updates_name() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);
        let root_id = doc.database.root().id().to_string();
        let id = doc.create_group(&root_id, "Old").expect("create");

        doc.rename_group(&id, " New ").expect("rename");

        assert_eq!(doc.snapshot().find_group(&id).unwrap().name, "New");
    }

    #[test]
    fn delete_group_moves_subtree_to_recycle_bin() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);
        let root_id = doc.database.root().id().to_string();
        let parent = doc.create_group(&root_id, "Parent").expect("parent");
        let child = doc.create_group(&parent, "Child").expect("child");
        let entry_id = doc
            .create_entry(
                &child,
                &EntryDraft {
                    title: "Inside".into(),
                    ..Default::default()
                },
            )
            .expect("entry");

        doc.delete_group(&parent).expect("delete");

        // The parent (and its subtree) should be reachable via the recycle bin.
        let bin = doc.database.recycle_bin().expect("rb created");
        let bin_id = bin.id().to_string();
        let rb_group = doc.snapshot().find_group(&bin_id).expect("rb in snapshot");
        let trashed_parent = rb_group
            .groups
            .iter()
            .find(|g| g.id == parent)
            .expect("parent under rb");
        let trashed_child = trashed_parent
            .groups
            .iter()
            .find(|g| g.id == child)
            .expect("child still nested under parent");
        assert!(
            trashed_child.entries.iter().any(|e| e.id == entry_id),
            "entry still inside child"
        );

        // And it must be gone from the visible (non-trash) tree.
        assert!(
            !doc.snapshot().root.groups.iter().any(|g| g.id == parent),
            "parent removed from root"
        );
    }

    #[test]
    fn delete_root_rejected() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);
        let root_id = doc.database.root().id().to_string();

        let err = doc
            .delete_group(&root_id)
            .expect_err("root cannot be deleted");
        assert!(matches!(err, MutationError::CannotDeleteRoot));
    }

    #[test]
    fn delete_recycle_bin_rejected() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);
        let root_id = doc.database.root().id().to_string();

        // Force the recycle bin into existence by deleting a throwaway entry.
        let throwaway = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "tmp".into(),
                    ..Default::default()
                },
            )
            .expect("entry");
        doc.delete_entry(&throwaway).expect("trash entry");

        let bin_id = doc.database.recycle_bin().unwrap().id().to_string();
        let err = doc.delete_group(&bin_id).expect_err("rb cannot be deleted");
        assert!(matches!(err, MutationError::CannotDeleteRecycleBin));
    }

    #[test]
    fn empty_group_name_rejected() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);
        let root_id = doc.database.root().id().to_string();

        assert!(matches!(
            doc.create_group(&root_id, "   "),
            Err(MutationError::GroupNameEmpty)
        ));
        let g = doc.create_group(&root_id, "Real").expect("create");
        assert!(matches!(
            doc.rename_group(&g, ""),
            Err(MutationError::GroupNameEmpty)
        ));
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
    fn toggle_starred_round_trips_via_favorite_tag() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);
        let root_id = doc.database.root().id().to_string();

        let id = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "Mail".into(),
                    tags: vec!["Personal".into()],
                    ..Default::default()
                },
            )
            .expect("create");

        // Initially: not starred. Snapshot agrees.
        assert!(!doc.snapshot().find_entry(&id).unwrap().starred);

        // Toggle on: returns true, tag added, other tags untouched.
        let now_starred = doc.toggle_starred(&id).expect("toggle on");
        assert!(now_starred);
        let entry = doc.snapshot().find_entry(&id).unwrap();
        assert!(entry.starred);
        assert!(
            entry
                .tags
                .iter()
                .any(|t| t.eq_ignore_ascii_case(FAVORITE_TAG))
        );
        assert!(entry.tags.contains(&"Personal".to_string()));

        // Toggle off: returns false, tag removed, other tags still there.
        let now_unstarred = doc.toggle_starred(&id).expect("toggle off");
        assert!(!now_unstarred);
        let entry = doc.snapshot().find_entry(&id).unwrap();
        assert!(!entry.starred);
        assert!(
            !entry
                .tags
                .iter()
                .any(|t| t.eq_ignore_ascii_case(FAVORITE_TAG))
        );
        assert!(entry.tags.contains(&"Personal".to_string()));
    }

    #[test]
    fn pre_existing_favorite_tag_is_recognised_case_insensitively() {
        // Cross-client compatibility: a vault opened from KeePassXC where
        // the user typed "favorite" or "FAVORITE" should still light up
        // our star without us silently rewriting their tag casing on read.
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);
        let root_id = doc.database.root().id().to_string();

        let id = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "External".into(),
                    tags: vec!["favorite".into()], // lowercase!
                    ..Default::default()
                },
            )
            .expect("create");

        assert!(doc.snapshot().find_entry(&id).unwrap().starred);
    }

    #[test]
    fn update_entry_preserves_existing_tags() {
        // Regression: previously every update wiped `entry.tags` because
        // `apply_draft_to_entry` did `entry.tags = draft.tags.clone()`
        // and the edit form sends an empty tags vec. That silently
        // destroyed tags maintained in another KeePass client.
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);
        let root_id = doc.database.root().id().to_string();

        let id = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "Tagged".into(),
                    tags: vec!["Personal".into(), "Mail".into()],
                    ..Default::default()
                },
            )
            .expect("create");

        // Update with an empty draft.tags — what the edit form sends today.
        doc.update_entry(
            &id,
            &EntryDraft {
                title: "Tagged (renamed)".into(),
                tags: Vec::new(),
                ..Default::default()
            },
        )
        .expect("update");

        let entry = doc.snapshot().find_entry(&id).expect("entry exists");
        assert_eq!(entry.title, "Tagged (renamed)");
        assert_eq!(
            entry.tags,
            vec!["Personal".to_string(), "Mail".to_string()],
            "edit must not wipe tags it doesn't manage"
        );
    }

    #[test]
    fn move_entry_relocates_between_groups() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);
        let root_id = doc.database.root().id().to_string();

        // Set up two child groups under root.
        let work_id = {
            let mut root = doc.database.root_mut();
            let mut group = root.add_group();
            group.name = "Work".into();
            let id = group.id().to_string();
            drop(group);
            drop(root);
            id
        };
        let personal_id = {
            let mut root = doc.database.root_mut();
            let mut group = root.add_group();
            group.name = "Personal".into();
            let id = group.id().to_string();
            drop(group);
            drop(root);
            id
        };
        doc.refresh_snapshot();

        // Create the entry in Work.
        let entry_id = doc
            .create_entry(
                &work_id,
                &EntryDraft {
                    title: "Movable".into(),
                    ..Default::default()
                },
            )
            .expect("create");

        // Sanity: lives in Work.
        assert!(
            doc.snapshot()
                .find_group(&work_id)
                .unwrap()
                .entries
                .iter()
                .any(|e| e.id == entry_id)
        );

        // Move to Personal.
        doc.move_entry(&entry_id, &personal_id).expect("move");

        assert!(
            doc.snapshot()
                .find_group(&personal_id)
                .unwrap()
                .entries
                .iter()
                .any(|e| e.id == entry_id),
            "entry now in Personal"
        );
        assert!(
            !doc.snapshot()
                .find_group(&work_id)
                .unwrap()
                .entries
                .iter()
                .any(|e| e.id == entry_id),
            "entry no longer in Work"
        );

        // Move back to root.
        doc.move_entry(&entry_id, &root_id).expect("move-to-root");
        assert!(
            doc.snapshot().root.entries.iter().any(|e| e.id == entry_id),
            "entry now at root"
        );
    }

    #[test]
    fn move_entry_rejects_unknown_ids() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);
        let root_id = doc.database.root().id().to_string();
        let id = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "X".into(),
                    ..Default::default()
                },
            )
            .expect("create");

        assert!(matches!(
            doc.move_entry("not-an-entry", &root_id),
            Err(MutationError::EntryNotFound)
        ));
        assert!(matches!(
            doc.move_entry(&id, "not-a-group"),
            Err(MutationError::GroupNotFound)
        ));
    }

    #[test]
    fn restore_entry_returns_to_root() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);

        let root_id = doc.database.root().id().to_string();
        let id = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "Recoverable".into(),
                    ..Default::default()
                },
            )
            .expect("create");

        doc.delete_entry(&id).expect("trash");
        let bin_id = doc.database.recycle_bin().expect("bin").id().to_string();
        // Sanity: entry is in the bin.
        assert!(
            doc.snapshot()
                .find_group(&bin_id)
                .map(|g| g.entries.iter().any(|e| e.id == id))
                .unwrap_or(false)
        );

        doc.restore_entry(&id).expect("restore");
        // Now in root, no longer in bin.
        assert!(doc.snapshot().root.entries.iter().any(|e| e.id == id));
        assert!(
            !doc.snapshot()
                .find_group(&bin_id)
                .map(|g| g.entries.iter().any(|e| e.id == id))
                .unwrap_or(false),
            "restored entry should leave the recycle bin"
        );
    }

    #[test]
    fn delete_entry_permanent_actually_removes_it() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);

        let root_id = doc.database.root().id().to_string();
        let id = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "Goner".into(),
                    ..Default::default()
                },
            )
            .expect("create");

        doc.delete_entry_permanent(&id).expect("perma delete");

        // Gone from snapshot AND not in recycle bin (perma-delete bypasses it).
        assert!(doc.snapshot().find_entry(&id).is_none());
        if let Some(bin_id) = &doc.snapshot().recycle_bin_id {
            let bin = doc.snapshot().find_group(bin_id).expect("bin");
            assert!(!bin.entries.iter().any(|e| e.id == id));
        }
    }

    /// Custom fields supplied via `EntryDraft.custom_fields` are persisted
    /// into the entry's `fields` map (with the `Protected` bit honoured),
    /// survive an in-memory save+reopen, surface back via the snapshot's
    /// `VaultEntry.custom_fields`, and are individually retrievable via
    /// `custom_field_value`. This is the round-trip the SAP launcher
    /// relies on — a regression here would silently break "open SAP GUI".
    #[test]
    fn custom_fields_round_trip() {
        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("custom.kdbx");

        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "vault-pw".into(), None);
        let root_id = doc.database.root().id().to_string();

        let draft = EntryDraft {
            title: "SAP DEV".into(),
            password: "hunter2".into(),
            custom_fields: vec![
                CustomField {
                    key: "SAP_CONN".into(),
                    value: "/H/sap.example.com/S/3200".into(),
                    protected: false,
                },
                CustomField {
                    key: "SAP_LANG".into(),
                    value: "DE".into(),
                    protected: false,
                },
                CustomField {
                    key: "API_TOKEN".into(),
                    // Stored as protected — represents a secret-like field
                    // the user might keep alongside the password.
                    value: "sk-ze9y-zhg0-x".into(),
                    protected: true,
                },
            ],
            ..Default::default()
        };
        let id = doc.create_entry(&root_id, &draft).expect("create");

        // In-memory snapshot already exposes them, sorted alphabetically.
        let fields = &doc.snapshot().find_entry(&id).expect("entry").custom_fields;
        let keys: Vec<&str> = fields.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, vec!["API_TOKEN", "SAP_CONN", "SAP_LANG"]);
        let api = fields.iter().find(|f| f.key == "API_TOKEN").unwrap();
        assert!(api.protected, "API_TOKEN must round-trip as protected");
        let conn = fields.iter().find(|f| f.key == "SAP_CONN").unwrap();
        assert!(!conn.protected, "SAP_CONN was unprotected on the draft");

        // Direct lookup helper used by the launcher path.
        assert_eq!(
            doc.custom_field_value(&id, "SAP_CONN").as_deref(),
            Some("/H/sap.example.com/S/3200")
        );

        // Save + reopen — the kdbx writer must serialise the protection
        // bits and the parser must restore them. (This is what would have
        // broken if we'd written `set_unprotected` for the protected
        // value, since kdbx stores them in different XML positions.)
        doc.save_payload().save_to(&path).expect("save");
        let reopened =
            crate::keepass::KeePassRepository::open(&path, "vault-pw", None).expect("reopen");
        let after = &reopened
            .snapshot()
            .find_entry(&id)
            .expect("entry survived save")
            .custom_fields;
        let api_after = after.iter().find(|f| f.key == "API_TOKEN").unwrap();
        assert!(
            api_after.protected,
            "Protected bit must survive the save/reopen cycle",
        );
        assert_eq!(api_after.value, "sk-ze9y-zhg0-x");
    }

    /// `update_entry` must apply the draft's `custom_fields`
    /// authoritatively — adding a row, editing an existing one, and
    /// dropping one all flow through the editor → draft → save pipe.
    /// Regression test for the T10 wire-up: pre-T10 we only wrote
    /// custom fields on `create_entry`, so any edit silently lost
    /// them. This test exercises all three transitions in one run.
    #[test]
    fn update_entry_rewrites_custom_fields_authoritatively() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);
        let root_id = doc.database.root().id().to_string();

        // Create with two custom fields.
        let id = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "SAP DEV".into(),
                    custom_fields: vec![
                        CustomField {
                            key: "SAP_CONN".into(),
                            value: "/H/old.host/S/3200".into(),
                            protected: false,
                        },
                        CustomField {
                            key: "TO_BE_DROPPED".into(),
                            value: "remove me".into(),
                            protected: false,
                        },
                    ],
                    ..Default::default()
                },
            )
            .expect("create");

        // Edit: change one value, drop one row, add a new protected one.
        doc.update_entry(
            &id,
            &EntryDraft {
                title: "SAP DEV".into(),
                custom_fields: vec![
                    CustomField {
                        key: "SAP_CONN".into(),
                        value: "/H/new.host/S/3200".into(),
                        protected: false,
                    },
                    CustomField {
                        key: "API_TOKEN".into(),
                        value: "sk-fresh".into(),
                        protected: true,
                    },
                ],
                ..Default::default()
            },
        )
        .expect("update");

        let fields = &doc.snapshot().find_entry(&id).expect("entry").custom_fields;
        let keys: Vec<&str> = fields.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, vec!["API_TOKEN", "SAP_CONN"]);
        assert_eq!(
            fields.iter().find(|f| f.key == "SAP_CONN").unwrap().value,
            "/H/new.host/S/3200"
        );
        assert!(
            fields
                .iter()
                .find(|f| f.key == "API_TOKEN")
                .unwrap()
                .protected,
            "newly-added protected row must serialize as protected"
        );
        assert!(
            fields.iter().all(|f| f.key != "TO_BE_DROPPED"),
            "the row removed in the draft must be gone from the entry"
        );
    }

    /// Standard fields (Title/UserName/Password/URL/Notes/otp) must NOT
    /// leak into `custom_fields`. KeePassXC users would notice the
    /// duplication immediately; we'd also break our own filter logic
    /// in the launcher detection.
    #[test]
    fn standard_fields_excluded_from_custom() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "vault-pw".into(), None);
        let root_id = doc.database.root().id().to_string();

        let id = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "Boring".into(),
                    username: "alice".into(),
                    password: "p".into(),
                    url: "u".into(),
                    notes: "n".into(),
                    ..Default::default()
                },
            )
            .expect("create");

        let entry = doc.snapshot().find_entry(&id).expect("entry");
        assert!(
            entry.custom_fields.is_empty(),
            "no standard field should surface as custom: {:?}",
            entry.custom_fields,
        );
    }

    /// EntryDraft.otp is persisted into the entry's OTP field, can be
    /// retrieved via `otp_url_for_entry`, and immediately yields a valid
    /// `OtpDisplay` from the same `totp_for_entry` path the UI uses.
    /// Catches regressions in either set/get plumbing or in keepass-rs's
    /// otpauth URL parser.
    #[test]
    fn create_entry_with_otp_round_trips() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);

        let root_id = doc.database.root().id().to_string();
        let url = "otpauth://totp/Example:alice?secret=JBSWY3DPEHPK3PXP&issuer=Example";
        let id = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "WithOtp".into(),
                    otp: url.into(),
                    ..Default::default()
                },
            )
            .expect("create");

        // Round-trip the URL itself for the Edit-prefill path.
        assert_eq!(doc.otp_url_for_entry(&id).as_deref(), Some(url));

        // Live code path. The pasted URL omits `digits=`, so keepass-rs
        // would have picked its non-standard default of 8; our wrapper
        // injects `digits=6` to match RFC 6238 + every real authenticator.
        let otp = doc.totp_for_entry(&id).expect("totp computes");
        assert!(otp.code.contains(' '), "code is formatted: {}", otp.code);
        let raw: String = otp.code.chars().filter(|c| c.is_ascii_digit()).collect();
        assert_eq!(raw.len(), 6, "expected 6 digits, got: {}", otp.code);
        assert!(otp.remaining_secs <= otp.period_secs);
    }

    /// A user pasting just the base32 secret (e.g. "JBSWY3DPEHPK3PXP")
    /// — what most authenticator apps and many setup pages hand out —
    /// must produce a working live code. Before the bare-secret
    /// fallback was added, the keepass crate's URL-only `from_str`
    /// rejected this input and the UI was stuck rendering "—".
    #[test]
    fn bare_secret_yields_live_code() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);

        let root_id = doc.database.root().id().to_string();
        let id = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "BareSecret".into(),
                    otp: "JBSWY3DPEHPK3PXP".into(),
                    ..Default::default()
                },
            )
            .expect("create");

        let otp = doc.totp_for_entry(&id).expect("bare secret resolves");
        let raw: String = otp.code.chars().filter(|c| c.is_ascii_digit()).collect();
        assert!(!raw.is_empty(), "code is digits, got: {}", otp.code);
        assert!(otp.remaining_secs <= otp.period_secs);
    }

    /// Bare-secret entries must produce 6-digit codes (the universal
    /// authenticator default), not the 8-digit value keepass-rs would
    /// hand back if we left its missing-param default in place.
    #[test]
    fn bare_secret_defaults_to_six_digits() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);

        let root_id = doc.database.root().id().to_string();
        let id = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "BareSecret".into(),
                    otp: "JBSWY3DPEHPK3PXP".into(),
                    ..Default::default()
                },
            )
            .expect("create");

        let otp = doc.totp_for_entry(&id).expect("totp computes");
        let digits: String = otp.code.chars().filter(|c| c.is_ascii_digit()).collect();
        assert_eq!(digits.len(), 6, "expected 6 digits, got: {}", otp.code);
    }

    /// An explicit `digits=8` in the pasted URL must win — we only
    /// inject the default when the URL is silent on the matter.
    #[test]
    fn explicit_eight_digits_is_respected() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);

        let root_id = doc.database.root().id().to_string();
        let url = "otpauth://totp/X?secret=JBSWY3DPEHPK3PXP&digits=8";
        let id = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "EightDigit".into(),
                    otp: url.into(),
                    ..Default::default()
                },
            )
            .expect("create");

        let otp = doc.totp_for_entry(&id).expect("totp computes");
        let digits: String = otp.code.chars().filter(|c| c.is_ascii_digit()).collect();
        assert_eq!(digits.len(), 8, "explicit digits=8 must be preserved");
    }

    /// Authenticator-style grouped + lower-case secrets ("jbsw y3dp …")
    /// must also work; we strip whitespace and upper-case before
    /// handing to the base32 decoder.
    #[test]
    fn bare_secret_accepts_spaces_and_lowercase() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);

        let root_id = doc.database.root().id().to_string();
        let id = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "Pretty".into(),
                    otp: "jbsw y3dp ehpk 3pxp".into(),
                    ..Default::default()
                },
            )
            .expect("create");

        assert!(doc.totp_for_entry(&id).is_some());
    }

    #[test]
    fn update_entry_clears_otp_when_field_empty() {
        let db = Database::new();
        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let mut doc = VaultDocument::new(db, snapshot, "pw".into(), None);

        let root_id = doc.database.root().id().to_string();
        let id = doc
            .create_entry(
                &root_id,
                &EntryDraft {
                    title: "Cleared".into(),
                    otp: "otpauth://totp/X?secret=JBSWY3DPEHPK3PXP".into(),
                    ..Default::default()
                },
            )
            .expect("create");
        assert!(doc.otp_url_for_entry(&id).is_some());

        doc.update_entry(
            &id,
            &EntryDraft {
                title: "Cleared".into(),
                otp: String::new(),
                ..Default::default()
            },
        )
        .expect("update");

        assert!(doc.otp_url_for_entry(&id).is_none(), "otp removed");
        assert!(doc.totp_for_entry(&id).is_none(), "no live code");
    }

    /// AES-KDF round-trip — proves the patched `keepass-rs` actually emits
    /// a UUID readable by other clients, *and* that our own re-open path
    /// accepts what we just wrote. Combined with the unit test inside
    /// `keepass-rs::config::kdf_dump_tests`, this is end-to-end coverage of
    /// the fix.
    #[test]
    fn save_aes_kdf_round_trip() {
        use keepass::config::{DatabaseConfig, KdfConfig};

        let tmp = TempDir::new().expect("tempdir");
        let path = tmp.path().join("aes.kdbx");

        let mut config = DatabaseConfig::default();
        // Modest rounds — enough to exercise the path without slowing the
        // test suite (real-world AES-KDF vaults use 60_000+).
        config.kdf_config = KdfConfig::Aes { rounds: 1_000 };

        let mut db = Database::with_config(config);
        let mut root = db.root_mut();
        let mut entry = root.add_entry();
        entry.set_unprotected(fields::TITLE, "AES roundtrip");
        entry.set_protected(fields::PASSWORD, "p4ss");
        let entry_id = entry.id().to_string();

        let snapshot = VaultSnapshot::new(VaultGroup::default());
        let doc = VaultDocument::new(db, snapshot, "vault-pw".into(), None);
        doc.save_payload().save_to(&path).expect("save");

        // Re-open via our own repository — uses the same parse path the UI
        // path takes, so success here means a real user could re-open too.
        let reopened =
            crate::keepass::KeePassRepository::open(&path, "vault-pw", None).expect("reopen");
        assert_eq!(
            reopened.password_for_entry(&entry_id).as_deref(),
            Some("p4ss"),
        );
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

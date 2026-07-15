//! Pure-data diff and three-way merge over keepass `Database`s, used by the
//! sync conflict resolution flow. No GPUI dependencies — fully unit-testable.
//!
//! Fidelity policy:
//! - Diffing compares every entry field as a `Value<String>`, so field
//!   presence, OTP values, arbitrary custom fields, and protected/unprotected
//!   bits all participate in conflict detection.
//! - Applying choices delegates the structural merge to `Database::merge`.
//!   That preserves UUIDs, group placement and additions, tombstone-driven
//!   deletions, entry history, and the complete field map.
//! - Manual picks only force which current entry version wins. The losing
//!   current version is retained in the winner's history before the database
//!   merge runs.
//! - The pinned keepass fork does not merge attachment or custom-icon
//!   stores. `apply_picks` rejects *divergent* stores explicitly instead of
//!   returning a database with dangling references or lost bytes; identical
//!   stores on both sides are retained safely.
//! - Passwords are compared in cleartext (necessarily — both sides are
//!   already decrypted) but the displayed `FieldDiff.local`/`.remote` for
//!   the Password row is redacted to `"••• (N chars)"` so the conflict
//!   screen is screen-sharing-safe.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    ops::Deref,
};

use chrono::NaiveDateTime;
use keepass::db::{
    AutoType, Color, CustomDataItem, Database, Entry, EntryId, EntryRef, GroupId, GroupRef, Icon,
    Times, Value, fields,
};

use crate::domain::CustomField;
use crate::keepass::repository::{
    STANDARD_FIELDS, collect_custom_fields, find_entry_id, find_group_id,
};

/// Value snapshot of an entry at the moment of diffing — owned, no borrows
/// of the source `Database`. Safe to keep around in UI state for as long as
/// the user is reviewing the conflict.
///
/// Carries the full set of entry fields the merge round-trips, not just the
/// five visible-in-UI ones. When the user picks "Remote" for a conflict, all
/// these fields get transplanted onto the local entry — partial transplants
/// were the source of a silent-data-loss bug pre-v0.2.1.
#[derive(Clone, PartialEq, Eq)]
pub struct EntryView {
    /// EntryId stringified to its UUID. Stable across diff/apply, and
    /// re-hydrated via `EntryId::from_uuid` when adding remote-only entries
    /// to the merged DB so cross-client sync keeps the same identity.
    pub id: String,
    pub title: String,
    pub username: String,
    /// Cleartext. Only ever rendered through `FieldDiff` (which redacts) or
    /// in the entry detail view after explicit reveal.
    pub password: String,
    pub url: String,
    pub notes: String,
    pub modified: Option<NaiveDateTime>,
    /// User-assigned tags. Surfaced as a `FieldDiff` row in the conflict UI.
    pub tags: Vec<String>,
    /// Plugin / metadata key-value pairs attached to the entry by other
    /// KeePass clients (e.g. KeePassXC stores favorite-marker hashes here).
    /// Silently preserved across merges; not surfaced as a diff row.
    pub custom_data: HashMap<String, CustomDataItem>,
    /// Non-standard string fields ("Additional attributes" in KeePassXC),
    /// e.g. our `SAP_CONN`. Pre-fix `populate_from_view` only replayed
    /// the six standard fields, so picking Remote silently wiped these
    /// off the local entry. Carried through here so the conflict-pick
    /// path round-trips them faithfully.
    pub custom_fields: Vec<CustomField>,
    pub autotype: Option<AutoType>,
    pub foreground_color: Option<Color>,
    pub background_color: Option<Color>,
    pub override_url: Option<String>,
}

impl fmt::Debug for EntryView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EntryView")
            .field("id", &self.id)
            .field("modified", &self.modified)
            .field("has_username", &!self.username.is_empty())
            .field("has_password", &!self.password.is_empty())
            .field("has_url", &!self.url.is_empty())
            .field("has_notes", &!self.notes.is_empty())
            .field("tag_count", &self.tags.len())
            .field("custom_data_count", &self.custom_data.len())
            .field("custom_field_count", &self.custom_fields.len())
            .field("has_autotype", &self.autotype.is_some())
            .field("has_override_url", &self.override_url.is_some())
            .finish()
    }
}

/// Internal fidelity snapshot. `EntryView` stays a UI-oriented, stable public
/// shape; the raw map is retained privately so diffing does not collapse a
/// missing field into an empty value or discard its protection bit.
struct EntrySnapshot {
    view: EntryView,
    fields: HashMap<String, Value<String>>,
    icon: Option<Icon>,
    quality_check: Option<bool>,
    previous_parent_group: Option<GroupId>,
}

/// One field's local-vs-remote comparison. `local` and `remote` are the
/// strings the UI should render directly — for the Password row those are
/// pre-redacted; for the rest they're the cleartext field values.
#[derive(Clone, PartialEq, Eq)]
pub struct FieldDiff {
    pub label: &'static str,
    pub local: String,
    pub remote: String,
    pub differs: bool,
}

impl fmt::Debug for FieldDiff {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FieldDiff")
            .field("label", &self.label)
            .field("local_present", &!self.local.is_empty())
            .field("remote_present", &!self.remote.is_empty())
            .field("differs", &self.differs)
            .finish()
    }
}

/// One entry's worth of conflict. Carries enough to render the side-by-side
/// columns and to apply the user's pick later without re-running diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryConflict {
    pub id: String,
    pub local: EntryView,
    pub remote: EntryView,
    pub fields: Vec<FieldDiff>,
}

/// One entry that diverged but was auto-resolved by `last_modification`
/// timestamp — the side with the strictly newer timestamp wins, no UI
/// prompt. `apply_picks` replays these alongside the user's manual picks
/// so the merged DB picks up the winner regardless of whether any other
/// entries forced the overlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoResolved {
    pub id: String,
    pub winner: Side,
    /// Carried so `apply_picks` can transplant remote fields when
    /// `winner == Remote` without re-walking the remote DB.
    pub remote: EntryView,
}

/// The full picture handed to the Conflict overlay. `conflicts` is the list
/// the user must resolve; `local_only` / `remote_only` / `auto_resolved`
/// are auto-merged.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ConflictReport {
    pub conflicts: Vec<EntryConflict>,
    pub local_only: Vec<EntryView>,
    pub remote_only: Vec<EntryView>,
    /// Entries that diverged on at least one visible field but where one
    /// side's `last_modification` is strictly newer — last-write-wins,
    /// applied silently.
    pub auto_resolved: Vec<AutoResolved>,
    /// Group topology/metadata, tombstones, recycle-bin metadata, or entry
    /// histories differ. Direction attribution is deliberately conservative:
    /// writing the merged result back may create a redundant remote version,
    /// but skipping it could strand a local deletion or empty-group change.
    pub structural_writeback_required: bool,
}

impl ConflictReport {
    /// True when no user decisions are required — diff was clean. Caller
    /// can skip the Conflict overlay entirely and just upload `apply_picks`
    /// with an empty pick map.
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
            && self.remote_only.is_empty()
            && self.auto_resolved.is_empty()
            && !self.structural_writeback_required
        // `local_only` doesn't dirty the merge: those entries are already in
        // the local DB we'll start the merge from.
    }

    /// True when applying this report changes the *remote* — i.e. the local
    /// side contributes something the server doesn't already have. That's
    /// either entries only we hold (`local_only`) or a field divergence our
    /// side won (`auto_resolved` with `winner == Local`).
    ///
    /// When this is false the merge is a pure fast-forward (we only pulled
    /// remote-side changes), so the post-merge DB already matches the server
    /// and the caller can skip the upload — avoiding a redundant remote
    /// version for what is really just someone else's change landing here.
    pub fn has_local_contribution(&self) -> bool {
        self.structural_writeback_required
            || !self.local_only.is_empty()
            || self
                .auto_resolved
                .iter()
                .any(|r| matches!(r.winner, Side::Local))
    }
}

/// Which side the user wants to keep for a given conflict.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Local,
    Remote,
}

/// A merge was refused because it could not be completed without either data
/// loss or guessing. Callers should keep both original databases untouched and
/// surface the error as a sync conflict/failure.
#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("attachment-aware merge is not supported yet (local: {local}, remote: {remote})")]
    AttachmentsUnsupported { local: usize, remote: usize },
    #[error(
        "custom-icon stores differ and cannot be merged safely (local: {local}, remote: {remote})"
    )]
    CustomIconsDiffer { local: usize, remote: usize },
    #[error("cannot merge databases with different root group UUIDs")]
    DifferentRoots,
    #[error("{side:?} entry {id} referenced by the conflict report no longer exists")]
    EntryMissing { id: String, side: Side },
    #[error("keepass database merge failed: {0}")]
    DatabaseMerge(String),
    #[error("keepass database merge completed with unresolved warnings: {0}")]
    DatabaseMergeWarnings(String),
}

/// Build a `ConflictReport` between two unlocked databases.
pub fn diff(local: &Database, remote: &Database) -> ConflictReport {
    let local_map = live_entries(local);
    let remote_map = live_entries(remote);

    let local_ids: HashSet<&String> = local_map.keys().collect();
    let remote_ids: HashSet<&String> = remote_map.keys().collect();

    let mut conflicts = Vec::new();
    let mut auto_resolved = Vec::new();
    for id in local_ids.intersection(&remote_ids) {
        let l = &local_map[*id];
        let r = &remote_map[*id];
        let fields = field_diffs(l, r);
        if !fields.iter().any(|f| f.differs) {
            continue;
        }
        // KeePass-style last-write-wins: when one side's `last_modification`
        // is strictly newer, take that side automatically. The overlay is
        // reserved for the genuinely ambiguous cases (timestamps tied or
        // missing) — pre-v0.4 every field-level divergence forced a prompt
        // even when the user had clearly saved one side later than the
        // other, which made benign sync round-trips noisy.
        match timestamp_winner(l.view.modified, r.view.modified) {
            Some(winner) => auto_resolved.push(AutoResolved {
                id: (*id).clone(),
                winner,
                remote: r.view.clone(),
            }),
            None => conflicts.push(EntryConflict {
                id: (*id).clone(),
                local: l.view.clone(),
                remote: r.view.clone(),
                fields,
            }),
        }
    }

    let mut local_only: Vec<EntryView> = local_ids
        .difference(&remote_ids)
        .map(|id| local_map[*id].view.clone())
        .collect();
    let mut remote_only: Vec<EntryView> = remote_ids
        .difference(&local_ids)
        .map(|id| remote_map[*id].view.clone())
        .collect();

    // Stable ordering for deterministic rendering + tests. Title is the
    // user-visible identifier; ties broken by id for full determinism.
    let by_title_then_id =
        |a: &EntryView, b: &EntryView| a.title.cmp(&b.title).then(a.id.cmp(&b.id));
    conflicts.sort_by(|a, b| by_title_then_id(&a.local, &b.local));
    local_only.sort_by(by_title_then_id);
    remote_only.sort_by(by_title_then_id);
    auto_resolved.sort_by(|a, b| a.remote.title.cmp(&b.remote.title).then(a.id.cmp(&b.id)));

    ConflictReport {
        conflicts,
        local_only,
        remote_only,
        auto_resolved,
        structural_writeback_required: structural_state_differs(local, remote),
    }
}

fn structural_state_differs(local: &Database, remote: &Database) -> bool {
    if local.deleted_objects != remote.deleted_objects
        || local.meta.recyclebin_uuid != remote.meta.recyclebin_uuid
    {
        return true;
    }

    let local_groups: HashSet<_> = local.iter_all_groups().map(|group| group.id()).collect();
    let remote_groups: HashSet<_> = remote.iter_all_groups().map(|group| group.id()).collect();
    if local_groups != remote_groups {
        return true;
    }
    for id in local_groups.intersection(&remote_groups) {
        let Some(local_group) = local.group(*id) else {
            return true;
        };
        let Some(remote_group) = remote.group(*id) else {
            return true;
        };
        if local_group.parent().map(|parent| parent.id())
            != remote_group.parent().map(|parent| parent.id())
            || local_group.name != remote_group.name
            || local_group.notes != remote_group.notes
            || local_group.icon() != remote_group.icon()
            || local_group.custom_data != remote_group.custom_data
            || local_group.is_expanded != remote_group.is_expanded
            || local_group.default_autotype_sequence != remote_group.default_autotype_sequence
            || local_group.enable_autotype != remote_group.enable_autotype
            || local_group.enable_searching != remote_group.enable_searching
            || local_group.previous_parent_group != remote_group.previous_parent_group
            || local_group.tags != remote_group.tags
        {
            return true;
        }
    }

    for local_entry in local.iter_all_entries() {
        let Some(remote_entry) = remote.entry(local_entry.id()) else {
            continue;
        };
        if local_entry.parent().id() != remote_entry.parent().id()
            || local_entry.history.as_ref() != remote_entry.history.as_ref()
        {
            return true;
        }
    }

    false
}

/// Returns the strictly-newer side, or `None` when timestamps are tied
/// or either side is missing a `last_modification` (treat as ambiguous —
/// surface to the user). Equal-second timestamps are ambiguous because
/// KeePass file format is second-precision and a true race on the same
/// second is the case where we *want* to prompt.
fn timestamp_winner(local: Option<NaiveDateTime>, remote: Option<NaiveDateTime>) -> Option<Side> {
    match (local, remote) {
        (Some(l), Some(r)) if l > r => Some(Side::Local),
        (Some(l), Some(r)) if r > l => Some(Side::Remote),
        _ => None,
    }
}

/// Build a merged `Database` from both complete inputs and the user's entry
/// choices. Structural changes and non-conflicting entry updates are delegated
/// to keepass-rs's timestamp/tombstone-aware merge implementation.
///
/// A missing manual pick defaults to Local, matching the conflict UI. The
/// chosen current version receives a deterministic newer timestamp and the
/// losing current version is inserted into its history before the structural
/// merge. The originals are never mutated.
pub fn apply_picks(
    local: &Database,
    remote: &Database,
    picks: &HashMap<String, Side>,
    report: &ConflictReport,
) -> Result<Database, ApplyError> {
    preflight_fidelity(local, remote)?;

    let mut merged = local.clone();
    let mut source = remote.clone();

    // Only genuinely ambiguous entries appear here. Timestamp-resolved rows
    // and one-sided additions are handled natively by Database::merge.
    for conflict in &report.conflicts {
        let side = picks.get(&conflict.id).copied().unwrap_or(Side::Local);
        force_manual_winner(&mut merged, &mut source, &conflict.id, side)?;
    }
    for resolved in &report.auto_resolved {
        preserve_auto_resolved_history(&mut merged, &mut source, resolved)?;
    }

    let log = merged
        .merge(&source)
        .map_err(|error| ApplyError::DatabaseMerge(error.to_string()))?;
    // The fork warns both for outcomes it already resolved by documented
    // policy and for genuinely lossy ones. Only the lossy ones may abort:
    // a policy-resolved warning's trigger lives in the remote file (e.g.
    // another client wrote entries without LocationChanged timestamps), so
    // treating it as fatal re-fails every retry identically and wedges sync
    // permanently with no user remedy.
    let lossy: Vec<&str> = log
        .warnings
        .iter()
        .map(String::as_str)
        .filter(|warning| !warning_is_harmless(warning, local, remote))
        .collect();
    if !lossy.is_empty() {
        return Err(ApplyError::DatabaseMergeWarnings(lossy.join("; ")));
    }

    // The merged database is saved directly (no document mutation runs in
    // between), and `merge_history` unions both sides' histories — trim here
    // or repeated conflicts grow entries past the vault's HistoryMaxItems.
    crate::keepass::document::enforce_history_limits(&mut merged);

    Ok(merged)
}

/// Warnings the pinned fork emits for situations it has already resolved
/// without dropping data: same-second diverged history versions (both are
/// kept), missing *history-entry* timestamps (deterministic substitutes
/// inside a history list that is unioned anyway), and missing history (an
/// empty default). The warning set is closed because the fork is pinned by
/// revision; anything unrecognized stays fatal. Deliberately NOT in this
/// list, because each one silently discards a remote change: "Cannot add
/// entry …" (entry dropped), "Cannot move entry/group …" (move dropped),
/// "Cannot determine which … move is more recent" (the fork keeps the
/// local location and the following upload overwrites the remote move),
/// and missing timestamps on *current* entries/groups (a remote rename
/// without a timestamp loses against the epoch substitute). Only the root
/// group, which cannot move anywhere, is exempt.
fn warning_is_policy_resolved(warning: &str) -> bool {
    warning.starts_with("History entries for ")
        || warning.starts_with("Cannot move root group ")
        || (warning.contains(" history entry ")
            && warning.contains("did not have a last modification timestamp"))
        || warning.ends_with("had no history.")
}

/// Extends `warning_is_policy_resolved` with a divergence check for
/// missing-timestamp warnings on *current* entries/groups: the fork emits
/// those before ever comparing the object, so a legacy entry that is
/// byte-identical on both sides would otherwise make every merge attempt
/// fatal forever — while a genuinely divergent one must stay fatal (the
/// epoch substitute would silently pick a winner). The comparison mirrors
/// the pinned fork's own divergence checks: timestamps and history never
/// define current-entry content, while group membership and parent location
/// are merged independently. A parent difference is accepted only when the
/// location timestamps identify one strictly newer side; missing location
/// timestamps also produce a separate fatal move warning.
fn warning_is_harmless(warning: &str, local: &Database, remote: &Database) -> bool {
    if warning_is_policy_resolved(warning) {
        return true;
    }
    if !warning.contains("did not have a last modification timestamp") {
        return false;
    }
    // "Source entry <id> did not …" / "Destination group <id> did not …"
    // ("… history entry …" was already accepted above.)
    let mut words = warning.split_whitespace();
    let kind = words.nth(1);
    let Some(id) = words.next() else {
        return false;
    };
    match kind {
        Some("entry") => entries_equivalent_for_timestamp_warning(local, remote, id),
        Some("group") => groups_equivalent_for_timestamp_warning(local, remote, id),
        _ => false,
    }
}

fn entries_equivalent_for_timestamp_warning(local: &Database, remote: &Database, id: &str) -> bool {
    let (Some(local_id), Some(remote_id)) = (find_entry_id(local, id), find_entry_id(remote, id))
    else {
        return false;
    };
    match (local.entry(local_id), remote.entry(remote_id)) {
        (Some(local_entry), Some(remote_entry)) => entry_content_eq(&local_entry, &remote_entry),
        _ => false,
    }
}

fn entry_content_eq(local: &EntryRef<'_>, remote: &EntryRef<'_>) -> bool {
    entry_location_is_resolved(local, remote)
        && local.fields == remote.fields
        && local.autotype == remote.autotype
        && local.tags == remote.tags
        && local.custom_data == remote.custom_data
        && local.icon() == remote.icon()
        && local.foreground_color == remote.foreground_color
        && local.background_color == remote.background_color
        && local.override_url == remote.override_url
        && local.quality_check == remote.quality_check
        && local.previous_parent_group == remote.previous_parent_group
        // The fork does not expose attachment names. Reference IDs are the
        // strongest public comparison and are also enforced by preflight.
        && sorted_attachment_ids(local) == sorted_attachment_ids(remote)
}

fn entry_location_is_resolved(local: &EntryRef<'_>, remote: &EntryRef<'_>) -> bool {
    local.parent().id() == remote.parent().id()
        || matches!(
            (local.times.location_changed, remote.times.location_changed),
            (Some(local_changed), Some(remote_changed)) if local_changed != remote_changed
        )
}

fn sorted_attachment_ids(entry: &EntryRef<'_>) -> Vec<usize> {
    let mut ids: Vec<_> = entry
        .attachments()
        .map(|attachment| attachment.id().id())
        .collect();
    ids.sort_unstable();
    ids
}

fn groups_equivalent_for_timestamp_warning(local: &Database, remote: &Database, id: &str) -> bool {
    let (Some(local_id), Some(remote_id)) = (find_group_id(local, id), find_group_id(remote, id))
    else {
        return false;
    };
    match (local.group(local_id), remote.group(remote_id)) {
        (Some(local_group), Some(remote_group)) => group_content_eq(&local_group, &remote_group),
        _ => false,
    }
}

fn group_content_eq(local: &GroupRef<'_>, remote: &GroupRef<'_>) -> bool {
    group_location_is_resolved(local, remote)
        && local.name == remote.name
        && local.notes == remote.notes
        && local.icon() == remote.icon()
        && local.custom_data == remote.custom_data
        && local.is_expanded == remote.is_expanded
        && local.default_autotype_sequence == remote.default_autotype_sequence
        && local.enable_autotype == remote.enable_autotype
        && local.enable_searching == remote.enable_searching
        && local.previous_parent_group == remote.previous_parent_group
        && local.tags == remote.tags
}

fn group_location_is_resolved(local: &GroupRef<'_>, remote: &GroupRef<'_>) -> bool {
    local.parent().map(|parent| parent.id()) == remote.parent().map(|parent| parent.id())
        || matches!(
            (local.times.location_changed, remote.times.location_changed),
            (Some(local_changed), Some(remote_changed)) if local_changed != remote_changed
        )
}

fn preserve_auto_resolved_history(
    local: &mut Database,
    remote: &mut Database,
    resolved: &AutoResolved,
) -> Result<(), ApplyError> {
    let entry_id = uuid::Uuid::parse_str(&resolved.id)
        .map(EntryId::from_uuid)
        .map_err(|_| ApplyError::EntryMissing {
            id: resolved.id.clone(),
            side: resolved.winner,
        })?;
    match resolved.winner {
        Side::Local => {
            let losing = clone_entry(remote, entry_id, Side::Remote)?;
            add_history_version(local, entry_id, losing, Side::Local)
        }
        Side::Remote => {
            let losing = clone_entry(local, entry_id, Side::Local)?;
            add_history_version(remote, entry_id, losing, Side::Remote)
        }
    }
}

fn preflight_fidelity(local: &Database, remote: &Database) -> Result<(), ApplyError> {
    if local.root().id() != remote.root().id() {
        return Err(ApplyError::DifferentRoots);
    }

    // The pinned fork's merge does not carry attachments across databases
    // (its own `// TODO: attachments`), so divergent stores would produce
    // dangling references or lost bytes. Identical stores are safe: every
    // attachment id a winning entry references resolves to the same bytes
    // on either side. Rejecting mere *presence* would make a vault with one
    // attachment permanently unable to resolve any conflict.
    if attachment_stores_diverge(local, remote) {
        return Err(ApplyError::AttachmentsUnsupported {
            local: local.num_attachments(),
            remote: remote.num_attachments(),
        });
    }

    let local_icons = custom_icon_store(local);
    let remote_icons = custom_icon_store(remote);
    if local_icons != remote_icons {
        return Err(ApplyError::CustomIconsDiffer {
            local: local_icons.len(),
            remote: remote_icons.len(),
        });
    }

    Ok(())
}

fn attachment_stores_diverge(local: &Database, remote: &Database) -> bool {
    // Store identity: same ids resolving to the same bytes on both sides.
    if local.num_attachments() != remote.num_attachments() {
        return true;
    }
    let remote_by_id: HashMap<usize, _> = remote
        .iter_all_attachments()
        .map(|attachment| (attachment.id().id(), attachment))
        .collect();
    if !local.iter_all_attachments().all(|attachment| {
        remote_by_id
            .get(&attachment.id().id())
            .is_some_and(|remote| remote.data == attachment.data)
    }) {
        return true;
    }

    // Identical blobs are not enough: the fork's merge does not carry the
    // per-entry attachment *references* either, so attaching an existing
    // blob to another entry (or detaching one) remotely would be silently
    // dropped by the merged result. For every entry present on both sides,
    // the reference sets must agree. One-sided entries are covered by the
    // store-identity check above: any blob they reference exists (or is
    // missing) in the store itself.
    // ponytail: reference *names* are not publicly readable off the fork's
    // Entry, so a pure rename (same blob, same entry, new name) still slips
    // through — the bytes stay attached under the old name. Compare names
    // too if the fork ever exposes them.
    let remote_refs = entry_attachment_refs(remote);
    entry_attachment_refs(local).iter().any(|(id, local_ids)| {
        remote_refs
            .get(id)
            .is_some_and(|remote_ids| remote_ids != local_ids)
    })
}

fn entry_attachment_refs(db: &Database) -> HashMap<String, Vec<usize>> {
    db.iter_all_entries()
        .map(|entry| {
            let mut ids: Vec<usize> = entry
                .attachments()
                .map(|attachment| attachment.id().id())
                .collect();
            ids.sort_unstable();
            (entry.id().to_string(), ids)
        })
        .collect()
}

fn custom_icon_store(
    db: &Database,
) -> HashMap<uuid::Uuid, (Vec<u8>, Option<String>, Option<NaiveDateTime>)> {
    db.iter_all_custom_icons()
        .map(|icon| {
            (
                icon.id().uuid(),
                (
                    icon.data.clone(),
                    icon.name.clone(),
                    icon.last_modification_time,
                ),
            )
        })
        .collect()
}

fn force_manual_winner(
    local: &mut Database,
    remote: &mut Database,
    raw_id: &str,
    winner: Side,
) -> Result<(), ApplyError> {
    let entry_id = uuid::Uuid::parse_str(raw_id)
        .map(EntryId::from_uuid)
        .map_err(|_| ApplyError::EntryMissing {
            id: raw_id.to_string(),
            side: winner,
        })?;

    let local_entry = clone_entry(local, entry_id, Side::Local)?;
    let remote_entry = clone_entry(remote, entry_id, Side::Remote)?;
    let winner_time = [
        Times::now(),
        local_entry
            .times
            .last_modification
            .unwrap_or_else(Times::epoch),
        remote_entry
            .times
            .last_modification
            .unwrap_or_else(Times::epoch),
    ]
    .into_iter()
    .max()
    .expect("forced winner timestamp candidates are non-empty");

    match winner {
        Side::Local => {
            add_history_version(local, entry_id, remote_entry, Side::Local)?;
            local
                .entry_mut(entry_id)
                .ok_or_else(|| ApplyError::EntryMissing {
                    id: raw_id.to_string(),
                    side: Side::Local,
                })?
                .times
                .last_modification = Some(winner_time);
            remote
                .entry_mut(entry_id)
                .ok_or_else(|| ApplyError::EntryMissing {
                    id: raw_id.to_string(),
                    side: Side::Remote,
                })?
                .times
                .last_modification = Some(Times::epoch());
        }
        Side::Remote => {
            add_history_version(remote, entry_id, local_entry, Side::Remote)?;
            remote
                .entry_mut(entry_id)
                .ok_or_else(|| ApplyError::EntryMissing {
                    id: raw_id.to_string(),
                    side: Side::Remote,
                })?
                .times
                .last_modification = Some(winner_time);
            local
                .entry_mut(entry_id)
                .ok_or_else(|| ApplyError::EntryMissing {
                    id: raw_id.to_string(),
                    side: Side::Local,
                })?
                .times
                .last_modification = Some(Times::epoch());
        }
    }

    Ok(())
}

fn clone_entry(db: &Database, id: EntryId, side: Side) -> Result<Entry, ApplyError> {
    db.entry(id)
        .map(|entry| entry.deref().clone())
        .ok_or_else(|| ApplyError::EntryMissing {
            id: id.to_string(),
            side,
        })
}

fn add_history_version(
    db: &mut Database,
    id: EntryId,
    mut losing: Entry,
    winner_side: Side,
) -> Result<(), ApplyError> {
    losing.history = None;
    let mut winner = db.entry_mut(id).ok_or_else(|| ApplyError::EntryMissing {
        id: id.to_string(),
        side: winner_side,
    })?;
    let history = winner.history.get_or_insert_default();
    if !history.get_entries().contains(&losing) {
        history.add_entry(losing);
    }
    Ok(())
}

// ---------- internals ----------

fn live_entries(db: &Database) -> HashMap<String, EntrySnapshot> {
    let recycle_bin_id: Option<GroupId> = db.recycle_bin().map(|g| g.id());
    db.iter_all_entries()
        .filter(|e| {
            // "Live" = not directly inside the recycle bin. We don't recurse
            // into recycle-bin subgroups because (a) they're rare and (b)
            // surfacing those as conflicts is more annoying than helpful.
            recycle_bin_id.map_or(true, |bin| e.parent().id() != bin)
        })
        .map(|e| {
            let snapshot = entry_to_snapshot(&e);
            (snapshot.view.id.clone(), snapshot)
        })
        .collect()
}

fn entry_to_snapshot(e: &EntryRef<'_>) -> EntrySnapshot {
    EntrySnapshot {
        view: EntryView {
            id: e.id().to_string(),
            title: e.get(fields::TITLE).unwrap_or("").to_string(),
            username: e.get(fields::USERNAME).unwrap_or("").to_string(),
            password: e.get(fields::PASSWORD).unwrap_or("").to_string(),
            url: e.get(fields::URL).unwrap_or("").to_string(),
            notes: e.get(fields::NOTES).unwrap_or("").to_string(),
            modified: e.times.last_modification,
            tags: e.tags.clone(),
            custom_data: e.custom_data.clone(),
            custom_fields: collect_custom_fields(e),
            autotype: e.autotype.clone(),
            foreground_color: e.foreground_color.clone(),
            background_color: e.background_color.clone(),
            override_url: e.override_url.clone(),
        },
        fields: e.fields.clone(),
        icon: e.icon().cloned(),
        quality_check: e.quality_check,
        previous_parent_group: e.previous_parent_group,
    }
}

fn field_diffs(local: &EntrySnapshot, remote: &EntrySnapshot) -> Vec<FieldDiff> {
    let mut diffs = vec![
        entry_field_diff("Title", fields::TITLE, local, remote, false),
        entry_field_diff("Username", fields::USERNAME, local, remote, false),
        entry_field_diff("Password", fields::PASSWORD, local, remote, true),
        entry_field_diff("URL", fields::URL, local, remote, false),
        entry_field_diff("Notes", fields::NOTES, local, remote, false),
        tags_diff(&local.view.tags, &remote.view.tags),
    ];

    if local.fields.contains_key(fields::OTP) || remote.fields.contains_key(fields::OTP) {
        diffs.push(entry_field_diff("OTP", fields::OTP, local, remote, true));
    }

    let local_additional = additional_fields(&local.fields);
    let remote_additional = additional_fields(&remote.fields);
    if !local_additional.is_empty() || !remote_additional.is_empty() {
        diffs.push(FieldDiff {
            label: "Additional fields",
            local: render_additional_fields(&local_additional),
            remote: render_additional_fields(&remote_additional),
            differs: local_additional != remote_additional,
        });
    }

    let local_protected = protected_field_names(&local.fields);
    let remote_protected = protected_field_names(&remote.fields);
    if !local_protected.is_empty() || !remote_protected.is_empty() {
        diffs.push(FieldDiff {
            label: "Protected fields",
            local: local_protected.join(", "),
            remote: remote_protected.join(", "),
            differs: local_protected != remote_protected,
        });
    }

    let metadata = metadata_differences(local, remote);
    if !metadata.is_empty() {
        let summary = metadata.join(", ");
        diffs.push(FieldDiff {
            label: "Entry settings",
            local: summary.clone(),
            remote: summary,
            differs: true,
        });
    }

    diffs
}

fn entry_field_diff(
    label: &'static str,
    key: &str,
    local: &EntrySnapshot,
    remote: &EntrySnapshot,
    always_redact: bool,
) -> FieldDiff {
    let local_value = local.fields.get(key);
    let remote_value = remote.fields.get(key);
    FieldDiff {
        label,
        local: render_field(local_value, always_redact),
        remote: render_field(remote_value, always_redact),
        // `Value` equality includes both the cleartext and its protected bit.
        differs: local_value != remote_value,
    }
}

fn render_field(value: Option<&Value<String>>, always_redact: bool) -> String {
    let Some(value) = value else {
        return String::new();
    };
    if always_redact || value.is_protected() {
        redact(value.get())
    } else {
        value.get().clone()
    }
}

fn additional_fields(fields_map: &HashMap<String, Value<String>>) -> Vec<(&str, &Value<String>)> {
    let mut fields: Vec<_> = fields_map
        .iter()
        .filter(|(key, _)| !STANDARD_FIELDS.contains(&key.as_str()))
        .map(|(key, value)| (key.as_str(), value))
        .collect();
    fields.sort_by_key(|(key, _)| *key);
    fields
}

fn render_additional_fields(fields: &[(&str, &Value<String>)]) -> String {
    fields
        .iter()
        .map(|(key, value)| {
            let rendered = render_field(Some(value), false);
            format!("{key} = {rendered}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn protected_field_names(fields_map: &HashMap<String, Value<String>>) -> Vec<&str> {
    let mut names: Vec<_> = fields_map
        .iter()
        .filter(|(_, value)| value.is_protected())
        .map(|(key, _)| key.as_str())
        .collect();
    names.sort_unstable();
    names
}

fn metadata_differences(local: &EntrySnapshot, remote: &EntrySnapshot) -> Vec<&'static str> {
    let mut changed = Vec::new();
    if local.view.autotype != remote.view.autotype {
        changed.push("Auto-Type");
    }
    if local.view.custom_data != remote.view.custom_data {
        changed.push("custom data");
    }
    if local.icon != remote.icon {
        changed.push("icon");
    }
    if local.view.foreground_color != remote.view.foreground_color {
        changed.push("foreground color");
    }
    if local.view.background_color != remote.view.background_color {
        changed.push("background color");
    }
    if local.view.override_url != remote.view.override_url {
        changed.push("URL override");
    }
    if local.quality_check != remote.quality_check {
        changed.push("quality check");
    }
    if local.previous_parent_group != remote.previous_parent_group {
        changed.push("previous group");
    }
    changed
}

fn tags_diff(local: &[String], remote: &[String]) -> FieldDiff {
    // Order-sensitive comparison: tags are technically a set in KeePass'
    // mental model, but in the file they're a Vec<String> and clients
    // (including ours) preserve write order. Treating reorder as a diff
    // is the simpler + safer behaviour.
    FieldDiff {
        label: "Tags",
        local: local.join(", "),
        remote: remote.join(", "),
        differs: local != remote,
    }
}

fn redact(pw: &str) -> String {
    if pw.is_empty() {
        String::new()
    } else {
        format!("••• ({} chars)", pw.chars().count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(db: &mut Database, title: &str, password: &str) -> EntryId {
        let mut root = db.root_mut();
        let mut e = root.add_entry();
        e.set_unprotected(fields::TITLE, title);
        e.set_unprotected(fields::USERNAME, "user");
        e.set_protected(fields::PASSWORD, password);
        e.set_unprotected(fields::URL, "https://example.com");
        e.id()
    }

    /// Helper: copy everything from `src` into a fresh DB so tests can
    /// build "local + remote diverged from same starting state" without
    /// fighting Database's lack of clone-with-explicit-id.
    fn fork(src: &Database) -> Database {
        src.clone()
    }

    #[test]
    fn identical_databases_have_no_conflicts() {
        let mut local = Database::new();
        add(&mut local, "GitHub", "secret");
        add(&mut local, "Gmail", "another");
        let remote = fork(&local);

        let report = diff(&local, &remote);
        assert!(report.conflicts.is_empty());
        assert!(report.local_only.is_empty());
        assert!(report.remote_only.is_empty());
        assert!(report.is_clean());
    }

    #[test]
    fn conflict_report_debug_omits_decrypted_entry_content() {
        let sentinels = [
            "local-title-secret",
            "remote-title-secret",
            "local-username-secret",
            "remote-username-secret",
            "local-password-secret",
            "remote-password-secret",
            "custom-key-secret",
            "custom-value-secret",
        ];
        let mut local = Database::new();
        let id = add(&mut local, sentinels[0], sentinels[4]);
        local
            .entry_mut(id)
            .expect("local entry")
            .set_unprotected(fields::USERNAME, sentinels[2]);
        local
            .entry_mut(id)
            .expect("local entry")
            .set_protected(sentinels[6], sentinels[7]);
        let mut remote = local.clone();
        let mut remote_entry = remote.entry_mut(id).expect("remote entry");
        remote_entry.set_unprotected(fields::TITLE, sentinels[1]);
        remote_entry.set_unprotected(fields::USERNAME, sentinels[3]);
        remote_entry.set_protected(fields::PASSWORD, sentinels[5]);

        let rendered = format!("{:?}", diff(&local, &remote));

        for sentinel in sentinels {
            assert!(!rendered.contains(sentinel), "debug leaked {sentinel}");
        }
        assert!(rendered.contains("conflicts"));
    }

    #[test]
    fn local_only_entry_shows_up_in_local_only() {
        let mut local = Database::new();
        let remote = fork(&local);
        add(&mut local, "OnlyHere", "x");

        let report = diff(&local, &remote);
        assert!(report.conflicts.is_empty());
        assert_eq!(report.local_only.len(), 1);
        assert_eq!(report.local_only[0].title, "OnlyHere");
        assert!(report.remote_only.is_empty());
        // Local-only doesn't require user decision — clean.
        assert!(report.is_clean());
    }

    #[test]
    fn remote_only_pull_is_a_pure_fast_forward() {
        // Remote gained an entry, local has nothing the server lacks. The
        // merge should be flagged as needing no upload — otherwise auto-sync
        // mints a redundant remote version just for pulling someone else's
        // change.
        let local = Database::new();
        let mut remote = fork(&local);
        add(&mut remote, "OnlyOnRemote", "x");

        let report = diff(&local, &remote);
        assert!(report.conflicts.is_empty());
        assert!(
            !report.has_local_contribution(),
            "pure remote pull must not require an upload"
        );
    }

    #[test]
    fn local_only_entry_requires_upload() {
        // We hold an entry the server doesn't — the merge must be pushed so
        // the other devices get it.
        let mut local = Database::new();
        let remote = fork(&local);
        add(&mut local, "OnlyHere", "x");

        let report = diff(&local, &remote);
        assert!(report.conflicts.is_empty());
        assert!(
            report.has_local_contribution(),
            "a local-only entry must be uploaded"
        );
    }

    #[test]
    fn remote_only_entry_shows_up_in_remote_only() {
        let local = Database::new();
        let mut remote = fork(&local);
        add(&mut remote, "OnlyOnRemote", "x");

        let report = diff(&local, &remote);
        assert!(report.conflicts.is_empty());
        assert!(report.local_only.is_empty());
        assert_eq!(report.remote_only.len(), 1);
        // Remote-only adds to the merged result, so it's NOT clean — the
        // caller still needs to run apply_picks.
        assert!(!report.is_clean());
    }

    #[test]
    fn divergent_password_creates_conflict_with_only_password_differing() {
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "old-password");
        let mut remote = fork(&local);

        // Local rotates the password
        local
            .entry_mut(id)
            .unwrap()
            .set_protected(fields::PASSWORD, "rotated-locally-24chars");
        // Remote rotates differently — id is preserved across `fork` (which
        // is just `Database::clone`), so the same EntryId is valid in both.
        remote
            .entry_mut(id)
            .unwrap()
            .set_protected(fields::PASSWORD, "rotated-remotely-18");

        let report = diff(&local, &remote);
        assert_eq!(report.conflicts.len(), 1);
        let c = &report.conflicts[0];
        assert_eq!(c.local.title, "GitHub");

        let pw_field = c.fields.iter().find(|f| f.label == "Password").unwrap();
        assert!(pw_field.differs);
        // Redaction: local password is 23 chars, remote is 19. Both rendered
        // as the redacted string, never cleartext.
        assert_eq!(pw_field.local, "••• (23 chars)");
        assert_eq!(pw_field.remote, "••• (19 chars)");

        let title_field = c.fields.iter().find(|f| f.label == "Title").unwrap();
        assert!(!title_field.differs);
        assert_eq!(title_field.local, "GitHub");
    }

    #[test]
    fn newer_remote_auto_resolves_without_user_prompt() {
        use chrono::NaiveDate;
        let older = NaiveDate::from_ymd_opt(2026, 5, 7)
            .unwrap()
            .and_hms_opt(12, 22, 0)
            .unwrap();
        let newer = NaiveDate::from_ymd_opt(2026, 5, 7)
            .unwrap()
            .and_hms_opt(13, 0, 0)
            .unwrap();

        let mut local = Database::new();
        let id = add(&mut local, "Elias SH1", "secret-pass-32-chars-padding-ok!");
        local.entry_mut(id).unwrap().times.last_modification = Some(older);
        let mut remote = fork(&local);
        // Remote has the strictly newer save with non-empty Notes.
        remote
            .entry_mut(id)
            .unwrap()
            .set_unprotected(fields::NOTES, "abc");
        remote.entry_mut(id).unwrap().times.last_modification = Some(newer);

        let report = diff(&local, &remote);
        assert!(
            report.conflicts.is_empty(),
            "newer remote should auto-resolve, not prompt — got {} conflicts",
            report.conflicts.len()
        );
        assert_eq!(report.auto_resolved.len(), 1);
        assert_eq!(report.auto_resolved[0].winner, Side::Remote);
        assert!(!report.is_clean(), "auto-resolved still requires writeback");

        let merged = apply_picks(&local, &remote, &HashMap::new(), &report)
            .expect("newer remote should merge");
        let merged_notes = merged
            .iter_all_entries()
            .find(|e| e.id() == id)
            .unwrap()
            .get(fields::NOTES)
            .unwrap_or("")
            .to_string();
        assert_eq!(merged_notes, "abc");
    }

    #[test]
    fn newer_local_auto_resolves_without_user_prompt() {
        use chrono::NaiveDate;
        let older = NaiveDate::from_ymd_opt(2026, 5, 7)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        let newer = NaiveDate::from_ymd_opt(2026, 5, 7)
            .unwrap()
            .and_hms_opt(13, 0, 0)
            .unwrap();

        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "shared");
        let mut remote = fork(&local);
        local
            .entry_mut(id)
            .unwrap()
            .set_unprotected(fields::URL, "https://new.example.com");
        local.entry_mut(id).unwrap().times.last_modification = Some(newer);
        remote.entry_mut(id).unwrap().times.last_modification = Some(older);

        let report = diff(&local, &remote);
        assert!(report.conflicts.is_empty());
        assert_eq!(report.auto_resolved.len(), 1);
        assert_eq!(report.auto_resolved[0].winner, Side::Local);

        let merged = apply_picks(&local, &remote, &HashMap::new(), &report)
            .expect("newer local should merge");
        let merged_url = merged
            .iter_all_entries()
            .find(|e| e.id() == id)
            .unwrap()
            .get(fields::URL)
            .unwrap_or("")
            .to_string();
        assert_eq!(merged_url, "https://new.example.com");
    }

    #[test]
    fn equal_timestamps_still_prompt_user() {
        use chrono::NaiveDate;
        let same = NaiveDate::from_ymd_opt(2026, 5, 7)
            .unwrap()
            .and_hms_opt(13, 0, 0)
            .unwrap();

        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "shared");
        let mut remote = fork(&local);
        local
            .entry_mut(id)
            .unwrap()
            .set_protected(fields::PASSWORD, "rotated-locally");
        local.entry_mut(id).unwrap().times.last_modification = Some(same);
        remote
            .entry_mut(id)
            .unwrap()
            .set_protected(fields::PASSWORD, "rotated-remotely");
        remote.entry_mut(id).unwrap().times.last_modification = Some(same);

        let report = diff(&local, &remote);
        assert_eq!(report.conflicts.len(), 1);
        assert!(report.auto_resolved.is_empty());
    }

    #[test]
    fn entry_in_recycle_bin_is_filtered_out() {
        let mut db = Database::new();
        let id = add(&mut db, "Trashed", "x");

        // Stand up a recycle bin manually (mirrors what document.rs does on
        // first delete) and move the entry into it.
        let bin_id = {
            let mut root = db.root_mut();
            let mut bin = root.add_group();
            bin.name = "Recycle Bin".into();
            let id = bin.id();
            drop(bin);
            drop(root);
            db.meta.recyclebin_uuid = Some(id.uuid());
            id
        };
        db.entry_mut(id).unwrap().move_to(bin_id).unwrap();

        // Remote has nothing → diff should not surface "Trashed" anywhere.
        let remote = Database::new();
        let report = diff(&db, &remote);
        assert!(report.conflicts.is_empty());
        assert!(
            report.local_only.is_empty(),
            "recycle-bin entries must be filtered out, got {:?}",
            report.local_only
        );
        assert!(report.remote_only.is_empty());
    }

    #[test]
    fn apply_picks_keeps_local_when_no_pick() {
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "local-pw");
        let mut remote = fork(&local);
        remote
            .entry_mut(id)
            .unwrap()
            .set_protected(fields::PASSWORD, "remote-pw");

        let report = diff(&local, &remote);
        // No picks supplied → defaults to Local → password unchanged.
        let merged = apply_picks(&local, &remote, &HashMap::new(), &report)
            .expect("default local pick should merge");
        let entry = merged.entry(id).unwrap();
        assert_eq!(entry.get_password(), Some("local-pw"));
    }

    #[test]
    fn apply_picks_replaces_with_remote_when_picked() {
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "local-pw");
        let mut remote = fork(&local);
        remote
            .entry_mut(id)
            .unwrap()
            .set_protected(fields::PASSWORD, "remote-pw");

        let report = diff(&local, &remote);
        let mut picks = HashMap::new();
        picks.insert(id.to_string(), Side::Remote);

        let merged =
            apply_picks(&local, &remote, &picks, &report).expect("remote pick should merge");
        let entry = merged.entry(id).unwrap();
        assert_eq!(entry.get_password(), Some("remote-pw"));
    }

    #[test]
    fn apply_picks_adds_remote_only_entries_to_root() {
        let local = Database::new();
        let mut remote = fork(&local);
        let remote_id = add(&mut remote, "NewRemote", "remote-secret");

        let report = diff(&local, &remote);
        let merged = apply_picks(&local, &remote, &HashMap::new(), &report)
            .expect("remote-only entry should merge");

        // UUID preservation regression test (bug fixed in v0.2.1): the
        // entry must be findable by the *original* remote EntryId in the
        // merged DB, not just by title. Without this, cross-client sync
        // (FerrisPass ↔ KeePass2) accumulates duplicates exponentially
        // because each merge rewrites the UUID and other clients then
        // see "an entry I haven't seen before" on every cycle.
        let added = merged
            .entry(remote_id)
            .expect("remote entry's UUID must be preserved through apply_picks");
        assert_eq!(added.get_title(), Some("NewRemote"));
        assert_eq!(added.get_password(), Some("remote-secret"));
    }

    #[test]
    fn apply_picks_remote_pick_replaces_tags() {
        // Bug-B regression: pre-v0.2.1, picking Remote only copied 5
        // standard fields. Tags + custom_data + colors stayed at the
        // local value, producing a hybrid the user never asked for.
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "pw");
        local.entry_mut(id).unwrap().tags = vec!["personal".to_string()];

        let mut remote = fork(&local);
        // Diverge: local kept "personal", remote rewrites to "work" + "shared"
        local
            .entry_mut(id)
            .unwrap()
            .set_protected(fields::PASSWORD, "local-pw");
        remote
            .entry_mut(id)
            .unwrap()
            .set_protected(fields::PASSWORD, "remote-pw");
        remote.entry_mut(id).unwrap().tags = vec!["work".to_string(), "shared".to_string()];

        let report = diff(&local, &remote);
        let conflict = report
            .conflicts
            .first()
            .expect("password divergence should produce a conflict");
        let tag_field = conflict
            .fields
            .iter()
            .find(|f| f.label == "Tags")
            .expect("Tags must be one of the field-diff rows");
        assert!(tag_field.differs, "tag-diff should fire when sets differ");
        assert_eq!(tag_field.local, "personal");
        assert_eq!(tag_field.remote, "work, shared");

        // User picks Remote → all remote fields land on the merged entry,
        // including the tags.
        let mut picks = HashMap::new();
        picks.insert(id.to_string(), Side::Remote);
        let merged =
            apply_picks(&local, &remote, &picks, &report).expect("remote tags should merge");

        let entry = merged.entry(id).unwrap();
        assert_eq!(entry.get_password(), Some("remote-pw"));
        assert_eq!(
            entry.tags,
            vec!["work".to_string(), "shared".to_string()],
            "Picking Remote must transplant tags, not just the 5 standard fields"
        );
    }

    /// Regression for the launch-feature precondition: pre-fix,
    /// `populate_from_view` only replayed the six standard fields, so
    /// any non-standard field on the local entry survived "pick remote"
    /// even when the remote side had explicitly removed it — and any
    /// remote-only custom field was silently lost. Either failure mode
    /// would have evaporated SAP_CONN-style configs on the next sync.
    #[test]
    fn apply_picks_remote_pick_replaces_custom_fields() {
        let mut local = Database::new();
        let id = add(&mut local, "SAP DEV", "pw");
        local
            .entry_mut(id)
            .unwrap()
            .set_unprotected("SAP_CONN", "/H/old.host/S/3200");
        local
            .entry_mut(id)
            .unwrap()
            .set_unprotected("LOCAL_ONLY", "should-disappear");

        let mut remote = fork(&local);
        // Diverge passwords so a conflict gets surfaced (apply_picks only
        // touches entries that actually appear in `report.conflicts`).
        local
            .entry_mut(id)
            .unwrap()
            .set_protected(fields::PASSWORD, "local-pw");
        remote
            .entry_mut(id)
            .unwrap()
            .set_protected(fields::PASSWORD, "remote-pw");
        // Remote keeps SAP_CONN but rewrites it, drops LOCAL_ONLY, and
        // adds a brand-new protected field.
        remote
            .entry_mut(id)
            .unwrap()
            .set_unprotected("SAP_CONN", "/H/new.host/S/3200");
        remote.entry_mut(id).unwrap().fields.remove("LOCAL_ONLY");
        remote
            .entry_mut(id)
            .unwrap()
            .set_protected("API_TOKEN", "sk-remote-only");

        let report = diff(&local, &remote);
        let mut picks = HashMap::new();
        picks.insert(id.to_string(), Side::Remote);
        let merged = apply_picks(&local, &remote, &picks, &report)
            .expect("remote custom fields should merge");
        let entry = merged.entry(id).unwrap();

        // Remote's value wins.
        assert_eq!(entry.get("SAP_CONN"), Some("/H/new.host/S/3200"));
        // Local-only field that remote dropped is gone from the merged result.
        assert!(
            entry.get("LOCAL_ONLY").is_none(),
            "LOCAL_ONLY should not survive a pick-Remote when remote dropped it"
        );
        // Remote-only field made it across.
        assert_eq!(entry.get("API_TOKEN"), Some("sk-remote-only"));
        // And the protection bit on that new field is preserved.
        let api_field = entry.fields.get("API_TOKEN").expect("API_TOKEN present");
        assert!(
            api_field.is_protected(),
            "Protected bit must round-trip through the conflict-pick path"
        );
    }

    #[test]
    fn otp_participates_in_diff_and_remote_pick_preserves_protection() {
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "pw");
        local
            .entry_mut(id)
            .unwrap()
            .set_protected(fields::OTP, "otpauth://totp/GitHub:alice?secret=LOCAL");
        let mut remote = fork(&local);
        remote
            .entry_mut(id)
            .unwrap()
            .set_protected(fields::OTP, "otpauth://totp/GitHub:alice?secret=REMOTE");

        let report = diff(&local, &remote);
        let conflict = report.conflicts.first().expect("OTP change must conflict");
        let otp = conflict
            .fields
            .iter()
            .find(|field| field.label == "OTP")
            .expect("OTP needs its own redacted diff row");
        assert!(otp.differs);
        assert!(!otp.local.contains("LOCAL"));
        assert!(!otp.remote.contains("REMOTE"));

        let picks = HashMap::from([(id.to_string(), Side::Remote)]);
        let merged =
            apply_picks(&local, &remote, &picks, &report).expect("OTP-aware merge should succeed");
        let entry = merged.entry(id).unwrap();
        let field = entry.fields.get(fields::OTP).unwrap();
        assert_eq!(field.get(), "otpauth://totp/GitHub:alice?secret=REMOTE");
        assert!(field.is_protected());
    }

    #[test]
    fn protection_only_change_is_detected_and_applied() {
        let mut local = Database::new();
        let id = add(&mut local, "Service", "pw");
        local
            .entry_mut(id)
            .unwrap()
            .set_unprotected("API_TOKEN", "same-value");
        let mut remote = fork(&local);
        remote
            .entry_mut(id)
            .unwrap()
            .set_protected("API_TOKEN", "same-value");

        let report = diff(&local, &remote);
        let conflict = report
            .conflicts
            .first()
            .expect("changing only the protection bit must conflict");
        assert!(
            conflict
                .fields
                .iter()
                .any(|field| field.label == "Protected fields" && field.differs)
        );

        let picks = HashMap::from([(id.to_string(), Side::Remote)]);
        let merged = apply_picks(&local, &remote, &picks, &report)
            .expect("protection-aware merge should succeed");
        let entry = merged.entry(id).unwrap();
        let field = entry.fields.get("API_TOKEN").unwrap();
        assert_eq!(field.get(), "same-value");
        assert!(field.is_protected());
    }

    #[test]
    fn remote_group_and_entry_location_are_preserved() {
        let local = Database::new();
        let mut remote = fork(&local);
        let (group_id, entry_id) = {
            let mut root = remote.root_mut();
            let mut group = root.add_group();
            group.name = "Infrastructure".into();
            let group_id = group.id();
            let mut entry = group.add_entry();
            entry.set_unprotected(fields::TITLE, "Router");
            entry.set_protected(fields::PASSWORD, "secret");
            (group_id, entry.id())
        };

        let report = diff(&local, &remote);
        let merged = apply_picks(&local, &remote, &HashMap::new(), &report)
            .expect("remote group tree should merge");

        assert_eq!(merged.group(group_id).unwrap().name, "Infrastructure");
        assert_eq!(merged.entry(entry_id).unwrap().parent().id(), group_id);
    }

    #[test]
    fn remote_tombstone_deletes_local_entry() {
        let mut local = Database::new();
        let id = add(&mut local, "Deleted elsewhere", "pw");
        let mut remote = fork(&local);
        remote.entry_mut(id).unwrap().track_changes().remove();

        let report = diff(&local, &remote);
        let merged = apply_picks(&local, &remote, &HashMap::new(), &report)
            .expect("tombstone-aware merge should succeed");
        assert!(merged.entry(id).is_none());
        assert!(merged.deleted_objects.contains_key(&id.uuid()));
    }

    #[test]
    fn local_tombstone_forces_writeback_and_is_not_resurrected() {
        let mut local = Database::new();
        let id = add(&mut local, "Deleted locally", "pw");
        let remote = fork(&local);
        local.entry_mut(id).unwrap().track_changes().remove();

        let report = diff(&local, &remote);
        assert!(
            report.has_local_contribution(),
            "a local tombstone must force upload of the merged result"
        );
        let merged = apply_picks(&local, &remote, &HashMap::new(), &report)
            .expect("newer local tombstone should merge");
        assert!(merged.entry(id).is_none());
        assert!(merged.deleted_objects.contains_key(&id.uuid()));
    }

    #[test]
    fn manual_remote_pick_keeps_both_losing_and_existing_history() {
        use chrono::NaiveDate;

        let older = NaiveDate::from_ymd_opt(2025, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "local-current");
        let mut remote = fork(&local);

        let mut old_remote = clone_entry(&remote, id, Side::Remote).unwrap();
        old_remote.history = None;
        old_remote.set_protected(fields::PASSWORD, "remote-history");
        old_remote.times.last_modification = Some(older);
        remote
            .entry_mut(id)
            .unwrap()
            .history
            .get_or_insert_default()
            .add_entry(old_remote);

        remote
            .entry_mut(id)
            .unwrap()
            .set_protected(fields::PASSWORD, "remote-current");
        let report = diff(&local, &remote);
        let picks = HashMap::from([(id.to_string(), Side::Remote)]);
        let merged = apply_picks(&local, &remote, &picks, &report)
            .expect("history-aware merge should succeed");

        let entry = merged.entry(id).unwrap();
        assert_eq!(entry.get_password(), Some("remote-current"));
        let history_passwords: Vec<_> = entry
            .history
            .as_ref()
            .unwrap()
            .get_entries()
            .iter()
            .filter_map(|historical| historical.get_password())
            .collect();
        assert!(history_passwords.contains(&"local-current"));
        assert!(history_passwords.contains(&"remote-history"));
    }

    #[test]
    fn attachments_fail_closed_instead_of_being_dropped() {
        let mut local = Database::new();
        let id = add(&mut local, "With attachment", "pw");
        let remote = fork(&local);
        local
            .entry_mut(id)
            .unwrap()
            .add_attachment("secret.bin", Value::protected(vec![1, 2, 3]));

        let error = apply_picks(&local, &remote, &HashMap::new(), &diff(&local, &remote))
            .expect_err("attachment merge must be refused");
        assert!(matches!(
            error,
            ApplyError::AttachmentsUnsupported {
                local: 1,
                remote: 0
            }
        ));
    }

    #[test]
    fn identical_attachment_stores_merge_instead_of_wedging() {
        let mut local = Database::new();
        let id = add(&mut local, "With attachment", "pw");
        local
            .entry_mut(id)
            .unwrap()
            .add_attachment("secret.bin", Value::protected(vec![1, 2, 3]));
        let mut remote = fork(&local);
        remote
            .entry_mut(id)
            .unwrap()
            .set_protected(fields::PASSWORD, "remote-newer");

        let report = diff(&local, &remote);
        let picks = HashMap::from([(id.to_string(), Side::Remote)]);
        let merged = apply_picks(&local, &remote, &picks, &report)
            .expect("identical attachment stores must not block conflict resolution");

        assert_eq!(merged.num_attachments(), 1);
        assert_eq!(
            merged.entry(id).unwrap().get_password(),
            Some("remote-newer")
        );
    }

    #[test]
    fn policy_resolved_merge_warnings_are_not_fatal() {
        // Exact strings the pinned fork emits for outcomes it already
        // resolved without loss — and the one that genuinely drops data.
        for benign in [
            "History entries for 1234 have the same modification timestamp 2026-01-01 but have diverged.",
            "Cannot move root group 1234",
            "Destination history entry 1234 did not have a last modification timestamp",
            "Source history entry 1234 did not have a last modification timestamp",
            "Source entry 1234 had no history.",
        ] {
            assert!(
                warning_is_policy_resolved(benign),
                "misclassified: {benign}"
            );
        }
        // Everything that silently discards a remote change stays fatal:
        // dropped entries, discarded moves, unorderable moves (the fork
        // keeps local and the next upload overwrites the remote move), and
        // missing timestamps on *current* entries/groups (a remote rename
        // would lose against the epoch substitute).
        for lossy in [
            "Cannot add entry 1234 because its parent group 5678 does not exist in the destination database.",
            "Cannot move entry 1234 to group 5678 because the group does not exist in the destination database.",
            "Cannot move group 1234 to group 5678 because the group does not exist in the destination database.",
            "Cannot determine which entry 1234 move is more recent because one of the entries does not have a location changed timestamp.",
            "Cannot determine which group 1234 move is more recent because one of the groups does not have a location changed timestamp.",
            "Source entry 1234 did not have a last modification timestamp",
            "Destination group 1234 did not have a last modification timestamp",
        ] {
            assert!(!warning_is_policy_resolved(lossy), "misclassified: {lossy}");
        }
    }

    #[test]
    fn missing_entry_timestamp_allows_resolved_move_and_history_merge() {
        let earlier = Times::epoch() + chrono::Duration::seconds(1);
        let later = earlier + chrono::Duration::seconds(1);
        let mut local = Database::new();
        let origin_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Origin".into();
            group.id()
        };
        let target_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Target".into();
            group.id()
        };
        let entry_id = {
            let mut origin = local.group_mut(origin_id).unwrap();
            let mut entry = origin.add_entry();
            entry.set_unprotected(fields::TITLE, "Legacy");
            entry.set_protected(fields::PASSWORD, "pw");
            entry.times.last_modification = Some(earlier);
            entry.times.location_changed = Some(earlier);
            entry.id()
        };

        let mut remote = fork(&local);
        remote
            .entry_mut(entry_id)
            .unwrap()
            .move_to(target_id)
            .unwrap();
        remote.entry_mut(entry_id).unwrap().times.location_changed = Some(later);
        remote.entry_mut(entry_id).unwrap().times.last_modification = None;
        let mut historical = clone_entry(&remote, entry_id, Side::Remote).unwrap();
        historical.history = None;
        historical.times.last_modification = Some(Times::epoch());
        historical.set_unprotected(fields::NOTES, "legacy history");
        remote
            .entry_mut(entry_id)
            .unwrap()
            .history
            .get_or_insert_default()
            .add_entry(historical);

        // Exercise the real fork warning: the current content is unchanged,
        // while location and history are independently and safely merged.
        let mut raw_merged = local.clone();
        let log = raw_merged.merge(&remote).expect("fork merge precondition");
        let warning = format!("Source entry {entry_id} did not have a last modification timestamp");
        assert!(log.warnings.contains(&warning));

        let report = diff(&local, &remote);
        let merged = apply_picks(&local, &remote, &HashMap::new(), &report)
            .expect("resolved move and history must not make timestamp warning fatal");
        let entry = merged.entry(entry_id).unwrap();
        assert_eq!(entry.parent().id(), target_id);
        assert_eq!(entry.history.as_ref().unwrap().get_entries().len(), 1);
    }

    #[test]
    fn missing_entry_timestamp_with_ambiguous_move_remains_fatal() {
        let timestamp = Times::epoch() + chrono::Duration::seconds(1);
        let mut local = Database::new();
        let origin_id = {
            let mut root = local.root_mut();
            root.add_group().id()
        };
        let target_id = {
            let mut root = local.root_mut();
            root.add_group().id()
        };
        let entry_id = {
            let mut origin = local.group_mut(origin_id).unwrap();
            let mut entry = origin.add_entry();
            entry.set_unprotected(fields::TITLE, "Ambiguous move");
            entry.times.last_modification = Some(timestamp);
            entry.times.location_changed = Some(timestamp);
            entry.id()
        };
        let mut remote = fork(&local);
        remote
            .entry_mut(entry_id)
            .unwrap()
            .move_to(target_id)
            .unwrap();
        remote.entry_mut(entry_id).unwrap().times.last_modification = None;
        remote.entry_mut(entry_id).unwrap().times.location_changed = Some(timestamp);

        // Equal concrete location timestamps do not emit the fork's missing-
        // location warning, but they also do not identify which move won.
        let mut raw_merged = local.clone();
        let log = raw_merged.merge(&remote).expect("fork merge precondition");
        let warning = format!("Source entry {entry_id} did not have a last modification timestamp");
        assert!(log.warnings.contains(&warning));

        let error = apply_picks(&local, &remote, &HashMap::new(), &diff(&local, &remote))
            .expect_err("an unorderable move must remain fail-closed");
        assert!(matches!(
            error,
            ApplyError::DatabaseMergeWarnings(message) if message.contains(&warning)
        ));
    }

    #[test]
    fn missing_group_timestamp_allows_independent_child_addition() {
        let mut local = Database::new();
        let group_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Legacy".into();
            group.id()
        };
        let mut remote = fork(&local);
        let child_id = {
            let mut group = remote.group_mut(group_id).unwrap();
            let mut child = group.add_entry();
            child.set_unprotected(fields::TITLE, "Added elsewhere");
            child.id()
        };
        remote.group_mut(group_id).unwrap().times.last_modification = None;

        let mut raw_merged = local.clone();
        let log = raw_merged.merge(&remote).expect("fork merge precondition");
        let warning = format!("Source group {group_id} did not have a last modification timestamp");
        assert!(log.warnings.contains(&warning));

        let report = diff(&local, &remote);
        let merged = apply_picks(&local, &remote, &HashMap::new(), &report)
            .expect("group membership must not make timestamp warning fatal");
        assert_eq!(merged.entry(child_id).unwrap().parent().id(), group_id);
    }

    #[test]
    fn missing_group_timestamp_with_divergent_content_remains_fatal() {
        let mut local = Database::new();
        let group_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Local name".into();
            group.id()
        };
        let mut remote = fork(&local);
        remote.group_mut(group_id).unwrap().name = "Remote name".into();
        remote.group_mut(group_id).unwrap().times.last_modification = None;

        let mut raw_merged = local.clone();
        let log = raw_merged.merge(&remote).expect("fork merge precondition");
        let warning = format!("Source group {group_id} did not have a last modification timestamp");
        assert!(log.warnings.contains(&warning));

        let error = apply_picks(&local, &remote, &HashMap::new(), &diff(&local, &remote))
            .expect_err("divergent current group content must stay fatal");
        assert!(matches!(
            error,
            ApplyError::DatabaseMergeWarnings(message) if message.contains(&warning)
        ));

        // Unknown object IDs remain fail-closed as well.
        let unknown = "Source entry 00000000-0000-0000-0000-000000000000 \
                       did not have a last modification timestamp";
        assert!(!warning_is_harmless(unknown, &local, &remote));
    }

    #[test]
    fn remote_added_custom_icon_fails_closed_instead_of_being_dropped() {
        let mut local = Database::new();
        let id = add(&mut local, "With custom icon", "pw");
        let mut remote = fork(&local);
        remote
            .entry_mut(id)
            .unwrap()
            .set_icon_custom_new(vec![1, 2, 3]);

        let error = apply_picks(&local, &remote, &HashMap::new(), &diff(&local, &remote))
            .expect_err("custom-icon merge must be refused");
        assert!(matches!(
            error,
            ApplyError::CustomIconsDiffer {
                local: 0,
                remote: 1
            }
        ));
    }

    #[test]
    fn identical_custom_icon_stores_are_allowed() {
        let mut local = Database::new();
        let id = add(&mut local, "With shared icon", "pw");
        local
            .entry_mut(id)
            .unwrap()
            .set_icon_custom_new(vec![1, 2, 3]);
        let remote = fork(&local);

        let merged = apply_picks(&local, &remote, &HashMap::new(), &diff(&local, &remote))
            .expect("identical custom-icon stores are safe to retain");
        assert_eq!(merged.num_custom_icons(), 1);
        assert_eq!(
            merged.entry(id).unwrap().custom_icon().unwrap().data,
            vec![1, 2, 3]
        );
    }

    #[test]
    fn three_way_round_trip_does_not_duplicate_entries() {
        // The end-to-end canary that pins the user-reported sync bug:
        // FerrisPass → cloud → KeePass2-style merge → cloud → FerrisPass
        // should leave the entry count stable. Pre-fix, this test failed
        // because UUID drift made each side treat the entry as new on
        // every cycle.

        // Round 1: KP2 creates an entry, cloud has it; FP local is empty.
        let local_fp = Database::new();
        let mut cloud = fork(&local_fp);
        add(&mut cloud, "TestKP", "secret");
        let kp2_local_after_round1 = cloud.clone();

        // FP merges. report.remote_only contains the new entry.
        let report = diff(&local_fp, &cloud);
        assert_eq!(
            report.remote_only.len(),
            1,
            "expected exactly one remote-only entry"
        );
        let merged_fp = apply_picks(&local_fp, &cloud, &HashMap::new(), &report)
            .expect("round-trip merge should succeed");
        assert_eq!(
            merged_fp.iter_all_entries().count(),
            1,
            "merged DB should have exactly the one entry, not more"
        );

        // FP uploads merged_fp; that's now the cloud state.
        let cloud_after_fp = merged_fp;

        // KP2 syncs against cloud_after_fp. KP2's local already had the
        // entry with its original UUID (because KP2 created it). With
        // UUID preservation, cloud_after_fp's entry has the *same* UUID,
        // so KP2's diff should be clean — no new entries to import,
        // no conflicts to resolve.
        let kp2_view = diff(&kp2_local_after_round1, &cloud_after_fp);
        assert!(
            kp2_view.is_clean(),
            "after FP merges with UUID preservation, KP2 should see a clean diff. \
             Got conflicts={:?} remote_only={:?} local_only={:?}",
            kp2_view.conflicts,
            kp2_view.remote_only,
            kp2_view.local_only,
        );

        // And the count stays at 1 across the round-trip.
        assert_eq!(
            cloud_after_fp.iter_all_entries().count(),
            1,
            "round-trip should preserve entry count, not multiply it"
        );
    }

    /// Regression: local has moved an entry to its recycle bin more recently
    /// than the remote copy was updated. The structural merge must retain the
    /// newer local location instead of resurrecting a second live copy.
    #[test]
    fn apply_picks_does_not_panic_when_remote_only_collides_with_local_bin() {
        use chrono::NaiveDate;

        let older = NaiveDate::from_ymd_opt(2025, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let newer = NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();

        let mut local = Database::new();
        let id = add(&mut local, "WasTrashed", "x");
        let mut remote = fork(&local);
        remote.entry_mut(id).unwrap().times.last_modification = Some(older);
        remote.entry_mut(id).unwrap().times.location_changed = Some(older);

        let bin_id = {
            let mut root = local.root_mut();
            let mut bin = root.add_group();
            bin.name = "Recycle Bin".into();
            let id = bin.id();
            drop(bin);
            drop(root);
            local.meta.recyclebin_uuid = Some(id.uuid());
            id
        };
        local.entry_mut(id).unwrap().move_to(bin_id).unwrap();
        local.entry_mut(id).unwrap().times.last_modification = Some(newer);
        local.entry_mut(id).unwrap().times.location_changed = Some(newer);

        let report = diff(&local, &remote);
        // Bug precondition: local has it in the bin (filtered), remote has
        // it live → diff classifies as remote_only.
        assert_eq!(report.remote_only.len(), 1);
        assert_eq!(report.remote_only[0].id, id.to_string());

        // Database::merge sees the same UUID on both sides and retains the
        // newer local location without adding another entry.
        let merged = apply_picks(&local, &remote, &HashMap::new(), &report)
            .expect("recycle-bin collision should merge without resurrection");
        // Entry still exists exactly once — in the recycle bin.
        let live_count = merged
            .iter_all_entries()
            .filter(|e| e.parent().id() != bin_id)
            .count();
        assert_eq!(
            live_count, 0,
            "trashed entry must not get resurrected by the import",
        );
        let bin_count = merged
            .iter_all_entries()
            .filter(|e| e.parent().id() == bin_id)
            .count();
        assert_eq!(bin_count, 1);
    }

    #[test]
    fn ordering_is_deterministic_alphabetical_by_title() {
        let local = Database::new();
        let mut remote = fork(&local);
        // Add in non-alphabetical order on remote
        add(&mut remote, "Zebra", "z");
        add(&mut remote, "Alpha", "a");
        add(&mut remote, "Mango", "m");

        let report = diff(&local, &remote);
        let titles: Vec<&str> = report
            .remote_only
            .iter()
            .map(|v| v.title.as_str())
            .collect();
        assert_eq!(titles, vec!["Alpha", "Mango", "Zebra"]);
    }
}

//! Pure-data diff and merge over two keepass `Database`s, used by the
//! sync conflict resolution flow. No GPUI dependencies - fully unit-testable.
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
//! - Passwords are compared in cleartext (necessarily - both sides are
//!   already decrypted) but the displayed `FieldDiff.local`/`.remote` for
//!   the Password row is redacted to `"••• (N chars)"` so the conflict
//!   screen is screen-sharing-safe.

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fmt,
    ops::{Deref, Not},
};

use chrono::NaiveDateTime;
use keepass::db::{
    AutoType, Color, CustomDataItem, CustomDataValue, CustomIcon, CustomIconId, Database, Entry,
    EntryId, EntryRef, GroupId, GroupRef, Icon, MemoryProtection, MergeWarning, Meta, Times, Value,
    fields,
};

use zeroize::Zeroizing;

use crate::domain::CustomField;
use crate::keepass::repository::{
    STANDARD_FIELDS, collect_custom_fields, find_entry_id, find_group_id,
};

/// Value snapshot of an entry at the moment of diffing - owned, no borrows
/// of the source `Database`. Safe to keep around in UI state for as long as
/// the user is reviewing the conflict.
///
/// Carries the full set of entry fields the merge round-trips, not just the
/// five visible-in-UI ones. When the user picks "Remote" for a conflict, all
/// these fields get transplanted onto the local entry - partial transplants
/// were the source of a silent-data-loss bug pre-v0.2.1.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct EntryView {
    /// EntryId stringified to its UUID. Stable across diff/apply, and
    /// re-hydrated via `EntryId::from_uuid` when adding remote-only entries
    /// to the merged DB so cross-client sync keeps the same identity.
    pub id: String,
    pub title: String,
    pub username: String,
    /// Cleartext, and wiped when the view is dropped. A conflict report holds
    /// two of these per diverged entry for as long as the overlay is open, so
    /// leaving the buffer behind for the allocator to hand out was the one
    /// long-lived copy of the password outside the database itself.
    pub password: Zeroizing<String>,
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

/// Same rule as `EntryDraft::password`: a conflict report holds two of these
/// per diverged entry for as long as the overlay is open.
const _: fn(&EntryView) = |view| {
    fn wipes_on_drop(_: &Zeroizing<String>) {}
    wipes_on_drop(&view.password);
};

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
    attachments: Vec<AttachmentFingerprint>,
    icon: IconShown<'static>,
    quality_check: Option<bool>,
    /// Where the entry sits, and when it was put there. Neither is in the
    /// field map, and both are decided on their own clock. The readable form
    /// is built at the conflict, where both sides are known and two places
    /// that read alike can be told apart.
    parent: GroupId,
    location_changed: Option<NaiveDateTime>,
    /// Only a date in force. One that is not says nothing, here as everywhere.
    expiry: Option<NaiveDateTime>,
}

#[derive(Clone, PartialEq, Eq)]
struct AttachmentFingerprint {
    name: String,
    protected: bool,
    data: Vec<u8>,
}

/// One field's local-vs-remote comparison. `local` and `remote` are the
/// strings the UI should render directly - for the Password row those are
/// pre-redacted; for the rest they're the cleartext field values.
#[derive(Clone, PartialEq, Eq)]
pub struct FieldDiff {
    /// Set when the two sides differ and their rendered values were equal, so
    /// the row had to name the sides to tell them apart. A renderer that
    /// builds its own text for this row, as the overlay does for passwords,
    /// has to do the same or it shows one string for two different values.
    pub sides_read_alike: bool,
    /// Owned where it has to be: a custom data key is part of the row's
    /// identity and is not known at compile time.
    pub label: Cow<'static, str>,
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

/// A group whose user-authored content diverged in a way no timestamp can
/// settle: both sides carry the same modification time, one carries none, or
/// one carries a time nobody could have written yet.
///
/// Groups have no last-write-wins path of their own. The fork merges them by
/// timestamp, which is exactly what a tie or a forged future date defeats, so
/// these have to reach the user the same way entry conflicts do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupConflict {
    pub id: String,
    /// The local name, which is what the row is labelled with.
    pub name: String,
    /// Ancestor names, root first. Two groups can share a name, and the user
    /// has to know which one they are deciding about.
    pub path: Vec<String>,
    pub fields: Vec<FieldDiff>,
}

/// The database's own settings, when the two copies disagree and no change
/// time can rank them.
///
/// There is one of these per merge, not one per object, so it has no id. Like
/// a group it has no history to fall back on: whichever side is not chosen is
/// gone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataConflict {
    pub fields: Vec<FieldDiff>,
}

/// The user's choices, by kind.
///
/// Two maps rather than one: entry and group ids are independent id spaces,
/// and a lookup for one must never be answered by the other.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolutions {
    pub entries: HashMap<String, Side>,
    pub groups: HashMap<String, Side>,
    pub metadata: Option<Side>,
}

impl Resolutions {
    pub fn for_entries(entries: HashMap<String, Side>) -> Self {
        Self {
            entries,
            groups: HashMap::new(),
            metadata: None,
        }
    }
}

/// One entry that diverged but was auto-resolved by `last_modification`
/// timestamp - the side with the strictly newer timestamp wins, no UI
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
    /// side's `last_modification` is strictly newer - last-write-wins,
    /// applied silently.
    pub auto_resolved: Vec<AutoResolved>,
    /// Group topology/metadata, tombstones, or recycle-bin metadata differ,
    /// or an entry moved. Direction attribution is deliberately conservative:
    /// writing the merged result back may create a redundant remote version,
    /// but skipping it could strand a local deletion or empty-group change.
    pub structural_writeback_required: bool,
    /// Entries where one side's `last_modification` sits implausibly far in
    /// the future. Those reach the overlay because nothing can be decided
    /// from a timestamp nobody could have written yet, and the user deserves
    /// to be told that rather than left wondering why a decision is needed.
    pub future_dated: Vec<String>,
    /// This copy holds entry history versions the remote does not, so the
    /// merged result has something to upload even when every current field
    /// already matches. Editing an entry and reverting it produces exactly
    /// that: the intermediate version exists only here.
    pub local_history_ahead: bool,
    /// The remote holds history versions this copy does not, so the merged
    /// result has something to write to disk even when no current field
    /// changed.
    pub remote_history_ahead: bool,
    /// Groups the user must decide, for the same reason entries reach
    /// `conflicts`: their content diverged and no timestamp can rank them.
    pub group_conflicts: Vec<GroupConflict>,
    /// The database's own settings, when they diverged and no change time can
    /// rank them. Set at most once per merge.
    pub metadata_conflict: Option<MetadataConflict>,
}

impl ConflictReport {
    /// True when the diff produced nothing at all: no decision, no remote
    /// pull, no writeback. Production splits this into the two finer checks
    /// it actually needs (`conflicts.is_empty()` gates the overlay,
    /// [`Self::has_local_contribution`] gates the upload), so this stays a
    /// test predicate rather than a third, subtly different rule.
    #[cfg(test)]
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
            && self.group_conflicts.is_empty()
            && self.metadata_conflict.is_none()
            && self.remote_only.is_empty()
            && self.auto_resolved.is_empty()
            && !self.structural_writeback_required
            && !self.local_history_ahead
            && !self.remote_history_ahead
        // `local_only` doesn't dirty the merge: those entries are already in
        // the local DB we'll start the merge from.
    }

    /// True when this merge cannot be applied without asking the user.
    ///
    /// Both kinds count. Applying a report with an unanswered conflict of
    /// either kind takes the default side and uploads it, which is the
    /// silent loss the conflict screen exists to prevent.
    pub fn needs_a_decision(&self) -> bool {
        !self.conflicts.is_empty()
            || !self.group_conflicts.is_empty()
            || self.metadata_conflict.is_some()
    }

    /// True when applying this report changes the *remote* - i.e. the local
    /// side contributes something the server doesn't already have. That's
    /// either entries only we hold (`local_only`) or a field divergence our
    /// side won (`auto_resolved` with `winner == Local`).
    ///
    /// When this is false the merge is a pure fast-forward (we only pulled
    /// remote-side changes), so the post-merge DB already matches the server
    /// and the caller can skip the upload - avoiding a redundant remote
    /// version for what is really just someone else's change landing here.
    pub fn has_local_contribution(&self) -> bool {
        self.structural_writeback_required
            || self.local_history_ahead
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

impl Side {
    /// How a row names this side when nothing else can tell the two apart.
    /// Deliberately free of anything derived from the value: a digest of what
    /// the row is hiding would be an oracle against exactly that.
    pub fn own_words(self) -> &'static str {
        match self {
            Self::Local => "this copy",
            Self::Remote => "other copy",
        }
    }
}

/// What a conflict row is about. Entry and group ids are separate spaces, so
/// a pick has to say which one it means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConflictKind {
    Entry,
    Group,
    /// The database's own settings. There is one of these, so it needs no id.
    Metadata,
}

/// A merge was refused because it could not be completed without either data
/// loss or guessing. Callers should keep both original databases untouched and
/// surface the error as a sync conflict/failure.
#[derive(Debug, thiserror::Error)]
pub enum ApplyError {
    #[error("custom icon {id} is referenced by the merged vault but its image is missing")]
    CustomIconUnrecoverable { id: String },
    #[error("cannot merge databases with different root group UUIDs")]
    DifferentRoots,
    #[error("{side:?} entry {id} referenced by the conflict report no longer exists")]
    EntryMissing { id: String, side: Side },
    #[error("keepass database merge failed: {0}")]
    DatabaseMerge(String),
    #[error("keepass database merge completed with unresolved warnings: {0}")]
    DatabaseMergeWarnings(String),
    #[error("{side:?} group {id} referenced by the conflict report no longer exists")]
    GroupMissing { id: String, side: Side },
}

/// Build a `ConflictReport` between two unlocked databases.
pub fn diff(local: &Database, remote: &Database) -> ConflictReport {
    let local_map = live_entries(local);
    let remote_map = live_entries(remote);

    let local_ids: HashSet<&String> = local_map.keys().collect();
    let remote_ids: HashSet<&String> = remote_map.keys().collect();

    let mut conflicts = Vec::new();
    let mut auto_resolved = Vec::new();
    let mut future_dated = Vec::new();
    for id in local_ids.intersection(&remote_ids) {
        let l = &local_map[*id];
        let r = &remote_map[*id];
        let mut fields = field_diffs(l, r);
        let content_differs = fields.iter().any(|f| f.differs);
        let moved = l.parent != r.parent;
        if !content_differs && !moved {
            continue;
        }
        // KeePass-style last-write-wins: when one side's `last_modification`
        // is strictly newer, take that side automatically. The overlay is
        // reserved for the genuinely ambiguous cases (timestamps tied or
        // missing) - pre-v0.4 every field-level divergence forced a prompt
        // even when the user had clearly saved one side later than the
        // other, which made benign sync round-trips noisy.
        // Two clocks, each deciding its own thing, exactly as for a group.
        // Content is ranked by `last_modification`, the placement by
        // `location_changed`, and a tie on either is the user's to settle:
        // the fork refuses to rank a tied move and refuses the whole merge
        // when a tied entry diverges, so falling through to the automatic
        // path meant "Merge blocked" with nothing to answer.
        let content_winner = timestamp_winner(l.view.modified, r.view.modified);
        let content_tied = content_differs && content_winner.is_none();
        let location_tied =
            moved && timestamp_winner(l.location_changed, r.location_changed).is_none();
        if !content_tied && !location_tied {
            if let Some(winner) = content_winner.filter(|_| content_differs) {
                auto_resolved.push(AutoResolved {
                    id: (*id).clone(),
                    winner,
                    remote: r.view.clone(),
                });
            }
            continue;
        }
        if [l.view.modified, r.view.modified]
            .into_iter()
            .flatten()
            .any(is_future_dated)
        {
            future_dated.push((*id).clone());
        }
        if moved {
            let (here, there) = location_pair(local, remote, l.parent, r.parent);
            fields.insert(
                0,
                FieldDiff {
                    sides_read_alike: false,
                    label: "Location".into(),
                    local: here,
                    remote: there,
                    differs: true,
                },
            );
        }
        conflicts.push(EntryConflict {
            id: (*id).clone(),
            local: l.view.clone(),
            remote: r.view.clone(),
            fields,
        });
    }

    let group_conflicts = group_conflicts(local, remote);
    let metadata_conflict = metadata_conflict(local, remote);
    let history = history_divergence(local, remote);

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
    future_dated.sort();

    ConflictReport {
        conflicts,
        future_dated,
        local_only,
        remote_only,
        auto_resolved,
        structural_writeback_required: structural_state_differs(local, remote),
        local_history_ahead: history.local_ahead,
        remote_history_ahead: history.remote_ahead,
        group_conflicts,
        metadata_conflict,
    }
}

/// The database's own settings when the two copies disagree and no change
/// time can rank them.
///
/// Every field here is a scalar with no history, so a tie cannot be merged:
/// one of the two values is going to be gone whatever happens. Keeping this
/// copy's silently was the old behaviour, and the next upload then wrote it
/// over theirs. Asking is the only outcome that loses nothing on its own.
fn metadata_conflict(local: &Database, remote: &Database) -> Option<MetadataConflict> {
    // Per field, not per block. A description edited here does not make our
    // history limit newer than theirs, and asking the wrong clock is how a
    // tied name got uploaded over the other side on the strength of an
    // unrelated edit.
    let fields: Vec<FieldDiff> = meta_divergences(local, remote)
        .into_iter()
        .filter(|divergence| divergence.winner.is_none())
        .map(|divergence| FieldDiff {
            sides_read_alike: false,
            differs: true,
            label: divergence.label,
            local: divergence.local,
            remote: divergence.remote,
        })
        .collect();
    fields
        .is_empty()
        .not()
        .then_some(MetadataConflict { fields })
}

fn structural_state_differs(local: &Database, remote: &Database) -> bool {
    if local.deleted_objects != remote.deleted_objects || meta_local_contributes(local, remote) {
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
            || !group_icons_equivalent(&local_group, &remote_group)
            || local_group.custom_data != remote_group.custom_data
            || local_group.is_expanded != remote_group.is_expanded
            || local_group.default_autotype_sequence != remote_group.default_autotype_sequence
            || local_group.enable_autotype != remote_group.enable_autotype
            || local_group.enable_searching != remote_group.enable_searching
            || !previous_groups_equivalent(
                local_group.previous_parent_group,
                remote_group.previous_parent_group,
            )
            || local_group.tags != remote_group.tags
            || !expiry_equivalent(&local_group.times, &remote_group.times)
        {
            return true;
        }
    }

    for local_entry in local.iter_all_entries() {
        let Some(remote_entry) = remote.entry(local_entry.id()) else {
            continue;
        };
        // Relocation, and expiry. Expiry lives in `times` rather than in the
        // field map, so `field_diffs` never saw it and an entry whose only
        // change was its expiry date read as no contribution at all: it was
        // never uploaded, and the next edit from the other side erased it.
        //
        // Histories are compared separately and directionally by
        // `history_divergence`: comparing them as one undirected "differs"
        // bit made `has_local_contribution` unconditionally true, because
        // trimming alone diverges them.
        if local_entry.parent().id() != remote_entry.parent().id()
            || !expiry_equivalent(&local_entry.times, &remote_entry.times)
        {
            return true;
        }
    }

    custom_icon_name_is_ours(local, remote)
}

/// Whether this copy holds the newer display name for an icon both copies
/// have.
///
/// The name and its modification time sit beside the image in KDBX 4.1, and
/// nothing else here looks at them: renaming an icon changed no entry, no
/// group and no setting, so it was never reported as something to send and
/// the other copy's next upload put the old name back.
///
/// Only the name this copy can be shown to have written last counts, the same
/// rule the merge applies to it. An image only one side has travels with the
/// reference to it and needs no separate mention.
fn custom_icon_name_is_ours(local: &Database, remote: &Database) -> bool {
    local.iter_all_custom_icons().any(|ours| {
        let Some(theirs) = remote.custom_icon(ours.id()) else {
            return false;
        };
        if ours.data != theirs.data || ours.name == theirs.name {
            return false;
        }
        match (ours.last_modification_time, theirs.last_modification_time) {
            (Some(ours), Some(theirs)) => ours > theirs,
            (Some(_), None) => true,
            _ => false,
        }
    })
}

/// Whether two objects say the same thing about expiry.
///
/// An object that does not expire has no expiry date, whatever value happens
/// to sit in the field. KeePassXC writes an explicit far-future date on
/// objects it marks as never expiring, and does so without touching their
/// modification time, so comparing the raw pair made every sync after a
/// KeePassXC save look like a local change, forever. Only a date that is in
/// force counts, and an absent `expires` means the same as `false`.
fn expiry_equivalent(local: &Times, remote: &Times) -> bool {
    fn in_force(times: &Times) -> Option<NaiveDateTime> {
        times
            .expires
            .unwrap_or(false)
            .then_some(times.expiry)
            .flatten()
    }
    in_force(local) == in_force(remote)
}

/// One database setting the two copies disagree about, paired with the change
/// time that decides it.
///
/// The pairing is the whole point. KDBX dates some of these fields
/// individually, leaves the rest to `SettingsChanged`, and dates each custom
/// data item on its own, and the merge follows exactly that. Asking any other
/// clock answers a question the merge is not asking: a description edited
/// here does not make our database name newer than theirs.
struct MetaDivergence {
    field: MetaField,
    label: Cow<'static, str>,
    local: String,
    remote: String,
    /// The side the merge takes this from, or `None` when nothing here can
    /// rank the two and the choice is the user's.
    winner: Option<Side>,
}

/// What the merge decides in one go. Several rows share `Settings`: KDBX
/// gives those fields no change time of their own, so they move together.
#[derive(Clone, Debug, PartialEq, Eq)]
enum MetaField {
    Name,
    Description,
    DefaultUsername,
    RecycleBin,
    EntryTemplatesGroup,
    Settings,
    CustomData(String),
    /// One custom icon's display name, by icon id. The image itself travels
    /// with the reference to it; only the name can diverge on its own.
    CustomIcon(String),
}

/// Every database setting the two copies disagree about, with the side the
/// merge will take it from.
///
/// `generator` is excluded because every client writes its own name into it,
/// and `last_selected_group` and `last_top_visible_group` because they follow
/// the cursor: comparing those would demand an upload after every sync.
/// `master_key_changed` describes the key the file is encrypted with, which
/// both copies share.
fn meta_divergences(local: &Database, remote: &Database) -> Vec<MetaDivergence> {
    // Strictly newer wins, and a side that dates its change against one that
    // does not is the side that recorded making it. Same rule as the fork's
    // settings merge, so the winner named here is the value that merge keeps.
    fn rank(ours: Option<NaiveDateTime>, theirs: Option<NaiveDateTime>) -> Option<Side> {
        // A time nobody could have written yet is a claim, not a record.
        // Anyone who can write the shared file could set one, so it decides
        // nothing here either: the field becomes the user's to settle, and
        // `force_metadata_winner` then stamps a time we do believe. Same rule
        // as `timestamp_winner`, one level up.
        if [ours, theirs].into_iter().flatten().any(is_future_dated) {
            return None;
        }
        match (ours, theirs) {
            (Some(ours), Some(theirs)) if ours > theirs => Some(Side::Local),
            (Some(ours), Some(theirs)) if theirs > ours => Some(Side::Remote),
            (Some(_), None) => Some(Side::Local),
            (None, Some(_)) => Some(Side::Remote),
            _ => None,
        }
    }

    fn number<T: fmt::Display>(value: Option<T>) -> String {
        value.map(|value| value.to_string()).unwrap_or_default()
    }
    fn group_name(database: &Database, id: Option<uuid::Uuid>) -> String {
        let Some(id) = normalised_group_uuid(id) else {
            return "None".into();
        };
        database
            .iter_all_groups()
            .find(|group| group.id().uuid() == id)
            .map_or_else(|| id.to_string(), |group| group.name.clone())
    }
    fn recycle_bin(database: &Database) -> String {
        let state = if database.meta.recyclebin_enabled.unwrap_or(false) {
            "On"
        } else {
            "Off"
        };
        // Two copies can each have made their own bin, and both are called
        // "Recycle Bin". Without the id the row showed two identical cells.
        match normalised_group_uuid(database.meta.recyclebin_uuid) {
            Some(id) => format!("{state}, {} ({id})", group_name(database, Some(id))),
            None => format!("{state}, no group"),
        }
    }
    // An absent block means the format's defaults, which the reader applies
    // when it parses one, so a file that omits it and a file that writes
    // those same values say the same thing.
    fn protection(meta: &Meta) -> MemoryProtection {
        meta.memory_protection.clone().unwrap_or_default()
    }
    fn protected_fields(protection: &MemoryProtection) -> String {
        [
            (protection.protect_title, "Title"),
            (protection.protect_username, "Username"),
            (protection.protect_password, "Password"),
            (protection.protect_url, "URL"),
            (protection.protect_notes, "Notes"),
        ]
        .into_iter()
        .filter_map(|(on, name)| on.then_some(name))
        .collect::<Vec<_>>()
        .join(", ")
    }

    // The rendered value is built only for a field that differs: naming a
    // group walks every group in the file, and a diff runs on every sync tick.
    fn push(
        out: &mut Vec<MetaDivergence>,
        field: MetaField,
        label: Cow<'static, str>,
        differs: bool,
        values: impl FnOnce() -> (String, String),
    ) {
        if !differs {
            return;
        }
        let (local, remote) = values();
        // The same last resort the entry and group rows use: a settings row
        // that differs must not show the same text twice.
        let (local, remote) = if local == remote {
            (
                format!("{local} (this copy)"),
                format!("{remote} (other copy)"),
            )
        } else {
            (local, remote)
        };
        out.push(MetaDivergence {
            field,
            label,
            local,
            remote,
            winner: None,
        });
    }

    let (a, b) = (&local.meta, &remote.meta);
    let mut out = Vec::new();

    push(
        &mut out,
        MetaField::Name,
        "Database name".into(),
        a.database_name != b.database_name,
        || {
            (
                unset_or(a.database_name.as_ref()),
                unset_or(b.database_name.as_ref()),
            )
        },
    );
    push(
        &mut out,
        MetaField::Description,
        "Description".into(),
        a.database_description != b.database_description,
        || {
            (
                unset_or(a.database_description.as_ref()),
                unset_or(b.database_description.as_ref()),
            )
        },
    );
    push(
        &mut out,
        MetaField::DefaultUsername,
        "Default username".into(),
        a.default_username != b.default_username,
        || {
            (
                unset_or(a.default_username.as_ref()),
                unset_or(b.default_username.as_ref()),
            )
        },
    );
    // Both values move on one clock. An absent `RecycleBinEnabled` means off,
    // and a nil uuid and an absent element both mean there is no such group:
    // clients disagree about which to write, and comparing the raw pair made
    // a KeePassXC round trip look like a settings change on every sync.
    push(
        &mut out,
        MetaField::RecycleBin,
        "Recycle bin".into(),
        a.recyclebin_enabled.unwrap_or(false) != b.recyclebin_enabled.unwrap_or(false)
            || !group_uuids_equivalent(a.recyclebin_uuid, b.recyclebin_uuid),
        || (recycle_bin(local), recycle_bin(remote)),
    );
    push(
        &mut out,
        MetaField::EntryTemplatesGroup,
        "Entry templates group".into(),
        !group_uuids_equivalent(a.entry_templates_group, b.entry_templates_group),
        || {
            // Two groups can share a name, on one side or across the two.
            named_pair(
                group_name(local, a.entry_templates_group),
                group_name(remote, b.entry_templates_group),
                a.entry_templates_group,
                b.entry_templates_group,
            )
        },
    );
    // Through the same reading the rest of the module uses, so a file that
    // leaves a limit to the format and one that writes the format's own value
    // are not asked about: KeePassXC writes both out, and comparing the raw
    // options made a round trip through it a prompt with nothing at stake.
    push(
        &mut out,
        MetaField::Settings,
        "History entries kept".into(),
        crate::keepass::document::history_cap(local)
            != crate::keepass::document::history_cap(remote),
        || (number(a.history_max_items), number(b.history_max_items)),
    );
    push(
        &mut out,
        MetaField::Settings,
        "History size kept".into(),
        crate::keepass::document::history_size_cap(local)
            != crate::keepass::document::history_size_cap(remote),
        || (number(a.history_max_size), number(b.history_max_size)),
    );
    push(
        &mut out,
        MetaField::Settings,
        "Colour".into(),
        a.color != b.color,
        || {
            (
                number(a.color.as_ref().map(Color::to_string)),
                number(b.color.as_ref().map(Color::to_string)),
            )
        },
    );
    push(
        &mut out,
        MetaField::Settings,
        "Days of history kept".into(),
        a.maintenance_history_days != b.maintenance_history_days,
        || {
            (
                number(a.maintenance_history_days),
                number(b.maintenance_history_days),
            )
        },
    );
    // An absent block is not "nothing protected": the reader substitutes the
    // format's defaults, which protect the password. Comparing the raw pair
    // asked the user about a difference neither file actually holds.
    push(
        &mut out,
        MetaField::Settings,
        "Protected fields".into(),
        protection(a) != protection(b),
        || {
            (
                protected_fields(&protection(a)),
                protected_fields(&protection(b)),
            )
        },
    );
    push(
        &mut out,
        MetaField::Settings,
        "Key change reminder".into(),
        a.master_key_change_rec != b.master_key_change_rec,
        || {
            (
                number(a.master_key_change_rec),
                number(b.master_key_change_rec),
            )
        },
    );
    push(
        &mut out,
        MetaField::Settings,
        "Key change enforcement".into(),
        a.master_key_change_force != b.master_key_change_force,
        || {
            (
                number(a.master_key_change_force),
                number(b.master_key_change_force),
            )
        },
    );

    // A custom icon's display name diverges on its own: the image decides
    // nothing about it, and nothing else in this comparison looks at it.
    for ours in local.iter_all_custom_icons() {
        let Some(theirs) = remote.custom_icon(ours.id()) else {
            continue;
        };
        if ours.data != theirs.data {
            continue;
        }
        push(
            &mut out,
            MetaField::CustomIcon(ours.id().to_string()),
            // The id, which the file already carries, rather than anything
            // derived from the image: a digest of vault content on screen is
            // an oracle against that content.
            format!("Icon name ({})", ours.id()).into(),
            ours.name != theirs.name,
            || (unset_or(ours.name.as_ref()), unset_or(theirs.name.as_ref())),
        );
    }

    let (ours, theirs) = (user_custom_data(a), user_custom_data(b));
    let mut keys: Vec<&String> = ours.keys().chain(theirs.keys()).copied().collect();
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        let (ours, theirs) = (ours.get(key), theirs.get(key));
        push(
            &mut out,
            MetaField::CustomData(key.clone()),
            format!("Plugin data \"{key}\"").into(),
            ours.map(|item| &item.value) != theirs.map(|item| &item.value),
            || (custom_data_value(ours), custom_data_value(theirs)),
        );
    }

    // The winners in one pass at the end, so every row states the clock it
    // was decided on right next to the values it decides between.
    for divergence in &mut out {
        divergence.winner = match &divergence.field {
            MetaField::Name => rank(a.database_name_changed, b.database_name_changed),
            MetaField::Description => rank(
                a.database_description_changed,
                b.database_description_changed,
            ),
            MetaField::DefaultUsername => {
                rank(a.default_username_changed, b.default_username_changed)
            }
            MetaField::RecycleBin => recycle_bin_winner(
                local,
                remote,
                rank(a.recyclebin_changed, b.recyclebin_changed),
            ),
            MetaField::EntryTemplatesGroup => rank(
                a.entry_templates_group_changed,
                b.entry_templates_group_changed,
            ),
            MetaField::Settings => rank(a.settings_changed, b.settings_changed),
            // A key only one side has is not a disagreement about a value.
            // The merge unions custom data, so that side simply keeps it.
            MetaField::CustomData(key) => match (ours.get(key), theirs.get(key)) {
                (Some(_), None) => Some(Side::Local),
                (None, Some(_)) => Some(Side::Remote),
                (ours, theirs) => rank(
                    ours.and_then(|item| item.last_modification_time),
                    theirs.and_then(|item| item.last_modification_time),
                ),
            },
            MetaField::CustomIcon(id) => rank(
                custom_icon_with_id(local, id).and_then(|icon| icon.last_modification_time),
                custom_icon_with_id(remote, id).and_then(|icon| icon.last_modification_time),
            ),
        };
    }
    out
}

/// A custom data value as one line. Binary plugin data is described rather
/// than rendered: it is not text, and the row exists so a person can tell two
/// values apart.
/// Redacted like a password, because that is what it may be.
///
/// Plugin data is opaque to this application: a browser integration keeps key
/// associations there, and anything else a plugin wants to keep. Rendering it
/// verbatim on a conflict screen put whatever that is in front of whoever is
/// looking at it, and this screen is the one place that already shows two
/// vaults side by side. The size is shown for the same reason it is for a
/// password: it is enough to see that the two sides hold something different.
fn custom_data_value(item: Option<&&CustomDataItem>) -> String {
    match item.and_then(|item| item.value.as_ref()) {
        Some(CustomDataValue::String(value)) => redact(value),
        Some(CustomDataValue::Binary(bytes)) => format!("••• ({} bytes)", bytes.len()),
        None => String::new(),
    }
}

/// The bin decision, unless following it would quietly change what the user
/// sees.
///
/// The clock says which copy decided last, and usually that is the end of it.
/// But when the other copy's bin still holds entries and the deciding copy
/// has that group as well, nothing in either file says which of two things
/// happened: the deciding copy retired that group and has been using it as an
/// ordinary one since, or the two bins met in some earlier merge and it
/// simply never designated the other. Following the clock deletes a live
/// archive in the first case and resurrects deletions in the second, both
/// without a word, so it is asked instead.
fn recycle_bin_winner(local: &Database, remote: &Database, ranked: Option<Side>) -> Option<Side> {
    let (winner, loser) = match ranked? {
        Side::Local => (local, remote),
        Side::Remote => (remote, local),
    };
    let Some(displaced) = normalised_group_uuid(loser.meta.recyclebin_uuid) else {
        return ranked;
    };
    if normalised_group_uuid(winner.meta.recyclebin_uuid) == Some(displaced) {
        return ranked;
    }
    let Some(group) = group_with_uuid(winner, displaced) else {
        // Never seen there, so it was made here on its own and there is
        // nothing to interpret.
        return ranked;
    };
    // Nothing is at stake when that group is empty, and none either when the
    // deciding copy already has it inside its own bin: both copies call its
    // contents deleted, which is the state an earlier reunion leaves behind.
    let settled = !holds_entries(winner, group)
        || normalised_group_uuid(winner.meta.recyclebin_uuid)
            .and_then(|bin| group_with_uuid(winner, bin))
            .is_some_and(|bin| crate::keepass::document::group_is_within(winner, group, bin));
    if settled { ranked } else { None }
}

/// Whether anything sits at or below this group.
fn holds_entries(database: &Database, group: GroupId) -> bool {
    database.iter_all_entries().any(|entry| {
        crate::keepass::document::group_is_within(database, entry.parent().id(), group)
    })
}

/// Whether the merged settings hold anything of ours that the remote copy
/// does not, and so have to be uploaded.
///
/// Only a field this copy can be shown to have changed last counts. A tie is
/// not evidence: reporting one as ours to send wrote our value over theirs
/// and their edit was gone. Ties reach the user through
/// [`metadata_conflict`] instead.
fn meta_local_contributes(local: &Database, remote: &Database) -> bool {
    meta_divergences(local, remote)
        .iter()
        .any(|divergence| divergence.winner == Some(Side::Local))
}

/// Metadata custom data minus the keys clients regenerate on every save.
///
/// KeePassXC rewrites `KPXC_RANDOM_SLUG` and `_LAST_MODIFIED` each time it
/// writes the file and treats both as generated rather than user data. Left
/// in the comparison they made every sync after a KeePassXC save look like a
/// local change and ask for an upload, forever.
fn user_custom_data(meta: &Meta) -> HashMap<&String, &CustomDataItem> {
    const GENERATED: [&str; 2] = ["KPXC_RANDOM_SLUG", "_LAST_MODIFIED"];
    meta.custom_data
        .iter()
        .filter(|(key, _)| !GENERATED.contains(&key.as_str()))
        .collect()
}

/// What a vault asks its histories to stay inside. Both limits are KeePass's,
/// and either can be the one that trimmed a version away.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct HistoryLimits {
    items: Option<usize>,
    bytes: Option<usize>,
}

/// The smallest version the other side lacks, as a size. Their history counts
/// as full when it could not take even that one.
fn smallest_missing(mine: &BTreeSet<HistoryVersion>, theirs: &BTreeSet<HistoryVersion>) -> usize {
    mine.iter()
        .filter(|version| !theirs.contains(version))
        .map(|version| version.bytes)
        .min()
        .unwrap_or(0)
}

/// Which side holds entry history versions the other does not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct HistoryDivergence {
    local_ahead: bool,
    remote_ahead: bool,
}

/// Compare histories per entry and per direction.
///
/// Two things diverge histories, and only one of them is somebody's edit.
/// `enforce_history_limits` drops the *oldest* versions to this vault's
/// `HistoryMaxItems`, and another client keeps a different number, so a
/// version the other side no longer holds may simply have been trimmed there.
/// A version that falls inside the range the other side still retains is one
/// it never saw.
///
/// Getting this wrong costs data either way. As one undirected bit it was
/// always true, so no pull was ever a fast-forward; dropped entirely, an
/// entry edited and then reverted looked identical to the remote copy and its
/// intermediate version was never uploaded.
fn history_divergence(local: &Database, remote: &Database) -> HistoryDivergence {
    let limits = |db| HistoryLimits {
        items: crate::keepass::document::history_cap(db),
        bytes: crate::keepass::document::history_size_cap(db),
    };
    let (local_cap, remote_cap) = (limits(local), limits(remote));
    let mut divergence = HistoryDivergence::default();
    for local_entry in local.iter_all_entries() {
        let Some(remote_entry) = remote.entry(local_entry.id()) else {
            continue;
        };
        let local_versions = history_versions(&local_entry);
        let remote_versions = history_versions(&remote_entry);
        divergence.local_ahead |=
            holds_a_version_the_other_side_never_saw(&local_versions, &remote_versions, remote_cap);
        divergence.remote_ahead |=
            holds_a_version_the_other_side_never_saw(&remote_versions, &local_versions, local_cap);
        if divergence.local_ahead && divergence.remote_ahead {
            break;
        }
    }
    divergence
}

/// True when `theirs` is missing one of `mine` that they cannot have trimmed
/// away.
///
/// Trimming drops the oldest versions once a history reaches the vault's
/// `HistoryMaxItems`, so a version older than everything they still hold
/// *might* be one they discarded. Might is not enough: a version they simply
/// never received looks identical, and reading that as trimming meant it was
/// never uploaded and the only copy stayed on one machine.
///
/// So trimming has to be established, not assumed. `their_cap` is their own
/// limit, and only a history that has actually reached it can have dropped
/// anything. When it has not, every version they lack is one they never saw.
/// The cost of being wrong the other way is a redundant upload, which is the
/// side to err on.
fn holds_a_version_the_other_side_never_saw(
    mine: &BTreeSet<HistoryVersion>,
    theirs: &BTreeSet<HistoryVersion>,
    their_cap: HistoryLimits,
) -> bool {
    // Either limit can be the one that filled up. KeePass trims by both, and
    // reading only the item count meant a copy that had trimmed by size,
    // while still under its item limit, looked like one that had never seen
    // those versions: we sent them again, it dropped them again, on every
    // sync.
    let held: usize = theirs.iter().map(|version| version.bytes).sum();
    // A client trims when the history goes *over* its budget, so one sitting
    // exactly on it is not full: they are full when the smallest version they
    // lack would not have fitted.
    let they_are_full = their_cap.items.is_some_and(|cap| theirs.len() >= cap)
        || their_cap
            .bytes
            .is_some_and(|cap| held.saturating_add(smallest_missing(mine, theirs)) > cap);
    let trim_horizon = match (they_are_full, theirs.first()) {
        // Full, so anything older than the oldest they kept is something they
        // dropped.
        (true, Some(oldest)) => Some(oldest.at),
        // Full while holding nothing: a `HistoryMaxItems` of zero. They keep
        // no versions at all, so every version is one they would have
        // trimmed, and none of mine is evidence of anything.
        (true, None) => return false,
        // Room to spare, so they trimmed nothing and every version they lack
        // is one they never saw.
        (false, _) => None,
    };
    mine.iter().any(|version| {
        !theirs.contains(version) && trim_horizon.is_none_or(|oldest| version.at > oldest)
    })
}

/// One archived version, identified the way the fork's history merge treats
/// them: by `last_modification`, plus a digest of the content so two
/// different versions written in the same second stay two versions. Keying on
/// the timestamp alone collapsed them into one, and neither side then looked
/// ahead of the other.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct HistoryVersion {
    at: NaiveDateTime,
    content: [u8; 32],
    /// Roughly what this version costs in the file. Not part of its identity
    /// in any meaningful sense: `content` already digests everything the
    /// version holds, so two versions that agree there agree here too. It
    /// rides along because the trimming question needs it.
    bytes: usize,
}

fn history_versions(entry: &EntryRef<'_>) -> BTreeSet<HistoryVersion> {
    // Through `historical`, not the raw `Entry`s, because an archived version
    // owns attachments and an icon that only a reference can resolve.
    (0..entry.history.iter().map(|h| h.get_entries().len()).sum())
        .filter_map(|index| entry.historical(index))
        .map(|version| HistoryVersion {
            // The fork substitutes the epoch for a version with no timestamp
            // and unions it like any other, so skipping those here made this
            // comparison disagree with the merge that follows it: the merge
            // added a version the report had said was not there, and a caller
            // that trusted the report wrote without it.
            at: version.times.last_modification.unwrap_or_else(Times::epoch),
            content: history_version_digest(&version),
            bytes: history_version_bytes(&version),
        })
        .collect()
}

/// Roughly what an archived version costs in the file: its field values and
/// its attachments. KeePass measures the serialized XML, which nothing here
/// reproduces exactly, so this is an estimate and is only ever used to ask
/// whether the other copy could still be holding a version, never to drop
/// one.
fn history_version_bytes(version: &EntryRef<'_>) -> usize {
    let fields: usize = version
        .fields
        .iter()
        .map(|(key, value)| key.len() + value.get().len())
        .sum();
    let attachments: usize = version
        .named_attachments()
        .map(|(name, attachment)| name.len() + attachment.data.get().len())
        .sum();
    // Everything else a version serializes and a client counts. Leaving these
    // out meant a history trimmed because of them still read as one that had
    // never seen those versions.
    let custom_data: usize = version
        .custom_data
        .iter()
        .map(|(key, item)| {
            key.len()
                + match &item.value {
                    Some(CustomDataValue::String(value)) => value.len(),
                    Some(CustomDataValue::Binary(bytes)) => bytes.len(),
                    None => 0,
                }
        })
        .sum();
    let tags: usize = version.tags.iter().map(String::len).sum();
    let autotype = version.autotype.as_ref().map_or(0, |autotype| {
        autotype.default_sequence.as_ref().map_or(0, String::len)
            + autotype
                .associations
                .iter()
                .map(|association| association.window.len() + association.sequence.len())
                .sum::<usize>()
    });
    let override_url = version.override_url.as_ref().map_or(0, String::len);
    fields + attachments + custom_data + tags + autotype + override_url
}

/// A digest over everything an archived version carries, matching what
/// `entry_content_eq` compares for a current one.
///
/// Cheap, order-stable, and only ever compared against another digest, never
/// stored or shown. It covered the field map alone, so two versions written
/// in the same second that differed only by a tag, an icon, an attachment or
/// an expiry date collapsed into one: the fork merged both, this said neither
/// side was ahead, and the version that existed on one machine was never sent
/// from it.
fn history_version_digest(version: &EntryRef<'_>) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let mut hasher = Sha256::new();
    let mut note = |bytes: &[u8]| {
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    };
    let mut fields: Vec<(&String, &Value<String>)> = version.fields.iter().collect();
    fields.sort_by_key(|(key, _)| *key);
    for (key, value) in fields {
        note(key.as_bytes());
        note(value.get().as_bytes());
        note(&[u8::from(value.is_protected())]);
    }
    let mut tags = version.tags.clone();
    tags.sort_unstable();
    for tag in &tags {
        note(tag.as_bytes());
    }
    let mut custom: Vec<(&String, &CustomDataItem)> = version.custom_data.iter().collect();
    custom.sort_by_key(|(key, _)| *key);
    for (key, item) in custom {
        note(key.as_bytes());
        note(format!("{:?}", item.value).as_bytes());
    }
    for attachment in attachment_fingerprint(version) {
        note(attachment.name.as_bytes());
        note(&[u8::from(attachment.protected)]);
        note(&attachment.data);
    }
    // Through the same comparison a current version gets: the fork omits the
    // element for the default icon while KeePassXC writes it out, and a merge
    // retargets an archived version's custom icon to whichever id the picture
    // already has here. Hashing the raw reference reported both sides ahead
    // after an ordinary round trip.
    match IconShown::of(
        version.icon(),
        version.custom_icon().as_deref(),
        DEFAULT_ENTRY_ICON,
    ) {
        IconShown::Picture(picture) => {
            note(b"picture");
            note(&picture);
        }
        other => note(format!("{other:?}").as_bytes()),
    }
    note(format!("{:?}", version.autotype).as_bytes());
    note(format!("{:?}", version.foreground_color).as_bytes());
    note(format!("{:?}", version.background_color).as_bytes());
    note(
        version
            .override_url
            .as_deref()
            .unwrap_or_default()
            .as_bytes(),
    );
    note(format!("{:?}", version.quality_check).as_bytes());
    // An expiry date that is not in force says nothing, the same way it says
    // nothing for a current entry.
    note(
        format!(
            "{:?}",
            version
                .times
                .expires
                .unwrap_or(false)
                .then_some(version.times.expiry)
                .flatten()
        )
        .as_bytes(),
    );
    hasher.finalize().into()
}

/// How far ahead of our clock a `last_modification` may sit and still count
/// as evidence. Covers ordinary drift between two machines; past it the
/// timestamp is a claim about the future, not a record of the past.
const MAX_CLOCK_SKEW: chrono::TimeDelta = chrono::TimeDelta::minutes(10);

/// Returns the strictly-newer side, or `None` when the timestamps are tied,
/// one is missing, or one sits implausibly far in the future (all ambiguous -
/// surface to the user). Equal-second timestamps are ambiguous because the
/// KeePass file format is second-precision and a true race on the same second
/// is the case where we *want* to prompt.
///
/// The future check matters because last-write-wins is decided entirely by a
/// number inside the shared file. Anyone who can write that file could set a
/// `LastModificationTime` of 2099 and have their version silently replace
/// everyone else's on the next sync, with nothing shown to the user. A
/// timestamp we cannot believe buys a prompt, not a win.
fn timestamp_winner(local: Option<NaiveDateTime>, remote: Option<NaiveDateTime>) -> Option<Side> {
    match (local, remote) {
        (Some(l), Some(r)) if is_future_dated(l) || is_future_dated(r) => None,
        (Some(l), Some(r)) if l > r => Some(Side::Local),
        (Some(l), Some(r)) if r > l => Some(Side::Remote),
        _ => None,
    }
}

/// True for a timestamp too far ahead of our clock to be a record of when
/// something was written.
fn is_future_dated(at: NaiveDateTime) -> bool {
    at > keepass::db::Times::now() + MAX_CLOCK_SKEW
}

/// The modification time a resolved divergence gets: later than every
/// timestamp we believe, and never later than that.
///
/// Taking the plain maximum handed the result whatever the other side
/// claimed. Keeping Local against a remote entry dated 2099 stamped the local
/// copy with 2099, so the attacker won every later comparison anyway and the
/// user's next genuine edit looked older than the thing it replaced. A
/// timestamp we already refused as evidence must not become our own.
fn resolution_time(candidates: [Option<NaiveDateTime>; 2]) -> NaiveDateTime {
    let now = Times::now();
    let believable_max = candidates
        .into_iter()
        .flatten()
        .filter(|candidate| !is_future_dated(*candidate))
        .chain(std::iter::once(now))
        .max()
        .expect("now is always a candidate");
    // Strictly later, not merely equal. A resolution that ties with the
    // version it replaced settles nothing: a third copy still holding the
    // other side at that same timestamp meets it as a fresh tie and asks
    // again. One second is the smallest step KDBX can represent.
    if believable_max >= now {
        believable_max + chrono::TimeDelta::seconds(1)
    } else {
        now
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
    picks: &Resolutions,
    report: &ConflictReport,
) -> Result<Database, ApplyError> {
    preflight_fidelity(local, remote)?;

    let mut merged = local.clone();
    let mut source = remote.clone();

    // The outer configuration is deliberately not merged, and this is the
    // least-bad of three bad options.
    //
    // It travels with the file rather than with any object, and KDBX gives it
    // no change time, so there is no evidence for whose version is newer.
    // Adopting the remote's was tried and is worse: a sync would then
    // silently re-encrypt the vault with whatever parameters the other file
    // carries, which can be weaker than the ones the user chose here.
    // Keeping ours means a cipher, KDF or compression change made elsewhere
    // is reverted by our next upload, which is visible in that client and
    // costs nobody an entry.

    // Only genuinely ambiguous entries appear here. Timestamp-resolved rows
    // and one-sided additions are handled natively by Database::merge.
    for conflict in &report.conflicts {
        let side = picks
            .entries
            .get(&conflict.id)
            .copied()
            .unwrap_or(Side::Local);
        force_manual_winner(&mut merged, &mut source, &conflict.id, side)?;
    }
    // Groups the same way. Without this the fork ranks them by timestamp,
    // which is exactly what a tie or a future date defeats, and the losing
    // side's name, notes, tags or settings are gone on the next upload.
    for conflict in &report.group_conflicts {
        let side = picks
            .groups
            .get(&conflict.id)
            .copied()
            .unwrap_or(Side::Local);
        force_group_winner(&mut merged, &mut source, &conflict.id, side)?;
    }
    // The database's own settings, same rule and same default.
    if report.metadata_conflict.is_some() {
        force_metadata_winner(
            &mut merged,
            &mut source,
            picks.metadata.unwrap_or(Side::Local),
        );
    }
    for resolved in &report.auto_resolved {
        preserve_auto_resolved_history(&mut merged, &mut source, resolved)?;
    }
    reconcile_unsurfaced_metadata(&mut merged, &mut source)?;

    let log = merged
        .merge(&source)
        .map_err(|error| ApplyError::DatabaseMerge(error.to_string()))?;
    // The fork warns both for outcomes it already resolved by documented
    // policy and for genuinely lossy ones. Only the lossy ones may abort:
    // a policy-resolved warning's trigger lives in the remote file (e.g.
    // another client wrote entries without LocationChanged timestamps), so
    // treating it as fatal re-fails every retry identically and wedges sync
    // permanently with no user remedy.
    let lossy: Vec<String> = log
        .warnings
        .iter()
        .filter(|warning| !warning_is_harmless(warning, local, remote))
        .map(MergeWarning::to_string)
        .collect();
    if !lossy.is_empty() {
        return Err(ApplyError::DatabaseMergeWarnings(lossy.join("; ")));
    }

    reunite_recycle_bins(&mut merged, [local, remote]);

    // The merged database is saved directly (no document mutation runs in
    // between), and `merge_history` unions both sides' histories - trim here
    // or repeated conflicts grow entries past the vault's HistoryMaxItems.
    crate::keepass::document::enforce_history_limits(&mut merged);

    Ok(merged)
}

/// Warnings the fork emits for situations it already resolved without
/// dropping data: same-second diverged history versions (both are kept),
/// missing history-entry timestamps (deterministic substitutes inside a list
/// that is unioned anyway), missing history at all (an empty default), and
/// the root group, which cannot move anywhere.
///
/// Everything else stays fatal, because each remaining variant silently
/// discards a remote change: a dropped entry, a dropped move, a move whose
/// direction could not be decided and so keeps the local location for the
/// next upload to impose, or a missing timestamp on a *current* object, where
/// the substitute decides a comparison the file did not.
///
/// Matched on the fork's `MergeWarning` enum. This used to read the message
/// text with `starts_with`, `contains` and word positions, which would have
/// flipped a fatal warning to harmless, silently, the first time the fork
/// reworded a sentence.
fn warning_is_policy_resolved(warning: &MergeWarning) -> bool {
    matches!(
        warning,
        MergeWarning::DivergedHistory { .. }
            | MergeWarning::MissingHistoryTimestamp { .. }
            | MergeWarning::NoHistory { .. }
            | MergeWarning::CannotMoveRootGroup { .. }
    )
}

/// Extends `warning_is_policy_resolved` with a divergence check for
/// missing-timestamp warnings on *current* entries/groups: the fork emits
/// those before ever comparing the object, so a legacy entry that is
/// byte-identical on both sides would otherwise make every merge attempt
/// fatal forever - while a genuinely divergent one must stay fatal (the
/// epoch substitute would silently pick a winner). The comparison mirrors
/// the pinned fork's own divergence checks: timestamps and history never
/// define current-entry content, while group membership and parent location
/// are merged independently. A parent difference is accepted only when the
/// location timestamps identify one strictly newer side; missing location
/// timestamps also produce a separate fatal move warning.
fn warning_is_harmless(warning: &MergeWarning, local: &Database, remote: &Database) -> bool {
    if warning_is_policy_resolved(warning) {
        return true;
    }
    match warning {
        MergeWarning::MissingEntryTimestamp { entry, .. } => {
            entries_equivalent_for_timestamp_warning(local, remote, &entry.to_string())
        }
        MergeWarning::MissingGroupTimestamp { group, .. } => {
            groups_equivalent_for_timestamp_warning(local, remote, &group.to_string())
        }
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

/// KeePass2/KeePassXC always write an explicit `<IconID>` (0 "Key" for
/// entries, 48 "Folder" for groups), while our pinned keepass fork omits the
/// element when the icon is unset. Both spellings mean "default icon", so
/// comparing them strictly would flag every entry of a foreign-written vault
/// as diverged. Normalise the explicit default to `None` before comparing.
const DEFAULT_ENTRY_ICON: usize = 0;
const DEFAULT_GROUP_ICON: usize = 48;

/// What an icon reference shows, which is what two copies of one object
/// are compared on.
///
/// The id behind a custom icon is bookkeeping. The merge lets a reference
/// share a picture the other copy already holds under its own id, so one
/// round trip leaves both copies showing the same favicon under two ids,
/// with the object's clock untouched. Compared by id, that asked about
/// every entry with a favicon, and either answer changed nothing on
/// screen. Two spellings of the default icon are one icon the same way:
/// KeePass writes the index out, the fork omits it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum IconShown<'a> {
    Default,
    BuiltIn(usize),
    Picture(Cow<'a, [u8]>),
    /// A custom icon whose picture this file does not hold, so the id is
    /// all there is to compare.
    Unresolved(CustomIconId),
}

impl<'a> IconShown<'a> {
    fn of(icon: Option<&Icon>, custom: Option<&'a CustomIcon>, default_index: usize) -> Self {
        match icon {
            None => Self::Default,
            Some(Icon::BuiltIn(index)) if *index == default_index => Self::Default,
            Some(Icon::BuiltIn(index)) => Self::BuiltIn(*index),
            Some(Icon::Custom(id)) => custom.map_or(Self::Unresolved(*id), |icon| {
                Self::Picture(Cow::Borrowed(icon.data.as_slice()))
            }),
        }
    }

    fn into_owned(self) -> IconShown<'static> {
        match self {
            Self::Picture(picture) => IconShown::Picture(Cow::Owned(picture.into_owned())),
            Self::Default => IconShown::Default,
            Self::BuiltIn(index) => IconShown::BuiltIn(index),
            Self::Unresolved(id) => IconShown::Unresolved(id),
        }
    }
}

fn entry_icons_equivalent(local: &EntryRef<'_>, remote: &EntryRef<'_>) -> bool {
    let (ours, theirs) = (local.custom_icon(), remote.custom_icon());
    IconShown::of(local.icon(), ours.as_deref(), DEFAULT_ENTRY_ICON)
        == IconShown::of(remote.icon(), theirs.as_deref(), DEFAULT_ENTRY_ICON)
}

fn group_icons_equivalent(local: &GroupRef<'_>, remote: &GroupRef<'_>) -> bool {
    let (ours, theirs) = (local.custom_icon(), remote.custom_icon());
    IconShown::of(local.icon(), ours.as_deref(), DEFAULT_GROUP_ICON)
        == IconShown::of(remote.icon(), theirs.as_deref(), DEFAULT_GROUP_ICON)
}

/// Some KeePass clients serialize an absent `PreviousParentGroup` as the nil
/// UUID while others omit the element. Both mean that the item has no former
/// parent.
///
/// Used only for the *group* structural-writeback check. Entry-level
/// `previous_parent_group` is excluded from conflict detection entirely:
/// it is invisible restore-location metadata that KeePass2's own merge
/// resolves silently, and vaults saved by FerrisPass before the fork
/// round-tripped the field (pre-0.7) have it stripped on every entry -
/// comparing it would re-conflict a whole foreign-written vault forever.
fn previous_groups_equivalent(local: Option<GroupId>, remote: Option<GroupId>) -> bool {
    group_uuids_equivalent(local.map(|id| id.uuid()), remote.map(|id| id.uuid()))
}

fn group_uuids_equivalent(local: Option<uuid::Uuid>, remote: Option<uuid::Uuid>) -> bool {
    normalised_group_uuid(local) == normalised_group_uuid(remote)
}

/// Some clients serialize "no such group" as the nil UUID, others omit the
/// element. Both mean the same thing, in `PreviousParentGroup` and in the
/// metadata pointers alike.
fn normalised_group_uuid(id: Option<uuid::Uuid>) -> Option<uuid::Uuid> {
    id.filter(|id| !id.is_nil())
}

fn entry_content_eq(local: &EntryRef<'_>, remote: &EntryRef<'_>) -> bool {
    entry_location_is_resolved(local, remote)
        && local.fields == remote.fields
        && local.autotype == remote.autotype
        && local.tags == remote.tags
        && local.custom_data == remote.custom_data
        && entry_icons_equivalent(local, remote)
        && local.foreground_color == remote.foreground_color
        && local.background_color == remote.background_color
        && local.override_url == remote.override_url
        && local.quality_check == remote.quality_check
        && attachment_fingerprint(local) == attachment_fingerprint(remote)
}

fn entry_location_is_resolved(local: &EntryRef<'_>, remote: &EntryRef<'_>) -> bool {
    local.parent().id() == remote.parent().id()
        || matches!(
            (local.times.location_changed, remote.times.location_changed),
            (Some(local_changed), Some(remote_changed)) if local_changed != remote_changed
        )
}

fn attachment_fingerprint(entry: &EntryRef<'_>) -> Vec<AttachmentFingerprint> {
    let mut attachments: Vec<_> = entry
        .named_attachments()
        .map(|(name, attachment)| AttachmentFingerprint {
            name: name.to_string(),
            protected: attachment.data.is_protected(),
            data: attachment.data.get().clone(),
        })
        .collect();
    attachments.sort_by(|left, right| left.name.cmp(&right.name));
    attachments
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
        && group_icons_equivalent(local, remote)
        && local.custom_data == remote.custom_data
        && local.is_expanded == remote.is_expanded
        && local.default_autotype_sequence == remote.default_autotype_sequence
        && local.enable_autotype == remote.enable_autotype
        && local.enable_searching == remote.enable_searching
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

    Ok(())
}

/// The pinned fork's `Database::merge` fails closed when two entries or
/// groups share a last-modification timestamp but differ on *any* field.
/// Divergences FerrisPass deliberately does not surface as conflicts -
/// default-icon spelling and previous-parent restore metadata - would
/// therefore wedge the merge with "have the same modification time but
/// have diverged". Align them on both sides before merging. Direction:
/// a real previous-parent UUID wins over an absent one (healing vaults
/// whose pre-0.7 saves stripped the field), otherwise the local (merged)
/// representation wins. Pairs whose timestamps differ are left alone -
/// the fork resolves those wholesale by the newer side.
/// Groups whose user-authored content diverged and whose timestamps cannot
/// rank them: tied, missing, or implausibly far in the future.
///
/// The future case matters as much as the tie. Group content has no
/// last-write-wins path in this module at all; the fork merges groups by
/// timestamp, so anyone who can write the shared file could stamp a group
/// rename with the year 2099 and have it replace everyone else's, with
/// nothing shown to anyone. Ranked timestamps are left to the fork.
fn group_conflicts(local: &Database, remote: &Database) -> Vec<GroupConflict> {
    let mut conflicts: Vec<GroupConflict> = local
        .iter_all_groups()
        .filter_map(|local_group| {
            let remote_group = remote.group(local_group.id())?;
            // Two independent reasons, each with its own clock. Content is
            // ranked by `last_modification`, where the group sits by
            // `location_changed`, and either can tie on its own.
            let content_tied = group_content_differs(&local_group, &remote_group)
                && timestamp_winner(
                    local_group.times.last_modification,
                    remote_group.times.last_modification,
                )
                .is_none();
            let moved = local_group.parent().map(|parent| parent.id())
                != remote_group.parent().map(|parent| parent.id());
            let location_tied = moved
                && timestamp_winner(
                    local_group.times.location_changed,
                    remote_group.times.location_changed,
                )
                .is_none();
            if !content_tied && !location_tied {
                return None;
            }
            let mut fields = group_field_diffs(&local_group, &remote_group);
            if moved {
                let ours = local_group.parent().map(|parent| parent.id());
                let theirs = remote_group.parent().map(|parent| parent.id());
                let (here, there) = match (ours, theirs) {
                    (Some(ours), Some(theirs)) => location_pair(local, remote, ours, theirs),
                    _ => (
                        group_location(local, local_group.id()),
                        group_location(remote, remote_group.id()),
                    ),
                };
                fields.insert(
                    0,
                    FieldDiff {
                        sides_read_alike: false,
                        differs: true,
                        label: "Location".into(),
                        local: here,
                        remote: there,
                    },
                );
            }
            Some(GroupConflict {
                id: local_group.id().to_string(),
                name: local_group.name.clone(),
                path: group_ancestry(local, local_group.id()),
                fields,
            })
        })
        .collect();
    conflicts.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    conflicts
}

/// Two names that may be the same word for two different objects. The id is
/// added only when the names alone would not tell them apart.
fn named_pair(
    here: String,
    there: String,
    ours: Option<uuid::Uuid>,
    theirs: Option<uuid::Uuid>,
) -> (String, String) {
    if here != there {
        return (here, there);
    }
    let tagged = |name: String, id: Option<uuid::Uuid>| match id {
        Some(id) => format!("{name} ({id})"),
        None => name,
    };
    (tagged(here, ours), tagged(there, theirs))
}

/// The two places a row is asking about, told apart.
///
/// Each side is read from its own database, because a group can exist in only
/// one of them. Two groups can then share a name, on one side or across the
/// two, and the row would show the same text twice and say nothing about the
/// choice it was asking for. The id is added only when it is needed.
fn location_pair(
    local: &Database,
    remote: &Database,
    ours: GroupId,
    theirs: GroupId,
) -> (String, String) {
    let here = group_location_of(local, ours);
    let there = group_location_of(remote, theirs);
    if here != there {
        return (here, there);
    }
    // The whole uuid: a prefix is short enough that two groups could be made
    // to share one, and this row exists to tell two places apart.
    let tagged = |location: String, id: GroupId| format!("{location} ({})", id.uuid());
    (tagged(here, ours), tagged(there, theirs))
}

/// Where an object sits when that object is inside `id`, as a person reads
/// it: the group's own path, then the group itself.
fn group_location_of(db: &Database, id: GroupId) -> String {
    let Some(group) = db.group(id) else {
        return "Vault root".to_string();
    };
    if group.parent().is_none() {
        return "Vault root".to_string();
    }
    // A group can carry no name, and two of them then read the same. The row
    // exists so a person can tell one place from another, so an id stands in.
    let name = if group.name.is_empty() {
        format!("Unnamed group ({})", id.uuid())
    } else {
        group.name.clone()
    };
    match group_location(db, id) {
        path if path == "Vault root" => name,
        path => format!("{path} > {name}"),
    }
}

/// Where a group sits, as a person would read it. The root group carries no
/// name of its own, so it reads as the vault rather than as an empty step.
fn group_location(db: &Database, id: GroupId) -> String {
    let path: Vec<String> = group_ancestry(db, id)
        .into_iter()
        .filter(|name| !name.is_empty())
        .collect();
    if path.is_empty() {
        "Vault root".to_string()
    } else {
        path.join(" > ")
    }
}

/// Ancestor names from the root down, excluding the group itself. Two groups
/// can share a name, and a conflict row that only says "Banking" does not
/// tell the user which one they are deciding about.
fn group_ancestry(db: &Database, id: GroupId) -> Vec<String> {
    let mut path = Vec::new();
    let mut next = db
        .group(id)
        .and_then(|group| group.parent().map(|p| p.id()));
    while let Some(parent_id) = next {
        let Some(parent) = db.group(parent_id) else {
            break;
        };
        path.push(parent.name.clone());
        next = parent.parent().map(|p| p.id());
    }
    path.reverse();
    path
}

/// The rows the conflict screen shows for a group. Same shape as the entry
/// rows so the overlay renders both through one path.
fn group_field_diffs(local: &GroupRef<'_>, remote: &GroupRef<'_>) -> Vec<FieldDiff> {
    // An unset value and an empty one are two different things, and quoting
    // is what keeps them, and a value that contains the separators, apart.
    fn optional(value: &Option<String>) -> String {
        value.clone().unwrap_or_else(|| "(not set)".to_string())
    }
    /// An Auto-Type sequence is free text and can hold a literal the user
    /// typed in, which on this screen is a credential in front of whoever is
    /// looking at it. The placeholders stay: they are the part that says what
    /// the sequence does, and they hold nothing.
    fn sequence(value: &Option<String>) -> String {
        value.as_deref().map_or_else(
            || "(not set)".to_string(),
            crate::autotype::sequence::redact_sequence_literals,
        )
    }
    fn tri_state(value: &Option<bool>) -> String {
        match value {
            Some(true) => "Yes".into(),
            Some(false) => "No".into(),
            None => "Inherited".into(),
        }
    }
    #[allow(clippy::ptr_arg)] // the closure is handed a &Vec by `diff_row`
    fn tags(tags: &Vec<String>) -> String {
        tags.iter()
            .map(|tag| format!("{tag:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
    fn icon_row(group: &GroupRef<'_>) -> String {
        match group.icon() {
            Some(Icon::BuiltIn(index)) => format!("Built-in icon {index}"),
            // Naming the image matters: two different ones both read as
            // "Custom image", so the row compared equal and never appeared.
            // Two pictures can also share a name, or have none, and the row
            // then falls back to naming the sides.
            Some(Icon::Custom(_)) => group.custom_icon().map_or_else(
                || "Custom image".to_string(),
                |icon| match icon.name.as_deref() {
                    Some(name) => format!("Custom image \"{name}\""),
                    None => "Custom image".to_string(),
                },
            ),
            None => "Default icon".into(),
        }
    }

    vec![
        diff_row("Name", &local.name, &remote.name, Clone::clone),
        // Compared on the picture, as everywhere: one id can mean two
        // pictures until the merge separates them, and two ids one picture
        // once it has. The name is this row's alone.
        {
            let (ours, theirs) = (local.custom_icon(), remote.custom_icon());
            FieldDiff {
                sides_read_alike: false,
                differs: !group_icons_equivalent(local, remote)
                    || ours.as_ref().map(|icon| &icon.name)
                        != theirs.as_ref().map(|icon| &icon.name),
                label: "Icon".into(),
                local: String::new(),
                remote: String::new(),
            }
            .with_values(icon_row(local), icon_row(remote))
        },
        diff_row(
            "Expires",
            &in_force_expiry(&local.times),
            &in_force_expiry(&remote.times),
            |at| expiry_row(*at),
        ),
        diff_row("Notes", &local.notes, &remote.notes, optional),
        diff_row("Tags", &local.tags, &remote.tags, tags),
        diff_row(
            "Auto-Type sequence",
            &local.default_autotype_sequence,
            &remote.default_autotype_sequence,
            sequence,
        ),
        diff_row(
            "Auto-Type enabled",
            &local.enable_autotype,
            &remote.enable_autotype,
            tri_state,
        ),
        diff_row(
            "Searchable",
            &local.enable_searching,
            &remote.enable_searching,
            tri_state,
        ),
        // Plugin data counts as content, so a difference here can be the only
        // reason a group is asked about. Without a row the screen showed an
        // empty list and the answer discarded the other side's keys for good:
        // a group keeps no history to recover them from.
        diff_row(
            "Plugin data",
            &ordered_custom_data(&local.custom_data),
            &ordered_custom_data(&remote.custom_data),
            |items| custom_data_summary(items),
        ),
    ]
    .into_iter()
    .filter(|diff| diff.differs)
    .collect()
}

fn group_with_uuid(database: &Database, id: uuid::Uuid) -> Option<GroupId> {
    database
        .iter_all_groups()
        .map(|group| group.id())
        .find(|group_id| group_id.uuid() == id)
}

/// An unset value and an empty one are two different things, and a row that
/// renders both as nothing stops saying which one is being chosen.
fn unset_or(value: Option<&String>) -> String {
    value.cloned().unwrap_or_else(|| "(not set)".to_string())
}

/// An expiry date only counts while it is in force, here as everywhere.
fn in_force_expiry(times: &Times) -> Option<NaiveDateTime> {
    times
        .expires
        .unwrap_or(false)
        .then_some(times.expiry)
        .flatten()
}

/// To the second: that is what KDBX stores and what the comparison uses, and
/// two dates inside one minute read alike without it.
fn expiry_row(at: Option<NaiveDateTime>) -> String {
    at.map_or_else(
        || "Never".to_string(),
        |at| at.format("%Y-%m-%d %H:%M:%S").to_string(),
    )
}

/// Plugin data in a fixed order, so its `Debug` is stable and two sides that
/// hold the same keys read the same.
fn ordered_custom_data(
    items: &std::collections::HashMap<String, CustomDataItem>,
) -> BTreeMap<&String, &CustomDataItem> {
    items.iter().collect()
}

/// One conflict row, decided on the values and rendered for a person.
///
/// Two rules, and enforcing them in one place is why this exists at all.
/// Whether the row differs is decided on the values, never on the text: a
/// rendering that maps two different values onto one string would otherwise
/// drop the row, and the answer would then discard one of them in silence.
/// And when they do differ, the two cells must not read alike, or the screen
/// asks a question without showing it. Where the rendering cannot tell them
/// apart, [`FieldDiff::with_sides_told_apart`] names the sides.
fn diff_row<T: PartialEq>(
    label: impl Into<Cow<'static, str>>,
    local: &T,
    remote: &T,
    render: impl Fn(&T) -> String,
) -> FieldDiff {
    FieldDiff {
        sides_read_alike: false,
        differs: local != remote,
        label: label.into(),
        local: render(local),
        remote: render(remote),
    }
    .with_sides_told_apart()
}

impl FieldDiff {
    /// Keep the decision, replace the rendering. For a row whose two sides
    /// are compared on one value but shown as two different things.
    fn with_values(self, local: String, remote: String) -> Self {
        Self {
            local,
            remote,
            ..self
        }
        .with_sides_told_apart()
    }

    /// The last resort when a rendering cannot show what the difference is.
    ///
    /// Deriving the marker from the value was the obvious idea and the wrong
    /// one: a digest of a password or an attachment, shown on screen, is an
    /// offline oracle against exactly the content the row is refusing to
    /// display. The marker says nothing about the value. The row is already
    /// flagged as differing, so naming the sides is enough to stop it asking
    /// a question it has not shown.
    fn with_sides_told_apart(self) -> Self {
        if !self.differs || self.local != self.remote {
            return self;
        }
        Self {
            local: format!("{} ({})", self.local, Side::Local.own_words()),
            remote: format!("{} ({})", self.remote, Side::Remote.own_words()),
            sides_read_alike: true,
            ..self
        }
    }
}

/// Plugin data as one line, sorted so two sides that hold the same keys read
/// the same. Values are described rather than dumped: they are opaque to this
/// application, and the row exists to tell two sets apart.
fn custom_data_summary(items: &BTreeMap<&String, &CustomDataItem>) -> String {
    // Quoted, because a key or a value may contain the separators. Without
    // that, one key called `a = 1, b` reads exactly like two keys `a` and
    // `b`, and the row stops telling the two sides apart.
    items
        .iter()
        .map(|(key, item)| match &item.value {
            None => format!("{key:?} (not set)"),
            // Redacted, not shown: see `custom_data_value`. Two values of one
            // size then read alike, and the row names its sides instead.
            Some(CustomDataValue::String(value)) => format!("{key:?} = {}", redact(value)),
            Some(CustomDataValue::Binary(bytes)) => {
                format!("{key:?} = ••• ({} bytes)", bytes.len())
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Content the user authored, as opposed to view state a client may rewrite
/// without bumping the modification time (`IsExpanded`, icon spelling,
/// previous-parent normalisation). The tie-break resolves both, but only a
/// content tie means a real edit is being set aside, so `diff` reports those
/// separately through the same predicate.
fn group_content_differs(a: &GroupRef<'_>, b: &GroupRef<'_>) -> bool {
    a.name != b.name
        || a.notes != b.notes
        || a.custom_data != b.custom_data
        || a.tags != b.tags
        || a.default_autotype_sequence != b.default_autotype_sequence
        || a.enable_autotype != b.enable_autotype
        || a.enable_searching != b.enable_searching
        // Both are things a person picked, and both were missing here, so a
        // tie on either was resolved in this copy's favour and uploaded over
        // the other one. Each is compared through the same normalisation the
        // rest of the module uses: two spellings of the default icon are one
        // icon, and a date that is not in force is not a date.
        || !group_icons_equivalent(a, b)
        || !expiry_equivalent(&a.times, &b.times)
}

/// An entry and a group carry an icon the same way. Addressed by id rather
/// than by handle, so one alignment can write to both databases in turn.
trait HasIcon: Copy {
    fn write_icon(self, db: &mut Database, icon: Option<&Icon>) -> Result<(), ApplyError>;
}

impl HasIcon for EntryId {
    fn write_icon(self, db: &mut Database, icon: Option<&Icon>) -> Result<(), ApplyError> {
        let Some(mut entry) = db.entry_mut(self) else {
            return Ok(());
        };
        match icon {
            Some(Icon::BuiltIn(index)) => entry.set_icon_builtin(*index),
            Some(Icon::Custom(id)) => entry.set_icon_custom(*id).map_err(|_| unrecoverable(*id))?,
            None => entry.set_icon_none(),
        }
        Ok(())
    }
}

impl HasIcon for GroupId {
    fn write_icon(self, db: &mut Database, icon: Option<&Icon>) -> Result<(), ApplyError> {
        let Some(mut group) = db.group_mut(self) else {
            return Ok(());
        };
        match icon {
            Some(Icon::BuiltIn(index)) => group.set_icon_builtin(*index),
            Some(Icon::Custom(id)) => group.set_icon_custom(*id).map_err(|_| unrecoverable(*id))?,
            None => group.set_icon_none(),
        }
        Ok(())
    }
}

fn unrecoverable(icon: CustomIconId) -> ApplyError {
    ApplyError::CustomIconUnrecoverable {
        id: icon.to_string(),
    }
}

/// Give both copies of `id` one reference for a picture they already show,
/// so the fork's tie check sees one icon.
///
/// The source copy takes the merged copy's reference. The id comes back
/// changed when the source keeps a different picture under it: the picture
/// then went in under a fresh id, and the merged copy follows, so both hold
/// it under an id neither used before.
fn share_icon(
    merged: &mut Database,
    source: &mut Database,
    id: impl HasIcon,
    shown: Option<&Icon>,
) -> Result<(), ApplyError> {
    let shared = reference_to_share(source, merged, shown)?;
    id.write_icon(source, shared.as_ref())?;
    if shared.as_ref() != shown {
        let followed = reference_to_share(merged, source, shared.as_ref())?;
        id.write_icon(merged, followed.as_ref())?;
    }
    Ok(())
}

/// The reference `target` can write for what `from` shows as `shown`. A
/// custom icon's picture has to be in `target`'s table first: `set_icon_custom`
/// clears the current icon and only then rejects an id it does not hold.
fn reference_to_share(
    target: &mut Database,
    from: &Database,
    shown: Option<&Icon>,
) -> Result<Option<Icon>, ApplyError> {
    match shown {
        Some(Icon::Custom(icon)) => target
            .adopt_custom_icon_from(from, *icon)
            .map(|adopted| Some(Icon::Custom(adopted)))
            .ok_or_else(|| unrecoverable(*icon)),
        other => Ok(other.cloned()),
    }
}

fn reconcile_unsurfaced_metadata(
    merged: &mut Database,
    source: &mut Database,
) -> Result<(), ApplyError> {
    fn canonical_previous_parent(
        merged_value: Option<GroupId>,
        source_value: Option<GroupId>,
    ) -> Option<GroupId> {
        merged_value
            .filter(|id| !id.uuid().is_nil())
            .or(source_value.filter(|id| !id.uuid().is_nil()))
    }

    let entry_ids: Vec<EntryId> = merged.iter_all_entries().map(|entry| entry.id()).collect();
    for id in entry_ids {
        let Some(merged_entry) = merged.entry(id) else {
            continue;
        };
        let Some(source_entry) = source.entry(id) else {
            continue;
        };
        if merged_entry.times.last_modification != source_entry.times.last_modification {
            continue;
        }
        let merged_icon = merged_entry.icon().cloned();
        let references_differ = merged_entry.icon() != source_entry.icon();
        let icons_agree = entry_icons_equivalent(&merged_entry, &source_entry);
        let merged_prev = merged_entry.previous_parent_group;
        let source_prev = source_entry.previous_parent_group;

        let canonical_prev = canonical_previous_parent(merged_prev, source_prev);
        if merged_prev != canonical_prev
            && let Some(mut entry) = merged.entry_mut(id)
        {
            entry.previous_parent_group = canonical_prev;
        }
        if source_prev != canonical_prev
            && let Some(mut entry) = source.entry_mut(id)
        {
            entry.previous_parent_group = canonical_prev;
        }

        if references_differ && icons_agree {
            share_icon(merged, source, id, merged_icon.as_ref())?;
        }
    }

    let group_ids: Vec<GroupId> = merged.iter_all_groups().map(|group| group.id()).collect();
    for id in group_ids {
        let Some(merged_group) = merged.group(id) else {
            continue;
        };
        let Some(source_group) = source.group(id) else {
            continue;
        };
        if merged_group.times.last_modification != source_group.times.last_modification {
            continue;
        }
        let merged_icon = merged_group.icon().cloned();
        let references_differ = merged_group.icon() != source_group.icon();
        let icons_agree = group_icons_equivalent(&merged_group, &source_group);
        let merged_prev = merged_group.previous_parent_group;
        let source_prev = source_group.previous_parent_group;

        let canonical_prev = canonical_previous_parent(merged_prev, source_prev);
        if merged_prev != canonical_prev
            && let Some(mut group) = merged.group_mut(id)
        {
            group.previous_parent_group = canonical_prev;
        }
        if source_prev != canonical_prev
            && let Some(mut group) = source.group_mut(id)
        {
            group.previous_parent_group = canonical_prev;
        }

        if references_differ && icons_agree {
            share_icon(merged, source, id, merged_icon.as_ref())?;
        }

        // Nothing is stamped as modified here any more. A tied group reaching
        // this point can only differ on bookkeeping: its icon reference was
        // just aligned above, its previous parent normalised, and the fork
        // ignores `IsExpanded` and `LastTopVisibleEntry` itself. A tie on the
        // name, notes, tags or settings never gets here, because a group
        // conflict is settled in `apply_picks` and that demotes the losing
        // side's clock.
        //
        // Buying past the fork's check with a fresh modification time cost
        // more than it bought: the stamp outlived the merge, and the next
        // sync read a collapsed twisty as an edit newer than somebody's
        // rename.
    }
    Ok(())
}

/// Settle one group conflict by copying the chosen content onto the merged
/// database and demoting the other side, so the fork's timestamp-ranked merge
/// cannot overrule the user's choice.
///
/// There is no history to preserve the loser in: KDBX archives entry versions,
/// not group versions. The losing content is genuinely discarded, which is
/// exactly why this is asked rather than decided.
fn force_group_winner(
    merged: &mut Database,
    source: &mut Database,
    raw_id: &str,
    winner: Side,
) -> Result<(), ApplyError> {
    let group_id = find_group_id(merged, raw_id).ok_or_else(|| ApplyError::GroupMissing {
        id: raw_id.to_string(),
        side: Side::Local,
    })?;
    let source_id = find_group_id(source, raw_id).ok_or_else(|| ApplyError::GroupMissing {
        id: raw_id.to_string(),
        side: Side::Remote,
    })?;

    if matches!(winner, Side::Remote) {
        let mut content = {
            let Some(chosen) = source.group(source_id) else {
                return Err(ApplyError::GroupMissing {
                    id: raw_id.to_string(),
                    side: Side::Remote,
                });
            };
            GroupContent::of(&chosen)
        };
        // The image has to be here before the reference to it can be written:
        // `set_icon_custom` clears the object's current icon and only then
        // rejects an unknown id, so writing the reference first left the
        // group with no icon at all. The fork's merge adopts images too, but
        // it runs after this. The id can come back changed, because both
        // files can use one id for two different pictures.
        if let Some(Icon::Custom(icon)) = content.icon {
            let Some(here) = merged.adopt_custom_icon_from(source, icon) else {
                return Err(ApplyError::CustomIconUnrecoverable {
                    id: icon.to_string(),
                });
            };
            content.icon = Some(Icon::Custom(here));
        }
        let Some(mut target) = merged.group_mut(group_id) else {
            return Err(ApplyError::GroupMissing {
                id: raw_id.to_string(),
                side: Side::Local,
            });
        };
        content.write_to(&mut target)?;
    }

    // Where the group sits is decided on its own clock, so the choice has to
    // reach that one too. Without this the fork ranked the move by
    // `location_changed`, which is exactly what a tie defeats, and the losing
    // side's placement went back on the next upload.
    //
    // The move itself is left to the fork rather than done here: the parent
    // the user chose can be a group that only the other copy has, and the
    // fork adds those before it looks at moves. Doing it here would have
    // skipped exactly that case, silently, after the user had answered.
    let their_parent = source
        .group(source_id)
        .and_then(|group| group.parent().map(|parent| parent.id()));
    let our_parent = merged
        .group(group_id)
        .and_then(|group| group.parent().map(|parent| parent.id()));
    let moved_time = resolution_time([
        merged
            .group(group_id)
            .and_then(|group| group.times.location_changed),
        source
            .group(source_id)
            .and_then(|group| group.times.location_changed),
    ]);
    let winner_time = resolution_time([
        merged
            .group(group_id)
            .and_then(|group| group.times.last_modification),
        source
            .group(source_id)
            .and_then(|group| group.times.last_modification),
    ]);
    // The two clocks point in opposite directions when the remote placement
    // wins: the content stays ours, because it was written into this copy
    // above, while the move has to be one the fork performs.
    let (our_move, their_move) = match winner {
        Side::Local => (moved_time, Times::epoch()),
        Side::Remote => (Times::epoch(), moved_time),
    };
    if let Some(mut group) = merged.group_mut(group_id) {
        group.times.last_modification = Some(winner_time);
        if our_parent != their_parent {
            group.times.location_changed = Some(our_move);
        }
    }
    if let Some(mut group) = source.group_mut(source_id) {
        group.times.last_modification = Some(Times::epoch());
        if our_parent != their_parent {
            group.times.location_changed = Some(their_move);
        }
    }
    Ok(())
}

/// Put a recycle bin that lost the metadata merge inside the one that won.
///
/// Two copies can each have made their own bin. The merge keeps both groups
/// and can only designate one, and everything the user deleted into the other
/// silently became live again: an ordinary group holding entries they had
/// thrown away, listed next to the ones they kept. Nesting the loser inside
/// the winner keeps both sides' deletions deleted, because everything below
/// the bin counts as deleted, and leaves the groups themselves intact so the
/// user can still restore from either.
fn reunite_recycle_bins(merged: &mut Database, sides: [&Database; 2]) {
    let Some(designated) = normalised_group_uuid(merged.meta.recyclebin_uuid) else {
        return;
    };
    let Some(bin) = group_with_uuid(merged, designated) else {
        return;
    };
    let displaced: BTreeSet<uuid::Uuid> = sides
        .into_iter()
        .filter_map(|side| normalised_group_uuid(side.meta.recyclebin_uuid))
        .filter(|id| *id != designated && !retired_on_purpose(sides, designated, *id))
        .collect();
    for id in displaced {
        let Some(group_id) = group_with_uuid(merged, id) else {
            continue;
        };
        nest_into_bin(merged, group_id, bin);
    }
}

/// Whether the side that decided the current bin also *has* this group.
///
/// It is the difference between two copies that each made their own bin and
/// one copy that moved on: a group the deciding side knows and deliberately
/// did not designate was retired there, perhaps as an ordinary archive full
/// of entries the user still wants. Nesting that under the bin would delete
/// all of them, which is the same loss in the other direction.
fn retired_on_purpose(
    sides: [&Database; 2],
    designated: uuid::Uuid,
    candidate: uuid::Uuid,
) -> bool {
    sides.into_iter().any(|side| {
        normalised_group_uuid(side.meta.recyclebin_uuid) == Some(designated)
            && group_with_uuid(side, candidate).is_some()
    })
}

/// Move `group` under `bin`, dated so the next merge keeps it there.
///
/// When the bin already sits inside the group the plain move is a cycle. That
/// happens once a previous merge nested them and a third copy then designates
/// the outer one: the answer is to lift the bin out first, not to give up.
/// Giving up left the outer group holding entries that stopped counting as
/// deleted the moment it was no longer the bin.
fn nest_into_bin(merged: &mut Database, group: GroupId, bin: GroupId) {
    if crate::keepass::document::group_is_within(merged, bin, group)
        && let Some(above) = parent_of(merged, group)
    {
        move_group(merged, bin, above);
    }
    move_group(merged, group, bin);
}

fn parent_of(merged: &Database, group: GroupId) -> Option<GroupId> {
    merged
        .group(group)
        .and_then(|group| group.parent().map(|parent| parent.id()))
}

/// `track_changes` dates the move, without which the next merge against a
/// third copy undoes it. A failure here is a cycle the caller already ruled
/// out, so nothing is left to repair.
fn move_group(merged: &mut Database, group: GroupId, target: GroupId) {
    let previous_parent = parent_of(merged, group);
    if let Some(mut group) = merged.group_mut(group)
        && group.track_changes().move_to(target).is_ok()
    {
        group.previous_parent_group = previous_parent;
    }
}

/// Apply the user's choice to the database settings no change time could
/// rank, and to those only.
///
/// Copying the whole metadata block instead would revert a field the other
/// side legitimately changed last, which is the very loss this screen exists
/// to prevent. Fields a clock can rank stay the merge's business.
fn force_metadata_winner(merged: &mut Database, source: &mut Database, winner: Side) {
    let mut tied: Vec<MetaField> = Vec::new();
    for divergence in meta_divergences(merged, source) {
        // Several rows share `Settings`, and the fields under it move as one.
        if divergence.winner.is_none() && !tied.contains(&divergence.field) {
            tied.push(divergence.field);
        }
    }
    for field in &tied {
        if matches!(winner, Side::Remote) {
            copy_meta_field(merged, source, field);
        }
        // The choice has to outrank both sides: for the merge that runs next,
        // and for the next merge against a third copy that still holds the
        // other value. Stamping the loser back to the epoch is what
        // `force_group_winner` does with the same problem.
        stamp_meta_field(merged, source, field);
    }
}

fn copy_meta_field(merged_db: &mut Database, source_db: &Database, field: &MetaField) {
    // A tied custom icon name belongs to the icon table, not to the metadata
    // block, and the borrows below are of the meta halves alone.
    if let MetaField::CustomIcon(id) = field {
        let name = custom_icon_with_id(source_db, id).and_then(|icon| icon.name.clone());
        if let Some(id) = custom_icon_id_from(merged_db, id)
            && let Some(mut icon) = merged_db.custom_icon_mut(id)
        {
            icon.name = name;
        }
        return;
    }
    let (merged, source) = (&mut merged_db.meta, &source_db.meta);
    match field {
        MetaField::Name => merged.database_name = source.database_name.clone(),
        MetaField::Description => {
            merged.database_description = source.database_description.clone();
        }
        MetaField::DefaultUsername => merged.default_username = source.default_username.clone(),
        MetaField::RecycleBin => {
            merged.recyclebin_enabled = source.recyclebin_enabled;
            merged.recyclebin_uuid = source.recyclebin_uuid;
        }
        MetaField::EntryTemplatesGroup => {
            merged.entry_templates_group = source.entry_templates_group;
        }
        MetaField::Settings => {
            merged.color = source.color.clone();
            merged.maintenance_history_days = source.maintenance_history_days;
            merged.memory_protection = source.memory_protection.clone();
            merged.history_max_items = source.history_max_items;
            merged.history_max_size = source.history_max_size;
            merged.master_key_change_rec = source.master_key_change_rec;
            merged.master_key_change_force = source.master_key_change_force;
        }
        // A tied custom data key exists on both sides by definition: one that
        // exists on only one side is ranked to that side, not tied.
        MetaField::CustomData(key) => {
            if let Some(item) = source.custom_data.get(key) {
                merged.custom_data.insert(key.clone(), item.clone());
            }
        }
        MetaField::CustomIcon(_) => unreachable!("handled above, before the meta borrows"),
    }
}

fn stamp_meta_field(merged: &mut Database, source: &mut Database, field: &MetaField) {
    let decided = resolution_time([meta_clock(merged, field), meta_clock(source, field)]);
    set_meta_clock(merged, field, decided);
    set_meta_clock(source, field, Times::epoch());
}

/// The change time KDBX keeps for one decision unit. `None` when the side
/// does not carry that custom data key or that icon.
fn meta_clock(database: &Database, field: &MetaField) -> Option<NaiveDateTime> {
    let meta = &database.meta;
    match field {
        MetaField::Name => meta.database_name_changed,
        MetaField::Description => meta.database_description_changed,
        MetaField::DefaultUsername => meta.default_username_changed,
        MetaField::RecycleBin => meta.recyclebin_changed,
        MetaField::EntryTemplatesGroup => meta.entry_templates_group_changed,
        MetaField::Settings => meta.settings_changed,
        MetaField::CustomData(key) => meta
            .custom_data
            .get(key)
            .and_then(|item| item.last_modification_time),
        MetaField::CustomIcon(id) => {
            custom_icon_with_id(database, id).and_then(|icon| icon.last_modification_time)
        }
    }
}

fn set_meta_clock(database: &mut Database, field: &MetaField, at: NaiveDateTime) {
    if let MetaField::CustomIcon(id) = field {
        if let Some(id) = custom_icon_id_from(database, id)
            && let Some(mut icon) = database.custom_icon_mut(id)
        {
            icon.last_modification_time = Some(at);
        }
        return;
    }
    let meta = &mut database.meta;
    match field {
        MetaField::Name => meta.database_name_changed = Some(at),
        MetaField::Description => meta.database_description_changed = Some(at),
        MetaField::DefaultUsername => meta.default_username_changed = Some(at),
        MetaField::RecycleBin => meta.recyclebin_changed = Some(at),
        MetaField::EntryTemplatesGroup => meta.entry_templates_group_changed = Some(at),
        MetaField::Settings => meta.settings_changed = Some(at),
        MetaField::CustomData(key) => {
            if let Some(item) = meta.custom_data.get_mut(key) {
                item.last_modification_time = Some(at);
            }
        }
        MetaField::CustomIcon(_) => unreachable!("handled above, before the meta borrow"),
    }
}

/// Custom icons are keyed by an opaque id the divergence carries as text.
fn custom_icon_id_from(database: &Database, id: &str) -> Option<CustomIconId> {
    database
        .iter_all_custom_icons()
        .map(|icon| icon.id())
        .find(|candidate| candidate.to_string() == id)
}

fn custom_icon_with_id<'a>(
    database: &'a Database,
    id: &str,
) -> Option<keepass::db::CustomIconRef<'a>> {
    custom_icon_id_from(database, id).and_then(|id| database.custom_icon(id))
}

/// The user-authored half of a group, the same set `group_content_differs`
/// compares. Kept as an owned snapshot so the read and the write can borrow
/// two different databases in turn.
struct GroupContent {
    name: String,
    notes: Option<String>,
    custom_data: std::collections::HashMap<String, CustomDataItem>,
    tags: Vec<String>,
    default_autotype_sequence: Option<String>,
    enable_autotype: Option<bool>,
    enable_searching: Option<bool>,
    icon: Option<Icon>,
    expiry: Option<NaiveDateTime>,
    expires: Option<bool>,
}

impl GroupContent {
    fn of(group: &GroupRef<'_>) -> Self {
        Self {
            name: group.name.clone(),
            notes: group.notes.clone(),
            custom_data: group.custom_data.clone(),
            tags: group.tags.clone(),
            default_autotype_sequence: group.default_autotype_sequence.clone(),
            enable_autotype: group.enable_autotype,
            enable_searching: group.enable_searching,
            icon: group.icon().cloned(),
            expiry: group.times.expiry,
            expires: group.times.expires,
        }
    }

    /// Fails only on a custom icon whose image is in neither database, which
    /// is a dangling reference in the file rather than a merge outcome.
    /// Dropping that error silently produced a group with no icon at all.
    fn write_to(self, group: &mut keepass::db::GroupMut<'_>) -> Result<(), ApplyError> {
        group.name = self.name;
        group.notes = self.notes;
        group.custom_data = self.custom_data;
        group.tags = self.tags;
        group.default_autotype_sequence = self.default_autotype_sequence;
        group.enable_autotype = self.enable_autotype;
        group.enable_searching = self.enable_searching;
        match self.icon {
            Some(Icon::BuiltIn(index)) => group.set_icon_builtin(index),
            // By id, so the reference stays the one both files agree on. The
            // caller has already brought the image across.
            Some(Icon::Custom(id)) => group
                .set_icon_custom(id)
                .map_err(|_| ApplyError::CustomIconUnrecoverable { id: id.to_string() })?,
            None => group.set_icon_none(),
        }
        group.times.expiry = self.expiry;
        group.times.expires = self.expires;
        Ok(())
    }
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
    let winner_time = resolution_time([
        local_entry.times.last_modification,
        remote_entry.times.last_modification,
    ]);
    // Where the entry sits is decided on its own clock, so the choice has to
    // reach that one too, in opposite directions: the fork performs the move
    // itself, and the target group can be one only the other copy has.
    let moved = local_entry.times.location_changed != remote_entry.times.location_changed
        || local.entry(entry_id).map(|entry| entry.parent().id())
            != remote.entry(entry_id).map(|entry| entry.parent().id());
    let moved_time = resolution_time([
        local_entry.times.location_changed,
        remote_entry.times.location_changed,
    ]);
    if moved {
        let (ours, theirs) = match winner {
            Side::Local => (moved_time, Times::epoch()),
            Side::Remote => (Times::epoch(), moved_time),
        };
        if let Some(mut entry) = local.entry_mut(entry_id) {
            entry.times.location_changed = Some(ours);
        }
        if let Some(mut entry) = remote.entry_mut(entry_id) {
            entry.times.location_changed = Some(theirs);
        }
    }

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
            // "Live" = nowhere below the recycle bin. Testing the direct
            // parent only was not enough: deleting a group moves its entries
            // one level deeper, and those raised conflicts the user could not
            // find anywhere in the UI.
            recycle_bin_id
                .is_none_or(|bin| !super::document::group_is_within(db, e.parent().id(), bin))
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
            password: Zeroizing::new(e.get(fields::PASSWORD).unwrap_or("").to_string()),
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
        attachments: attachment_fingerprint(e),
        icon: IconShown::of(e.icon(), e.custom_icon().as_deref(), DEFAULT_ENTRY_ICON).into_owned(),
        quality_check: e.quality_check,
        parent: e.parent().id(),
        location_changed: e.times.location_changed,
        expiry: e
            .times
            .expires
            .unwrap_or(false)
            .then_some(e.times.expiry)
            .flatten(),
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
        diffs.push(diff_row(
            "Additional fields",
            &local_additional,
            &remote_additional,
            |fields| render_additional_fields(fields),
        ));
    }

    // A count, because attachment names and bytes are not a conflict row's
    // business, but decided on the attachments themselves: two different
    // sets of the same size read alike, and the row then vanished.
    diffs.push(diff_row(
        "Attachments",
        &local.attachments,
        &remote.attachments,
        |attachments| attachment_summary(attachments.len()),
    ));

    let local_protected = protected_field_names(&local.fields);
    let remote_protected = protected_field_names(&remote.fields);
    if !local_protected.is_empty() || !remote_protected.is_empty() {
        diffs.push(diff_row(
            "Protected fields",
            &local_protected,
            &remote_protected,
            |names| names.join(", "),
        ));
    }

    if local.expiry != remote.expiry {
        diffs.push(diff_row("Expires", &local.expiry, &remote.expiry, |at| {
            expiry_row(*at)
        }));
    }

    let metadata = metadata_differences(local, remote);
    if !metadata.is_empty() {
        // Per-side values, not a shared change list. The row is flagged as
        // differing, so rendering the same string in both columns asked the
        // user to choose between two identical cells.
        let render = |side: fn(&MetadataDifference) -> &String| {
            metadata
                .iter()
                .map(|difference| side(difference).as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        diffs.push(
            FieldDiff {
                sides_read_alike: false,
                label: "Entry settings".into(),
                local: String::new(),
                remote: String::new(),
                differs: true,
            }
            .with_values(
                render(|difference| &difference.local),
                render(|difference| &difference.remote),
            ),
        );
    }

    diffs
}

fn attachment_summary(count: usize) -> String {
    match count {
        1 => "1 attachment".into(),
        count => format!("{count} attachments"),
    }
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
        sides_read_alike: false,
        label: label.into(),
        local: render_field(local_value, always_redact),
        remote: render_field(remote_value, always_redact),
        // `Value` equality includes both the cleartext and its protected bit.
        differs: local_value != remote_value,
    }
    // Two passwords of one length redact to the same string, and the row then
    // asked the user to choose between two identical cells.
    .with_sides_told_apart()
}

fn render_field(value: Option<&Value<String>>, always_redact: bool) -> String {
    // Not the empty string: a field nobody set and a field set to nothing are
    // two different things, and a row that renders both the same way stops
    // saying which one the user is choosing.
    let Some(value) = value else {
        return "(not set)".to_string();
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

/// One entry-setting difference, with what each side actually holds.
struct MetadataDifference {
    local: String,
    remote: String,
}

/// An entry's Auto-Type settings, with every literal taken out of the
/// sequences. See `redact_sequence_literals`.
fn describe_autotype(autotype: Option<&AutoType>) -> String {
    let Some(autotype) = autotype else {
        return "no Auto-Type".to_string();
    };
    let default = autotype.default_sequence.as_deref().map_or_else(
        String::new,
        crate::autotype::sequence::redact_sequence_literals,
    );
    let windows: Vec<String> = autotype
        .associations
        .iter()
        .map(|association| {
            format!(
                "{} -> {}",
                association.window,
                crate::autotype::sequence::redact_sequence_literals(&association.sequence)
            )
        })
        .collect();
    format!(
        "Auto-Type enabled {:?}, sequence {default:?}, {} windows [{}]",
        autotype.enabled,
        windows.len(),
        windows.join(", ")
    )
}

fn metadata_differences(local: &EntrySnapshot, remote: &EntrySnapshot) -> Vec<MetadataDifference> {
    fn describe<T: std::fmt::Debug>(label: &str, value: &Option<T>) -> String {
        match value {
            Some(value) => format!("{label} {value:?}"),
            None => format!("no {label}"),
        }
    }

    let mut changed = Vec::new();
    let mut note = |local_side: String, remote_side: String| {
        changed.push(MetadataDifference {
            local: local_side,
            remote: remote_side,
        });
    };

    if local.view.autotype != remote.view.autotype {
        // Described, not dumped: a sequence is free text and can hold a
        // literal the user typed in, which is a credential on a screen whose
        // whole point is that it does not show them.
        note(
            describe_autotype(local.view.autotype.as_ref()),
            describe_autotype(remote.view.autotype.as_ref()),
        );
    }
    if local.view.custom_data != remote.view.custom_data {
        // Named, not counted: two different sets of the same size read alike,
        // and the user was then asked to choose between two identical cells.
        note(
            custom_data_summary(&ordered_custom_data(&local.view.custom_data)),
            custom_data_summary(&ordered_custom_data(&remote.view.custom_data)),
        );
    }
    if local.icon != remote.icon {
        note(icon_label(&local.icon), icon_label(&remote.icon));
    }
    if local.view.foreground_color != remote.view.foreground_color {
        note(
            describe("text colour", &local.view.foreground_color),
            describe("text colour", &remote.view.foreground_color),
        );
    }
    if local.view.background_color != remote.view.background_color {
        note(
            describe("background", &local.view.background_color),
            describe("background", &remote.view.background_color),
        );
    }
    if local.view.override_url != remote.view.override_url {
        note(
            describe("URL override", &local.view.override_url),
            describe("URL override", &remote.view.override_url),
        );
    }
    if local.quality_check != remote.quality_check {
        note(
            describe("quality check", &local.quality_check),
            describe("quality check", &remote.quality_check),
        );
    }
    changed
}

fn icon_label(icon: &IconShown<'_>) -> String {
    match icon {
        IconShown::BuiltIn(index) => format!("built-in icon {index}"),
        IconShown::Picture(_) | IconShown::Unresolved(_) => "custom icon".to_string(),
        IconShown::Default => "default icon".to_string(),
    }
}

fn tags_diff(local: &[String], remote: &[String]) -> FieldDiff {
    // Order-sensitive comparison: tags are technically a set in KeePass'
    // mental model, but in the file they're a Vec<String> and clients
    // (including ours) preserve write order. Treating reorder as a diff
    // is the simpler + safer behaviour.
    // Quoted, because a tag may contain the separator: one tag `a, b` reads
    // exactly like two tags `a` and `b` without it.
    fn rendered(tags: &[String]) -> String {
        tags.iter()
            .map(|tag| format!("{tag:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
    FieldDiff {
        sides_read_alike: false,
        label: "Tags".into(),
        local: rendered(local),
        remote: rendered(remote),
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

    /// Append one history version with a chosen timestamp. Version identity
    /// in KDBX is the `last_modification` time, so the tests set it directly
    /// rather than relying on wall-clock ordering.
    fn add_history_at(db: &mut Database, id: EntryId, notes: &str, at: NaiveDateTime) {
        let mut version = db.entry(id).expect("entry exists").deref().clone();
        version.set_unprotected(fields::NOTES, notes);
        version.times.last_modification = Some(at);
        version.history = None;
        let mut entry = db.entry_mut(id).expect("entry exists");
        entry.history.get_or_insert_default().add_entry(version);
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
    fn explicit_default_icon_vs_absent_icon_is_not_a_conflict() {
        // KeePassXC writes <IconID>0</IconID> on every entry; our fork omits
        // the element when unset. Both mean "default icon" - a vault
        // round-tripped through both clients must not conflict on it.
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "secret");
        local
            .entry_mut(id)
            .expect("local entry")
            .set_icon_builtin(0);
        let mut remote = fork(&local);
        remote.entry_mut(id).expect("remote entry").set_icon_none();

        let report = diff(&local, &remote);
        assert!(
            report.conflicts.is_empty(),
            "default-icon spelling must not conflict"
        );
        assert!(report.is_clean());
    }

    #[test]
    fn nil_previous_group_vs_absent_previous_group_is_not_a_difference() {
        assert!(group_uuids_equivalent(Some(uuid::Uuid::nil()), None,));

        let actual_group = uuid::Uuid::new_v4();
        assert!(!group_uuids_equivalent(Some(actual_group), None));
        assert!(!group_uuids_equivalent(
            Some(actual_group),
            Some(uuid::Uuid::new_v4()),
        ));
    }

    #[test]
    fn entry_previous_parent_metadata_never_conflicts() {
        // FerrisPass saves before the 0.7 fork bump stripped
        // PreviousParentGroup from every entry, so a foreign-written remote
        // carries a real group UUID where local has None - with tied
        // timestamps. Restore-location metadata must not surface as a
        // per-entry conflict (KeePass2's merge never prompts for it either).
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "secret");
        let mut remote = fork(&local);
        let remote_root = remote.root().id();
        remote
            .entry_mut(id)
            .expect("remote entry")
            .previous_parent_group = Some(remote_root);

        let report = diff(&local, &remote);
        assert!(
            report.conflicts.is_empty(),
            "previous-parent metadata must not conflict"
        );
        assert!(report.is_clean());
    }

    #[test]
    fn apply_picks_tolerates_stripped_previous_parent_metadata() {
        // Unsurfaced divergence + tied timestamps used to trip the fork's
        // fail-closed "same modification time but have diverged" check and
        // wedge sync with "Merge blocked". The merge must reconcile before
        // Database::merge runs - and heal: the real remote UUID survives
        // into the merged result instead of local's stripped None.
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "secret");
        let mut remote = fork(&local);
        let remote_root = remote.root().id();
        remote
            .entry_mut(id)
            .expect("remote entry")
            .previous_parent_group = Some(remote_root);

        let report = diff(&local, &remote);
        let merged = apply_picks(&local, &remote, &Resolutions::default(), &report)
            .expect("previous-parent metadata must not block the merge");
        assert_eq!(
            merged
                .entry(id)
                .expect("merged entry")
                .previous_parent_group,
            Some(remote_root),
            "the side carrying restore metadata must win"
        );
    }

    #[test]
    fn apply_picks_tolerates_default_icon_spelling_divergence() {
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "secret");
        local
            .entry_mut(id)
            .expect("local entry")
            .set_icon_builtin(0);
        let mut remote = fork(&local);
        remote.entry_mut(id).expect("remote entry").set_icon_none();

        let report = diff(&local, &remote);
        apply_picks(&local, &remote, &Resolutions::default(), &report)
            .expect("default-icon spelling must not block the merge");
    }

    #[test]
    fn apply_picks_breaks_tied_group_divergence_in_local_favor() {
        // KeePassXC toggles IsExpanded without bumping the group's
        // modification time, so both sides tie while the fork's fail-closed
        // check sees a divergence ("Groups with UUID … have the same
        // modification time but have diverged"). Groups have no conflict
        // UI - the merge must break the tie itself, keeping local.
        let mut local = Database::new();
        let group_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Infrastructure".into();
            group.is_expanded = true;
            group.id()
        };
        let mut remote = fork(&local);
        remote
            .group_mut(group_id)
            .expect("remote group")
            .is_expanded = false;

        let report = diff(&local, &remote);
        let merged = apply_picks(&local, &remote, &Resolutions::default(), &report)
            .expect("tied group view-state divergence must not block the merge");
        assert!(
            merged.group(group_id).expect("merged group").is_expanded,
            "local side must win the tie"
        );
    }

    #[test]
    fn non_default_icon_difference_still_conflicts() {
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "secret");
        local
            .entry_mut(id)
            .expect("local entry")
            .set_icon_builtin(5);
        let mut remote = fork(&local);
        remote.entry_mut(id).expect("remote entry").set_icon_none();

        let report = diff(&local, &remote);
        assert_eq!(
            report.conflicts.len(),
            1,
            "a real icon change must still surface"
        );
        assert!(
            report.conflicts[0]
                .fields
                .iter()
                .any(|f| f.label == "Entry settings" && f.differs),
            "icon divergence must show in the Entry settings diff row"
        );
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
        // Local-only doesn't require user decision - clean.
        assert!(report.is_clean());
    }

    #[test]
    fn remote_only_pull_is_a_pure_fast_forward() {
        // Remote gained an entry, local has nothing the server lacks. The
        // merge should be flagged as needing no upload - otherwise auto-sync
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
        // We hold an entry the server doesn't - the merge must be pushed so
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
        // Remote-only adds to the merged result, so it's NOT clean - the
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
        // Remote rotates differently - id is preserved across `fork` (which
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
            "newer remote should auto-resolve, not prompt - got {} conflicts",
            report.conflicts.len()
        );
        assert_eq!(report.auto_resolved.len(), 1);
        assert_eq!(report.auto_resolved[0].winner, Side::Remote);
        assert!(!report.is_clean(), "auto-resolved still requires writeback");

        let merged = apply_picks(&local, &remote, &Resolutions::default(), &report)
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

        let merged = apply_picks(&local, &remote, &Resolutions::default(), &report)
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
            let _ = bin;
            let _ = root;
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
        let merged = apply_picks(&local, &remote, &Resolutions::default(), &report)
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

        let merged = apply_picks(&local, &remote, &Resolutions::for_entries(picks), &report)
            .expect("remote pick should merge");
        let entry = merged.entry(id).unwrap();
        assert_eq!(entry.get_password(), Some("remote-pw"));
    }

    #[test]
    fn apply_picks_adds_remote_only_entries_to_root() {
        let local = Database::new();
        let mut remote = fork(&local);
        let remote_id = add(&mut remote, "NewRemote", "remote-secret");

        let report = diff(&local, &remote);
        let merged = apply_picks(&local, &remote, &Resolutions::default(), &report)
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
        // Quoted, so one tag containing the separator cannot read as two.
        assert_eq!(tag_field.local, "\"personal\"");
        assert_eq!(tag_field.remote, "\"work\", \"shared\"");

        // User picks Remote → all remote fields land on the merged entry,
        // including the tags.
        let mut picks = HashMap::new();
        picks.insert(id.to_string(), Side::Remote);
        let merged = apply_picks(&local, &remote, &Resolutions::for_entries(picks), &report)
            .expect("remote tags should merge");

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
    /// even when the remote side had explicitly removed it - and any
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
        let merged = apply_picks(&local, &remote, &Resolutions::for_entries(picks), &report)
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
        let merged = apply_picks(&local, &remote, &Resolutions::for_entries(picks), &report)
            .expect("OTP-aware merge should succeed");
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
        let merged = apply_picks(&local, &remote, &Resolutions::for_entries(picks), &report)
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
        let merged = apply_picks(&local, &remote, &Resolutions::default(), &report)
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
        let merged = apply_picks(&local, &remote, &Resolutions::default(), &report)
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
        let merged = apply_picks(&local, &remote, &Resolutions::default(), &report)
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
        let merged = apply_picks(&local, &remote, &Resolutions::for_entries(picks), &report)
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
    fn attachment_added_only_locally_is_preserved_by_merge() {
        let mut local = Database::new();
        let id = add(&mut local, "With attachment", "pw");
        let remote = fork(&local);
        local
            .entry_mut(id)
            .unwrap()
            .add_attachment("secret.bin", Value::protected(vec![1, 2, 3]));

        let report = diff(&local, &remote);
        let picks = HashMap::from([(id.to_string(), Side::Local)]);
        let merged = apply_picks(&local, &remote, &Resolutions::for_entries(picks), &report)
            .expect("the locally selected attachment must survive the merge");

        let entry = merged.entry(id).unwrap();
        let attachment = entry.attachment_by_name("secret.bin").unwrap();
        assert_eq!(attachment.data.get(), &[1, 2, 3]);
        assert!(attachment.data.is_protected());
    }

    #[test]
    fn equal_attachment_counts_with_different_data_can_pick_remote() {
        let mut local = Database::new();
        let id = add(&mut local, "Changed attachment", "pw");
        local
            .entry_mut(id)
            .unwrap()
            .add_attachment("secret.bin", Value::protected(vec![1, 2, 3]));
        let mut remote = fork(&local);
        remote
            .entry_mut(id)
            .unwrap()
            .attachment_by_name_mut("secret.bin")
            .unwrap()
            .data = Value::protected(vec![4, 5, 6]);

        let report = diff(&local, &remote);
        assert_eq!(report.conflicts.len(), 1);
        assert!(
            report.conflicts[0]
                .fields
                .iter()
                .any(|field| field.label == "Attachments" && field.differs)
        );

        let picks = HashMap::from([(id.to_string(), Side::Remote)]);
        let merged = apply_picks(&local, &remote, &Resolutions::for_entries(picks), &report)
            .expect("remote attachment bytes must be selectable");
        let entry = merged.entry(id).unwrap();
        let attachment = entry.attachment_by_name("secret.bin").unwrap();
        assert_eq!(attachment.data.get(), &[4, 5, 6]);
        assert!(attachment.data.is_protected());
        assert_eq!(
            merged.num_attachments(),
            2,
            "losing bytes remain in history"
        );
    }

    #[test]
    fn attachment_rename_with_unchanged_bytes_is_detected_and_applied() {
        let mut local = Database::new();
        let id = add(&mut local, "Renamed attachment", "pw");
        local
            .entry_mut(id)
            .unwrap()
            .add_attachment("old.bin", Value::unprotected(vec![7, 8, 9]));
        let mut remote = fork(&local);
        {
            let mut entry = remote.entry_mut(id).unwrap();
            entry.remove_attachment_by_name("old.bin");
            entry.add_attachment("new.bin", Value::unprotected(vec![7, 8, 9]));
        }

        let report = diff(&local, &remote);
        assert_eq!(report.conflicts.len(), 1);
        let picks = HashMap::from([(id.to_string(), Side::Remote)]);
        let merged = apply_picks(&local, &remote, &Resolutions::for_entries(picks), &report)
            .expect("attachment rename must be mergeable");
        let entry = merged.entry(id).unwrap();
        assert!(entry.attachment_by_name("old.bin").is_none());
        assert_eq!(
            entry.attachment_by_name("new.bin").unwrap().data.get(),
            &[7, 8, 9]
        );
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
        let merged = apply_picks(&local, &remote, &Resolutions::for_entries(picks), &report)
            .expect("identical attachment stores must not block conflict resolution");

        assert_eq!(merged.num_attachments(), 1);
        assert_eq!(
            merged.entry(id).unwrap().get_password(),
            Some("remote-newer")
        );
    }

    #[test]
    fn policy_resolved_merge_warnings_are_not_fatal() {
        let mut db = Database::new();
        let entry = add(&mut db, "Any", "pw");
        let group = db.root().id();

        // Outcomes the fork already resolved without losing anything.
        for benign in [
            MergeWarning::DivergedHistory {
                entry,
                at: Times::epoch(),
            },
            MergeWarning::CannotMoveRootGroup { group },
            MergeWarning::MissingHistoryTimestamp {
                side: keepass::db::MergeSide::Destination,
                entry,
            },
            MergeWarning::MissingHistoryTimestamp {
                side: keepass::db::MergeSide::Source,
                entry,
            },
            MergeWarning::NoHistory {
                side: keepass::db::MergeSide::Source,
                entry,
            },
        ] {
            assert!(
                warning_is_policy_resolved(&benign),
                "misclassified: {benign}"
            );
        }
        // Everything that silently discards a remote change stays fatal:
        // dropped entries, discarded moves, unorderable moves (the fork
        // keeps local and the next upload overwrites the remote move), and
        // missing timestamps on *current* entries/groups (a remote rename
        // would lose against the epoch substitute).
        for lossy in [
            MergeWarning::CannotAddEntry {
                entry,
                parent: group,
            },
            MergeWarning::CannotMoveEntry { entry, into: group },
            MergeWarning::CannotMoveGroup { group, into: group },
            MergeWarning::AmbiguousEntryMove { entry },
            MergeWarning::AmbiguousGroupMove { group },
            MergeWarning::MissingEntryTimestamp {
                side: keepass::db::MergeSide::Source,
                entry,
            },
            MergeWarning::MissingGroupTimestamp {
                side: keepass::db::MergeSide::Destination,
                group,
            },
        ] {
            assert!(
                !warning_is_policy_resolved(&lossy),
                "misclassified: {lossy}"
            );
        }
    }

    /// The fork's `Display` text is what the user sees in "Merge blocked",
    /// and what earlier versions of this module classified by. Pinning it
    /// keeps a reworded message a visible change rather than a silent one.
    #[test]
    fn fork_warning_wording_is_pinned() {
        let mut db = Database::new();
        let entry = add(&mut db, "Any", "pw");
        let group = db.root().id();

        assert_eq!(
            MergeWarning::CannotAddEntry {
                entry,
                parent: group
            }
            .to_string(),
            format!(
                "Cannot add entry {entry} because its parent group {group} does not exist in the \
                 destination database."
            )
        );
        assert_eq!(
            MergeWarning::MissingEntryTimestamp {
                side: keepass::db::MergeSide::Source,
                entry
            }
            .to_string(),
            format!("Source entry {entry} did not have a last modification timestamp")
        );
        assert_eq!(
            MergeWarning::CannotMoveRootGroup { group }.to_string(),
            format!("Cannot move root group {group}")
        );
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
        assert!(log.warnings.contains(&MergeWarning::MissingEntryTimestamp {
            side: keepass::db::MergeSide::Source,
            entry: entry_id,
        }));

        let report = diff(&local, &remote);
        let merged = apply_picks(&local, &remote, &Resolutions::default(), &report)
            .expect("resolved move and history must not make timestamp warning fatal");
        let entry = merged.entry(entry_id).unwrap();
        assert_eq!(entry.parent().id(), target_id);
        assert_eq!(entry.history.as_ref().unwrap().get_entries().len(), 1);
    }

    /// An unorderable move used to be fail-closed: nothing surfaced it, so
    /// the merge refused and the user had no way to answer. It is a conflict
    /// now, with a row saying where each side put the entry, and the answer
    /// decides the placement.
    #[test]
    fn an_unorderable_entry_move_is_asked_rather_than_refused() {
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
        assert!(log.warnings.contains(&MergeWarning::MissingEntryTimestamp {
            side: keepass::db::MergeSide::Source,
            entry: entry_id,
        }));

        let report = diff(&local, &remote);
        let conflict = report
            .conflicts
            .first()
            .expect("nothing can rank the move, so it has to be asked");
        assert!(
            conflict
                .fields
                .iter()
                .any(|field| field.label == "Location" && field.local != field.remote),
            "and the row has to say where each side put it: {:?}",
            conflict.fields
        );

        let picks = Resolutions {
            entries: HashMap::from([(entry_id.to_string(), Side::Remote)]),
            groups: HashMap::new(),
            metadata: None,
        };
        let merged = apply_picks(&local, &remote, &picks, &report).expect("resolvable");
        assert_eq!(
            merged.entry(entry_id).expect("the entry").parent().id(),
            target_id,
            "choosing theirs has to put it where they put it"
        );
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
        assert!(log.warnings.contains(&MergeWarning::MissingGroupTimestamp {
            side: keepass::db::MergeSide::Source,
            group: group_id,
        }));

        let report = diff(&local, &remote);
        let merged = apply_picks(&local, &remote, &Resolutions::default(), &report)
            .expect("group membership must not make timestamp warning fatal");
        assert_eq!(merged.entry(child_id).unwrap().parent().id(), group_id);
    }

    #[test]
    fn missing_group_timestamp_with_divergent_content_is_a_question() {
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
        assert!(log.warnings.contains(&MergeWarning::MissingGroupTimestamp {
            side: keepass::db::MergeSide::Source,
            group: group_id,
        }));

        // A missing timestamp cannot rank the two sides, so this is a
        // question, not a wedge. It used to be fatal, which meant sync
        // stopped for good against a vault some other client wrote without
        // timestamps.
        let report = diff(&local, &remote);
        assert_eq!(report.group_conflicts.len(), 1);
        let merged =
            apply_picks(&local, &remote, &Resolutions::default(), &report).expect("resolvable");
        assert_eq!(
            merged.group(group_id).expect("group").name,
            "Local name",
            "and the default keeps this machine's version"
        );

        // An id neither side carries remains fail-closed as well: the
        // equivalence check cannot confirm anything about an object it
        // cannot find.
        let unknown = MergeWarning::MissingEntryTimestamp {
            side: keepass::db::MergeSide::Source,
            entry: add(&mut Database::new(), "Elsewhere", "pw"),
        };
        assert!(!warning_is_harmless(&unknown, &local, &remote));
    }

    /// A remote-only custom icon used to be fatal: the fork copied the icon
    /// reference but not the image, so the merge was refused outright rather
    /// than write a reference nothing could resolve. Downloading favicons on
    /// one machine therefore made that vault unsyncable. The fork now carries
    /// the image, and the merge has to succeed with the picture intact.
    #[test]
    fn a_remote_only_custom_icon_survives_the_merge() {
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
        let id = add(&mut local, "With custom icon", "pw");
        local.entry_mut(id).unwrap().times.last_modification = Some(older);
        let mut remote = fork(&local);
        let image = vec![1, 2, 3];
        remote
            .entry_mut(id)
            .unwrap()
            .set_icon_custom_new(image.clone());
        remote.entry_mut(id).unwrap().times.last_modification = Some(newer);

        let merged = apply_picks(
            &local,
            &remote,
            &Resolutions::default(),
            &diff(&local, &remote),
        )
        .expect("a remote icon is merged, not refused");

        let entry = merged.entry(id).expect("entry survives");
        assert_eq!(
            entry.custom_icon().expect("icon resolves").data,
            image,
            "the image travelled with the reference"
        );
    }

    /// The merge lets a reference share a picture the other copy already
    /// holds under its own id, so one round trip leaves both copies showing
    /// the same favicon under two ids, with the entry's clock untouched.
    /// Compared by id, that asked about every entry with a favicon, and
    /// either answer changed nothing on screen.
    #[test]
    fn the_same_picture_under_two_ids_is_one_entry_icon() {
        let tied = keepass::db::Times::now();
        let picture = vec![0x89, b'P', b'N', b'G', 7];

        let mut local = Database::new();
        let id = add(&mut local, "AdWords", "pw");
        let ours = local
            .entry_mut(id)
            .unwrap()
            .set_icon_custom_new(picture.clone())
            .id();
        local.entry_mut(id).unwrap().times.last_modification = Some(tied);

        let mut remote = fork(&local);
        let theirs = remote
            .entry_mut(id)
            .unwrap()
            .set_icon_custom_new(picture.clone())
            .id();
        remote.entry_mut(id).unwrap().times.last_modification = Some(tied);
        assert_ne!(ours, theirs, "one picture, two ids");

        let report = diff(&local, &remote);
        assert!(
            report.conflicts.is_empty(),
            "nothing to ask: {:?}",
            report.conflicts
        );
        assert!(
            !report.has_local_contribution(),
            "and nothing to send back either"
        );

        let merged = apply_picks(&local, &remote, &Resolutions::default(), &report)
            .expect("the fork's tie check sees one icon");
        let entry = merged.entry(id).expect("entry survives");
        assert_eq!(
            entry.icon(),
            Some(&Icon::Custom(ours)),
            "this copy keeps its own reference"
        );
        assert_eq!(entry.custom_icon().expect("which resolves").data, picture);
    }

    /// The same for a group, which has its own comparison, its own
    /// conflict list and its own alignment before the fork's merge.
    #[test]
    fn the_same_picture_under_two_ids_is_one_group_icon() {
        let tied = keepass::db::Times::now();
        let picture = vec![0x89, b'P', b'N', b'G', 8];

        let mut local = Database::new();
        let group_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Banking".to_string();
            group.id()
        };
        let ours = local
            .group_mut(group_id)
            .unwrap()
            .set_icon_custom_new(picture.clone())
            .id();
        local.group_mut(group_id).unwrap().times.last_modification = Some(tied);

        let mut remote = fork(&local);
        let theirs = remote
            .group_mut(group_id)
            .unwrap()
            .set_icon_custom_new(picture.clone())
            .id();
        remote.group_mut(group_id).unwrap().times.last_modification = Some(tied);
        assert_ne!(ours, theirs, "one picture, two ids");

        let report = diff(&local, &remote);
        assert!(
            report.group_conflicts.is_empty(),
            "nothing to ask: {:?}",
            report.group_conflicts
        );
        assert!(
            !report.has_local_contribution(),
            "a reference is not a contribution"
        );

        let merged = apply_picks(&local, &remote, &Resolutions::default(), &report)
            .expect("the fork's tie check sees one icon");
        let group = merged.group(group_id).expect("group survives");
        assert_eq!(
            group.icon(),
            Some(&Icon::Custom(ours)),
            "this copy keeps its own reference"
        );
        assert_eq!(group.custom_icon().expect("which resolves").data, picture);
    }

    /// The other copy can keep a different picture under this copy's id
    /// while showing this picture under its own. Adopting the picture then
    /// minted a fresh id on that side alone, the two references still
    /// differed, and the fork refused the tie. Both sides now follow the
    /// fresh id.
    #[test]
    fn a_picture_whose_id_is_taken_on_the_other_side_is_shared_under_a_fresh_one() {
        let tied = keepass::db::Times::now();
        let picture = vec![0x89, b'P', b'N', b'G', 10];
        let other = vec![0x89, b'P', b'N', b'G', 11];

        let mut local = Database::new();
        let id = add(&mut local, "AdWords", "pw");
        let ours = local
            .entry_mut(id)
            .unwrap()
            .set_icon_custom_new(picture.clone())
            .id();
        local.entry_mut(id).unwrap().times.last_modification = Some(tied);

        let mut remote = fork(&local);
        // Something there keeps our id alive, then repaints it.
        let bystander = add(&mut remote, "Analytics", "pw");
        remote
            .entry_mut(bystander)
            .unwrap()
            .set_icon_custom(ours)
            .expect("the id is here");
        remote
            .entry_mut(id)
            .unwrap()
            .set_icon_custom_new(picture.clone());
        remote.entry_mut(id).unwrap().times.last_modification = Some(tied);
        remote.custom_icon_mut(ours).expect("still referenced").data = other.clone();

        let report = diff(&local, &remote);
        assert!(
            report.conflicts.is_empty(),
            "the entry shows one picture on both sides: {:?}",
            report.conflicts
        );

        let merged = apply_picks(&local, &remote, &Resolutions::default(), &report)
            .expect("shared under a fresh id");
        assert_eq!(
            merged
                .entry(id)
                .unwrap()
                .custom_icon()
                .expect("resolves")
                .data,
            picture
        );
        assert_eq!(
            merged
                .entry(bystander)
                .unwrap()
                .custom_icon()
                .expect("resolves")
                .data,
            other,
            "and the other picture arrived with its entry"
        );
    }

    /// A merge retargets an archived version's custom icon too, to whichever
    /// id the picture already has on the receiving side, so after one round
    /// trip one version references two ids on the two copies. Digested by
    /// reference, each side looked to hold a version the other had never
    /// seen, and both uploaded on every sync.
    #[test]
    fn an_archived_version_showing_the_same_picture_under_another_id_is_the_same_version() {
        use chrono::NaiveDate;
        let at = NaiveDate::from_ymd_opt(2026, 5, 7)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        let picture = vec![0x89, b'P', b'N', b'G', 9];

        let mut local = Database::new();
        let id = add(&mut local, "AdWords", "pw");
        local
            .entry_mut(id)
            .unwrap()
            .set_icon_custom_new(picture.clone());
        let mut remote = fork(&local);
        remote
            .entry_mut(id)
            .unwrap()
            .set_icon_custom_new(picture.clone());
        // The same version on both sides, each carrying its copy's id.
        add_history_at(&mut local, id, "archived", at);
        add_history_at(&mut remote, id, "archived", at);

        let report = diff(&local, &remote);
        assert!(
            !report.local_history_ahead,
            "the other copy has this version, under its own id"
        );
        assert!(!report.has_local_contribution());
    }

    /// Expanding a group in KeePassXC writes `IsExpanded` without touching
    /// the modification time, so two copies routinely disagree on it while
    /// their clocks tie.
    ///
    /// That used to trip the fork's fail-closed check, and this module bought
    /// its way past it by stamping the group as modified now. The stamp
    /// outlives the merge: the next sync reads it as a real edit and lets it
    /// win last-write-wins against somebody's actual rename. The fork ignores
    /// view state itself now, so there is nothing left to buy off.
    #[test]
    fn a_group_that_differs_only_in_view_state_keeps_its_modification_time() {
        use chrono::NaiveDate;
        let at = NaiveDate::from_ymd_opt(2026, 5, 7)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();

        let mut local = Database::new();
        let group_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Banking".to_string();
            group.id()
        };
        local.group_mut(group_id).unwrap().is_expanded = true;
        local.group_mut(group_id).unwrap().times.last_modification = Some(at);

        // Collapsed there, and nothing else about it differs.
        let mut remote = fork(&local);
        remote.group_mut(group_id).unwrap().is_expanded = false;

        let report = diff(&local, &remote);
        assert!(
            report.group_conflicts.is_empty(),
            "view state is nobody's edit: {:?}",
            report.group_conflicts
        );

        let merged = apply_picks(&local, &remote, &Resolutions::default(), &report)
            .expect("view state is not a divergence");

        assert_eq!(
            merged
                .group(group_id)
                .expect("group survives")
                .times
                .last_modification,
            Some(at),
            "a collapsed twisty is not an edit to stamp"
        );
    }

    /// The same version, and the merge has to end with one of it.
    ///
    /// This module can align the reference on a current entry before handing
    /// the two files to the fork, but an archived version is inside the entry
    /// the fork merges, out of reach. The fork ranked those by their raw
    /// reference, kept both and warned, so a vault with favicons grew a
    /// duplicate version per entry per round trip until the history cap
    /// trimmed real versions away to make room.
    #[test]
    fn an_archived_version_seen_on_both_sides_survives_the_merge_once() {
        use chrono::NaiveDate;
        let at = NaiveDate::from_ymd_opt(2026, 5, 7)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        let picture = vec![0x89, b'P', b'N', b'G', 12];

        let mut local = Database::new();
        let id = add(&mut local, "AdWords", "pw");
        local
            .entry_mut(id)
            .unwrap()
            .set_icon_custom_new(picture.clone());
        let mut remote = fork(&local);
        remote
            .entry_mut(id)
            .unwrap()
            .set_icon_custom_new(picture.clone());
        add_history_at(&mut local, id, "archived", at);
        add_history_at(&mut remote, id, "archived", at);

        let merged = apply_picks(
            &local,
            &remote,
            &Resolutions::default(),
            &diff(&local, &remote),
        )
        .expect("one picture on both sides is not a divergence");

        assert_eq!(
            merged
                .entry(id)
                .expect("entry survives")
                .history
                .as_ref()
                .map_or(0, |history| history.get_entries().len()),
            1,
            "one version, not one per copy"
        );
    }

    /// Last-write-wins is decided by a number inside the shared file. Anyone
    /// who can write that file could stamp a far-future
    /// `LastModificationTime` and have their version silently replace
    /// everyone else's on the next sync, with nothing shown to the user.
    #[test]
    fn a_remote_timestamp_from_the_future_asks_instead_of_winning() {
        let mut local = Database::new();
        let id = add(&mut local, "Bank", "local-password");
        local.entry_mut(id).unwrap().times.last_modification =
            Some(keepass::db::Times::now() - chrono::TimeDelta::hours(1));
        let mut remote = fork(&local);
        remote
            .entry_mut(id)
            .unwrap()
            .set_protected(fields::PASSWORD, "attacker-password");
        remote.entry_mut(id).unwrap().times.last_modification =
            Some(keepass::db::Times::now() + chrono::TimeDelta::days(365));

        let report = diff(&local, &remote);

        assert!(
            report.auto_resolved.is_empty(),
            "an unbelievable timestamp must not win silently"
        );
        assert_eq!(
            report.conflicts.len(),
            1,
            "it becomes a decision the user gets to see"
        );
        assert_eq!(
            report.future_dated,
            vec![id.to_string()],
            "and the overlay can say why it is being asked"
        );
    }

    /// A resolution that ties with the version it replaced settles nothing:
    /// a third copy still holding the other side at that timestamp meets it
    /// as a fresh tie and asks again. Taking the maximum of the believable
    /// candidates did exactly that when both sides were tied inside the skew
    /// window.
    #[test]
    fn a_resolution_is_strictly_newer_than_the_versions_it_replaces() {
        let tied = keepass::db::Times::now() + chrono::TimeDelta::minutes(5);
        let mut local = Database::new();
        let id = add(&mut local, "Bank", "local-password");
        local.entry_mut(id).unwrap().times.last_modification = Some(tied);
        let mut remote = fork(&local);
        remote
            .entry_mut(id)
            .unwrap()
            .set_protected(fields::PASSWORD, "remote-password");
        remote.entry_mut(id).unwrap().times.last_modification = Some(tied);

        let report = diff(&local, &remote);
        assert_eq!(report.conflicts.len(), 1, "tied timestamps ask");
        let picks = HashMap::from([(id.to_string(), Side::Local)]);
        let merged = apply_picks(&local, &remote, &Resolutions::for_entries(picks), &report)
            .expect("resolvable");

        let resolved = merged
            .entry(id)
            .expect("entry survives")
            .times
            .last_modification
            .expect("resolution stamps a time");
        assert!(
            resolved > tied,
            "the resolution has to outrank both sides, not tie with them: {resolved} vs {tied}"
        );
        assert!(
            resolved <= keepass::db::Times::now() + MAX_CLOCK_SKEW,
            "and still stay inside the horizon: {resolved}"
        );
    }

    /// Keeping Local against a far-future remote used to stamp the local
    /// result with the remote's timestamp, because the winner took the
    /// maximum of both. The attacker then won every later comparison anyway,
    /// and the user's next real edit looked older than the thing it replaced.
    #[test]
    fn keeping_local_does_not_adopt_an_unbelievable_timestamp() {
        let mut local = Database::new();
        let id = add(&mut local, "Bank", "local-password");
        local.entry_mut(id).unwrap().times.last_modification =
            Some(keepass::db::Times::now() - chrono::TimeDelta::hours(1));
        let mut remote = fork(&local);
        remote
            .entry_mut(id)
            .unwrap()
            .set_protected(fields::PASSWORD, "attacker-password");
        remote.entry_mut(id).unwrap().times.last_modification =
            Some(keepass::db::Times::now() + chrono::TimeDelta::days(365));

        let report = diff(&local, &remote);
        let picks = HashMap::from([(id.to_string(), Side::Local)]);
        let merged = apply_picks(&local, &remote, &Resolutions::for_entries(picks), &report)
            .expect("resolvable");

        let resolved = merged
            .entry(id)
            .expect("entry survives")
            .times
            .last_modification
            .expect("resolution stamps a time");
        assert!(
            resolved <= keepass::db::Times::now() + MAX_CLOCK_SKEW,
            "the merged result must stay inside the skew horizon: {resolved}"
        );
        assert!(
            resolved >= keepass::db::Times::now() - chrono::TimeDelta::minutes(1),
            "and still be newer than the version it replaced: {resolved}"
        );
    }

    /// Ordinary clock drift between two machines still resolves on its own.
    /// A prompt on every sync would be worse than the problem.
    #[test]
    fn a_remote_timestamp_within_clock_skew_still_wins() {
        let mut local = Database::new();
        let id = add(&mut local, "Bank", "local-password");
        local.entry_mut(id).unwrap().times.last_modification =
            Some(keepass::db::Times::now() - chrono::TimeDelta::hours(1));
        let mut remote = fork(&local);
        remote
            .entry_mut(id)
            .unwrap()
            .set_protected(fields::PASSWORD, "colleague-password");
        remote.entry_mut(id).unwrap().times.last_modification =
            Some(keepass::db::Times::now() + chrono::TimeDelta::minutes(2));

        let report = diff(&local, &remote);

        assert!(report.conflicts.is_empty(), "no prompt for ordinary drift");
        assert_eq!(report.auto_resolved.len(), 1);
        assert!(matches!(report.auto_resolved[0].winner, Side::Remote));
    }

    /// Trimming diverges histories without anyone editing anything: a vault
    /// drops its oldest versions once it reaches its own `HistoryMaxItems`.
    /// Reading that as a local contribution made `has_local_contribution`
    /// always true, so a pure pull still uploaded and minted a redundant
    /// remote version.
    ///
    /// This case is history-only on purpose. The test that used to carry this
    /// name also changed the remote Notes value, so the auto-resolved entry
    /// carried the assertion and the history rule was never exercised.
    #[test]
    fn history_the_remote_trimmed_away_does_not_force_an_upload() {
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "secret");
        let oldest = keepass::db::Times::now() - chrono::TimeDelta::minutes(30);
        let newer = keepass::db::Times::now() - chrono::TimeDelta::minutes(10);
        add_history_at(&mut local, id, "v1", oldest);
        add_history_at(&mut local, id, "v2", newer);

        // Both vaults keep one version, so the remote is at its limit and the
        // older one is demonstrably something it dropped rather than
        // something it never had. The cap is set on both sides because a cap
        // that differs is itself a setting the merge has to write back.
        local.meta.history_max_items = Some(1);
        let mut remote = fork(&local);
        remote.entry_mut(id).unwrap().history = None;
        add_history_at(&mut remote, id, "v2", newer);

        let report = diff(&local, &remote);

        assert!(report.conflicts.is_empty(), "no current field differs");
        assert!(
            !report.local_history_ahead,
            "a full history that lacks an older version trimmed it"
        );
        assert!(
            !report.has_local_contribution(),
            "a pure pull is a fast-forward and needs no upload"
        );
    }

    /// Renaming a custom icon changes no entry, no group and no setting, so
    /// nothing reported it as something to send and the other copy's next
    /// upload put the old name back.
    #[test]
    fn a_renamed_custom_icon_is_a_local_contribution() {
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "secret");
        let earlier = keepass::db::Times::now() - chrono::TimeDelta::minutes(10);
        let icon_id = {
            let mut entry = local.entry_mut(id).unwrap();
            let mut icon = entry.set_icon_custom_new(vec![0x89, b'P', b'N', b'G', 3]);
            icon.name = Some("Old name".into());
            icon.last_modification_time = Some(earlier);
            icon.id()
        };
        let remote = fork(&local);

        assert!(
            !diff(&local, &remote).has_local_contribution(),
            "identical copies need nothing"
        );

        {
            let mut icon = local.custom_icon_mut(icon_id).unwrap();
            icon.name = Some("Bank".into());
            icon.last_modification_time = Some(keepass::db::Times::now());
        }
        assert!(
            diff(&local, &remote).structural_writeback_required,
            "a rename here has to reach the other copy"
        );

        // And a name we cannot show is ours is not ours to send: the merge
        // takes theirs, so this is a pull.
        let mut theirs = fork(&remote);
        {
            let mut icon = theirs.custom_icon_mut(icon_id).unwrap();
            icon.name = Some("Their name".into());
            icon.last_modification_time =
                Some(keepass::db::Times::now() + chrono::TimeDelta::minutes(1));
        }
        assert!(
            !diff(&remote, &theirs).structural_writeback_required,
            "adopting their name is a pull, not something to send back"
        );
    }

    /// Icon timestamps have one-second precision, so two renames can tie. The
    /// merge keeps this copy's name on a tie and the next upload wrote it
    /// over theirs, which is the same silent loss as a tied database name.
    #[test]
    fn a_tied_custom_icon_name_is_asked_about() {
        let tied = keepass::db::Times::now();
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "secret");
        let icon_id = {
            let mut entry = local.entry_mut(id).unwrap();
            let mut icon = entry.set_icon_custom_new(vec![0x89, b'P', b'N', b'G', 4]);
            icon.name = Some("Ours".into());
            icon.last_modification_time = Some(tied);
            icon.id()
        };
        let mut remote = fork(&local);
        {
            let mut icon = remote.custom_icon_mut(icon_id).unwrap();
            icon.name = Some("Theirs".into());
            icon.last_modification_time = Some(tied);
        }

        // An undated pair is the same ambiguity: older clients wrote no time
        // at all, and nothing there says which name came last either.
        let mut undated_local = fork(&local);
        let mut undated_remote = fork(&remote);
        for db in [&mut undated_local, &mut undated_remote] {
            db.custom_icon_mut(icon_id).unwrap().last_modification_time = None;
        }
        assert!(
            diff(&undated_local, &undated_remote)
                .metadata_conflict
                .is_some(),
            "two names, no times: still the user's to settle"
        );

        let report = diff(&local, &remote);
        let conflict = report
            .metadata_conflict
            .as_ref()
            .expect("two names, one second: the user has to choose");
        assert!(
            conflict
                .fields
                .iter()
                .any(|field| field.label.starts_with("Icon name")
                    && field.local == "Ours"
                    && field.remote == "Theirs"),
            "and the row has to say what each side calls it: {:?}",
            conflict.fields
        );

        let picks = Resolutions {
            entries: HashMap::new(),
            groups: HashMap::new(),
            metadata: Some(Side::Remote),
        };
        let merged = apply_picks(&local, &remote, &picks, &report).expect("resolvable");
        assert_eq!(
            merged
                .custom_icon(icon_id)
                .expect("the icon")
                .name
                .as_deref(),
            Some("Theirs"),
            "choosing theirs has to apply theirs"
        );
    }

    /// The default icon has two spellings, and an archived version has to be
    /// read through the same normalisation as a current one. Hashing the raw
    /// pair reported both sides ahead after an ordinary KeePassXC round trip,
    /// which uploads and downloads the same file forever.
    #[test]
    fn the_two_spellings_of_the_default_icon_are_one_history_version() {
        let same_second = keepass::db::Times::now() - chrono::TimeDelta::minutes(10);
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "secret");
        let mut remote = fork(&local);

        // Each side archives a version carrying its own spelling of the same
        // default icon, then leaves the current entry identical, so only the
        // archived versions differ.
        for (db, written_out) in [(&mut local, false), (&mut remote, true)] {
            if written_out {
                db.entry_mut(id).expect("entry").set_icon_builtin(0);
            }
            let mut version = db.entry(id).expect("entry").deref().clone();
            version.times.last_modification = Some(same_second);
            version.history = None;
            db.entry_mut(id)
                .expect("entry")
                .history
                .get_or_insert_default()
                .add_entry(version);
            db.entry_mut(id).expect("entry").set_icon_none();
        }

        let report = diff(&local, &remote);

        assert!(
            !report.local_history_ahead,
            "one icon, written two ways, is one version"
        );
        assert!(!report.has_local_contribution());
    }

    /// Two archived versions written in the same second can differ by
    /// anything an entry carries, not just by a field value.
    ///
    /// The digest covered the field map alone, so a version that differed
    /// only by a tag looked identical to one that did not have it: the fork
    /// merged both, this reported neither side as ahead, and the version that
    /// existed on one machine was never sent from it while the status said
    /// everything was in sync.
    #[test]
    fn a_history_version_that_differs_only_by_a_tag_is_still_ours_to_send() {
        let same_second = keepass::db::Times::now() - chrono::TimeDelta::minutes(10);
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "secret");
        let mut remote = fork(&local);

        // The same archived version on both sides, except for one tag.
        for (db, tags) in [
            (&mut local, vec!["work".to_string()]),
            (&mut remote, Vec::new()),
        ] {
            let mut version = db.entry(id).expect("entry").deref().clone();
            version.times.last_modification = Some(same_second);
            version.tags = tags;
            version.history = None;
            db.entry_mut(id)
                .expect("entry")
                .history
                .get_or_insert_default()
                .add_entry(version);
        }

        let report = diff(&local, &remote);

        assert!(report.conflicts.is_empty(), "no current field differs");
        assert!(
            report.local_history_ahead,
            "a tagged version the other side does not hold is ours to send"
        );
        assert!(report.has_local_contribution());
    }

    /// KeePass trims a history by two limits, and reading only the item count
    /// meant a copy that had trimmed by size, while still well under its item
    /// limit, looked like one that had never seen those versions. We sent
    /// them again, it dropped them again, on every single sync.
    #[test]
    fn a_history_the_remote_trimmed_by_size_does_not_force_an_upload() {
        let oldest = keepass::db::Times::now() - chrono::TimeDelta::minutes(30);
        let newer = keepass::db::Times::now() - chrono::TimeDelta::minutes(10);
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "secret");
        let bulky = "x".repeat(4096);
        add_history_at(&mut local, id, &bulky, oldest);
        add_history_at(&mut local, id, &format!("{bulky}!"), newer);

        // Room for ten versions on both sides, but a byte budget that only
        // one of these fits into.
        local.meta.history_max_items = Some(10);
        local.meta.history_max_size = Some(5000);
        let mut remote = fork(&local);
        remote.entry_mut(id).unwrap().history = None;
        add_history_at(&mut remote, id, &format!("{bulky}!"), newer);

        let report = diff(&local, &remote);

        assert!(report.conflicts.is_empty(), "no current field differs");
        assert!(
            !report.local_history_ahead,
            "their budget is full, so the version they lack is one they dropped"
        );
        assert!(
            !report.has_local_contribution(),
            "and sending it again would only have it dropped again"
        );

        // The boundary, taken from the estimate itself rather than guessed:
        // a client trims when the history goes over its budget, so one that
        // fits exactly still had room.
        let versions = |db: &Database| history_versions(&db.entry(id).expect("entry"));
        let held: usize = versions(&remote).iter().map(|version| version.bytes).sum();
        let missing = versions(&local)
            .difference(&versions(&remote))
            .map(|version| version.bytes)
            .min()
            .expect("one version they lack");

        for (budget, expected) in [
            (held + missing, true),
            (held + missing - 1, false),
            (1_000_000, true),
            // A budget of nothing is a budget, not the absence of one.
            (0, false),
        ] {
            let mut ours = fork(&local);
            ours.meta.history_max_size = Some(budget as isize);
            let mut theirs = fork(&remote);
            theirs.meta.history_max_size = Some(budget as isize);
            assert_eq!(
                diff(&ours, &theirs).local_history_ahead,
                expected,
                "budget {budget} against {held} held and {missing} missing"
            );
        }

        // And what a version costs is more than its field map: a history
        // trimmed because of tags or an override url was trimmed all the same.
        let plain = {
            let mut db = Database::new();
            let entry = add(&mut db, "Extra", "secret");
            add_history_at(&mut db, entry, "note", newer);
            history_version_bytes(&db.entry(entry).unwrap().historical(0).expect("a version"))
        };
        let richer = {
            let mut db = Database::new();
            let entry = add(&mut db, "Extra", "secret");
            db.entry_mut(entry).unwrap().tags = vec!["t".repeat(64)];
            db.entry_mut(entry).unwrap().override_url = Some("u".repeat(32));
            add_history_at(&mut db, entry, "note", newer);
            history_version_bytes(&db.entry(entry).unwrap().historical(0).expect("a version"))
        };
        assert!(
            richer >= plain + 96,
            "tags and an override url count too: {richer} against {plain}"
        );
    }

    /// The same shape, but the other side is nowhere near its limit, so it
    /// cannot have trimmed anything: it simply never received this version.
    /// Reading that as trimming meant the only copy stayed on one machine
    /// while the status pill said everything was in sync.
    #[test]
    fn an_old_version_a_roomy_remote_lacks_is_still_uploaded() {
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "secret");
        let older = keepass::db::Times::now() - chrono::TimeDelta::minutes(30);
        let shared = keepass::db::Times::now() - chrono::TimeDelta::minutes(10);
        add_history_at(&mut local, id, "local-only", older);
        add_history_at(&mut local, id, "shared", shared);

        let mut remote = fork(&local);
        remote.entry_mut(id).unwrap().history = None;
        add_history_at(&mut remote, id, "shared", shared);
        // Default cap of ten, holding one version: nothing was trimmed here.

        let report = diff(&local, &remote);

        assert!(report.conflicts.is_empty(), "no current field differs");
        assert!(
            report.local_history_ahead,
            "the remote had room, so the missing version is one it never saw"
        );
        assert!(report.has_local_contribution());
    }

    /// Database settings are merged by the fork now, so a difference in them
    /// means the merged result differs from at least one side and has to be
    /// written back. Nothing reported that, so renaming the database here
    /// stayed here.
    #[test]
    fn a_database_setting_that_differs_needs_writing_back() {
        let mut local = Database::new();
        add(&mut local, "GitHub", "secret");
        let mut remote = fork(&local);

        assert!(
            !diff(&local, &remote).structural_writeback_required,
            "identical copies need nothing"
        );

        local.meta.database_name = Some("Team vault".into());
        local.meta.database_name_changed = Some(keepass::db::Times::now());
        assert!(
            diff(&local, &remote).structural_writeback_required,
            "a rename here has to reach the other copy"
        );

        // A difference we cannot show is ours is not ours to upload: the
        // merge keeps our value on a tie, and sending it would write over
        // theirs. The two copies disagree until somebody edits one.
        remote.meta.database_name = Some("Their name".into());
        remote.meta.database_name_changed = local.meta.database_name_changed;
        assert!(
            !diff(&local, &remote).structural_writeback_required,
            "a tie is not evidence that ours is the one to send"
        );

        remote.meta.database_name = Some("Team vault".into());
        assert!(!diff(&local, &remote).structural_writeback_required);

        // View state must not: it follows the cursor, and comparing it would
        // demand an upload after every sync.
        local.meta.last_selected_group = Some(uuid::Uuid::new_v4());
        local.meta.generator = Some("SomeOtherClient".into());
        assert!(
            !diff(&local, &remote).structural_writeback_required,
            "the cursor and the writer's name are not settings"
        );
    }

    /// An entry whose expiry alone diverges in the same second was never a
    /// conflict: expiry is not in the field map, so nothing differed, and the
    /// fork then refused the whole merge with nothing for the user to answer.
    #[test]
    fn a_tied_entry_expiry_is_a_conflict_with_a_row() {
        let tied = keepass::db::Times::now();
        let mut local = Database::new();
        let id = add(&mut local, "Bank", "secret");
        local.entry_mut(id).unwrap().times.last_modification = Some(tied);

        let mut remote = fork(&local);
        {
            let mut entry = remote.entry_mut(id).unwrap();
            entry.times.expires = Some(true);
            entry.times.expiry = Some(tied + chrono::TimeDelta::days(30));
            entry.times.last_modification = Some(tied);
        }

        let report = diff(&local, &remote);
        let conflict = report
            .conflicts
            .first()
            .expect("one second, two answers about when it expires");
        assert!(
            conflict
                .fields
                .iter()
                .any(|field| field.label == "Expires" && field.local == "Never"),
            "and the row has to say what each side holds: {:?}",
            conflict.fields
        );

        let picks = Resolutions {
            entries: HashMap::from([(id.to_string(), Side::Remote)]),
            groups: HashMap::new(),
            metadata: None,
        };
        let merged = apply_picks(&local, &remote, &picks, &report).expect("resolvable");
        assert_eq!(
            merged.entry(id).expect("the entry").times.expires,
            Some(true),
            "choosing theirs has to apply theirs"
        );
    }

    /// The digest a conflict row must never carry, in the shortest form one
    /// could plausibly be shown in.
    fn byte_digest(value: &str) -> String {
        byte_digest_bytes(value.as_bytes())
    }

    fn byte_digest_bytes(data: &[u8]) -> String {
        use sha2::{Digest as _, Sha256};
        Sha256::digest(data)
            .iter()
            .take(4)
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// The rule, rather than the cases: a row that differs never shows the
    /// same text on both sides. Deciding a row on the string it renders, or
    /// rendering two different values the same way, is how a conflict screen
    /// asks a question it has not shown, and the answer then discards one
    /// side in silence.
    #[test]
    fn no_conflict_row_that_differs_reads_the_same_on_both_sides() {
        use keepass::db::{CustomDataItem, CustomDataValue};

        let tied = keepass::db::Times::now();
        let text = |value: &str| CustomDataItem {
            value: Some(CustomDataValue::String(value.into())),
            last_modification_time: None,
        };
        let blob = |bytes: Vec<u8>| CustomDataItem {
            value: Some(CustomDataValue::Binary(bytes)),
            last_modification_time: None,
        };

        // Every shape whose rendering used to lose the difference: unset
        // against empty, one tag against two that spell the same line, sets
        // of one size holding different things, dates inside a minute.
        let mut local = Database::new();
        let id = add(&mut local, "Bank", "secret");
        let group_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Banking".to_string();
            group.id()
        };
        {
            let mut entry = local.entry_mut(id).unwrap();
            entry.times.last_modification = Some(tied);
            entry.times.expires = Some(true);
            entry.times.expiry = Some(tied + chrono::TimeDelta::days(30));
            entry.tags = vec!["a, b".to_string()];
            entry.custom_data.insert("k".into(), text("one"));
            entry.set_unprotected(fields::NOTES, "");
            // Attachments are shown as a count on purpose, so this is the
            // row that has nothing but the fallback to tell the sides apart.
            entry.add_attachment("file.bin", Value::unprotected(vec![1, 2, 3]));
            // Two secrets of one length redact to one string.
            entry.set_protected(fields::PASSWORD, "aaaaaaaa");
        }
        local.group_mut(group_id).unwrap().times.last_modification = Some(tied);
        local.group_mut(group_id).unwrap().tags = vec!["x, y".to_string()];
        local
            .group_mut(group_id)
            .unwrap()
            .custom_data
            .insert("p".into(), blob(vec![1, 2, 3, 4]));
        local.group_mut(group_id).unwrap().notes = Some(String::new());

        // A name that spells what an unset one is rendered as: the last case
        // where a settings row can still read the same on both sides.
        local.meta.database_name = Some("(not set)".into());
        local.meta.database_name_changed = Some(tied);

        let mut remote = fork(&local);
        remote.meta.database_name = None;
        remote.meta.database_name_changed = Some(tied);
        {
            let mut entry = remote.entry_mut(id).unwrap();
            entry.times.expiry =
                Some(tied + chrono::TimeDelta::days(30) + chrono::TimeDelta::seconds(20));
            entry.tags = vec!["a".to_string(), "b".to_string()];
            entry.custom_data.insert("k".into(), text("two"));
            entry.add_attachment("file.bin", Value::unprotected(vec![4, 5, 6]));
            entry.set_protected(fields::PASSWORD, "bbbbbbbb");
            entry.fields.remove(fields::NOTES);
            entry.times.last_modification = Some(tied);
        }
        {
            let mut group = remote.group_mut(group_id).unwrap();
            group.tags = vec!["x".to_string(), "y".to_string()];
            group.custom_data.insert("p".into(), blob(vec![5, 6, 7, 8]));
            group.notes = None;
            group.times.last_modification = Some(tied);
        }

        let report = diff(&local, &remote);
        // Named, not counted, wherever the values can be named at all: the
        // fallback below keeps the two sides apart, but it says nothing about
        // what the user is choosing between.
        let settings = report
            .conflicts
            .iter()
            .flat_map(|conflict| &conflict.fields)
            .find(|field| field.label == "Entry settings")
            .expect("the entry's plugin data differs");
        assert!(
            settings.local.contains("\"k\""),
            "the row has to name the key: {}",
            settings.local
        );
        // Nothing on a row may be derived from the content it is refusing to
        // show: a digest of a password or an attachment, on screen, is an
        // offline oracle against exactly that.
        for row in report
            .conflicts
            .iter()
            .flat_map(|conflict| &conflict.fields)
        {
            assert!(
                !row.local.contains(&byte_digest("aaaaaaaa"))
                    && !row.remote.contains(&byte_digest("bbbbbbbb"))
                    && !row.local.contains(&byte_digest_bytes(&[1, 2, 3]))
                    && !row.remote.contains(&byte_digest_bytes(&[4, 5, 6])),
                "row {:?} carries a digest of what it hides",
                row.label
            );
        }
        let rows = report
            .conflicts
            .iter()
            .flat_map(|conflict| &conflict.fields)
            .chain(
                report
                    .group_conflicts
                    .iter()
                    .flat_map(|conflict| &conflict.fields),
            )
            .chain(
                report
                    .metadata_conflict
                    .iter()
                    .flat_map(|conflict| &conflict.fields),
            );
        let mut differing = Vec::new();
        for row in rows {
            if row.differs {
                differing.push(row.label.to_string());
                assert_ne!(
                    row.local, row.remote,
                    "row {:?} differs but reads the same on both sides",
                    row.label
                );
            }
        }
        // Named rather than counted: a row that quietly stopped firing would
        // otherwise satisfy the loop above by not being in it.
        for expected in [
            "Database name",
            "Password",
            "Notes",
            "Tags",
            "Attachments",
            "Expires",
            "Entry settings",
            "Plugin data",
        ] {
            assert!(
                differing.iter().any(|label| label == expected),
                "the fixture has to produce a {expected} row, got {differing:?}"
            );
        }
    }

    /// Every one of these rows once decided whether it existed by comparing
    /// its own rendered text, so two different values that happened to read
    /// alike dropped the row and the answer discarded one of them in silence.
    #[test]
    fn a_row_that_reads_alike_still_says_the_two_sides_differ() {
        use keepass::db::{CustomDataItem, CustomDataValue};

        // Two different blobs of the same length.
        let mut local = Database::new();
        let group_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Banking".to_string();
            group.id()
        };
        let tied = keepass::db::Times::now();
        local.group_mut(group_id).unwrap().times.last_modification = Some(tied);
        let blob = |bytes: Vec<u8>| CustomDataItem {
            value: Some(CustomDataValue::Binary(bytes)),
            last_modification_time: None,
        };
        local
            .group_mut(group_id)
            .unwrap()
            .custom_data
            .insert("Plugin".into(), blob(vec![1, 2, 3, 4]));
        let mut remote = fork(&local);
        remote
            .group_mut(group_id)
            .unwrap()
            .custom_data
            .insert("Plugin".into(), blob(vec![5, 6, 7, 8]));
        remote.group_mut(group_id).unwrap().times.last_modification = Some(tied);

        let conflict = diff(&local, &remote)
            .group_conflicts
            .into_iter()
            .next()
            .expect("two different values are a tie");
        let row = conflict
            .fields
            .iter()
            .find(|field| field.label == "Plugin data")
            .expect("and the row is there");
        assert_ne!(row.local, row.remote, "four bytes are not four other bytes");

        // Two expiry dates inside one minute.
        let mut local = Database::new();
        let id = add(&mut local, "Bank", "secret");
        let at = tied + chrono::TimeDelta::days(30);
        {
            let mut entry = local.entry_mut(id).unwrap();
            entry.times.last_modification = Some(tied);
            entry.times.expires = Some(true);
            entry.times.expiry = Some(at);
        }
        let mut remote = fork(&local);
        {
            let mut entry = remote.entry_mut(id).unwrap();
            entry.times.expiry = Some(at + chrono::TimeDelta::seconds(20));
            entry.times.last_modification = Some(tied);
        }
        let conflict = diff(&local, &remote)
            .conflicts
            .into_iter()
            .next()
            .expect("twenty seconds apart is still apart");
        let row = conflict
            .fields
            .iter()
            .find(|field| field.label == "Expires")
            .expect("and the row is there");
        assert_ne!(row.local, row.remote, "and the row has to show it");

        // Two sibling groups with one name.
        let mut local = Database::new();
        let id = add(&mut local, "Bank", "secret");
        let (here, there) = {
            let mut root = local.root_mut();
            (root.add_group().id(), root.add_group().id())
        };
        for group in [here, there] {
            local.group_mut(group).unwrap().name = "Archive".into();
        }
        let mut remote = fork(&local);
        for (db, target) in [(&mut local, here), (&mut remote, there)] {
            let mut entry = db.entry_mut(id).unwrap();
            entry.move_to(target).unwrap();
            entry.times.location_changed = Some(tied);
        }
        let conflict = diff(&local, &remote)
            .conflicts
            .into_iter()
            .next()
            .expect("a tied move");
        let row = conflict
            .fields
            .iter()
            .find(|field| field.label == "Location")
            .expect("and the row is there");
        assert_ne!(
            row.local, row.remote,
            "two groups called Archive are two places"
        );

        // One key whose name contains the separators, against two keys that
        // spell the same line. Only quoting keeps them apart.
        let mut local = Database::new();
        let group_id = local.root_mut().add_group().id();
        local.group_mut(group_id).unwrap().times.last_modification = Some(tied);
        let text = |value: &str| CustomDataItem {
            value: Some(CustomDataValue::String(value.into())),
            last_modification_time: None,
        };
        let mut remote = fork(&local);
        local
            .group_mut(group_id)
            .unwrap()
            .custom_data
            .insert("a = 1, b".into(), text(""));
        {
            let mut group = remote.group_mut(group_id).unwrap();
            group.custom_data.insert("a".into(), text("1"));
            group.custom_data.insert("b".into(), text(""));
            group.times.last_modification = Some(tied);
        }

        let conflict = diff(&local, &remote)
            .group_conflicts
            .into_iter()
            .next()
            .expect("two different maps are a tie");
        let row = conflict
            .fields
            .iter()
            .find(|field| field.label == "Plugin data")
            .expect("and the row is there");
        assert_ne!(
            row.local, row.remote,
            "one key is not two keys that spell the same line"
        );
    }

    /// A file that leaves a history limit to the format and one that writes
    /// the format's own value for it say the same thing. KeePassXC writes
    /// both out, so comparing the raw options made a round trip through it a
    /// question with nothing at stake.
    #[test]
    fn the_two_spellings_of_a_history_limit_are_not_a_question() {
        let mut local = Database::new();
        add(&mut local, "GitHub", "secret");
        local.meta.history_max_items = None;
        local.meta.history_max_size = None;
        let mut remote = fork(&local);
        remote.meta.history_max_items = Some(10);
        remote.meta.history_max_size = Some(6 * 1024 * 1024);

        assert!(
            diff(&local, &remote).metadata_conflict.is_none(),
            "the format's default is the format's default"
        );

        // A limit the user actually changed is still asked about.
        remote.meta.history_max_items = Some(25);
        let conflict = diff(&local, &remote)
            .metadata_conflict
            .expect("twenty five is not ten");
        assert!(
            conflict
                .fields
                .iter()
                .any(|field| field.label == "History entries kept"),
            "{:?}",
            conflict.fields
        );
    }

    /// An Auto-Type sequence is free text, and people put credentials in one:
    /// the token type keeps its literals out of `Debug` for that reason. A
    /// conflict row that printed the sequence put them on screen instead.
    #[test]
    fn an_auto_type_sequence_does_not_show_its_literals() {
        let mut local = Database::new();
        let group_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Banking".to_string();
            group.id()
        };
        let tied = keepass::db::Times::now();
        local.group_mut(group_id).unwrap().times.last_modification = Some(tied);
        local.group_mut(group_id).unwrap().default_autotype_sequence =
            Some("{USERNAME}{TAB}hunter2{ENTER}".into());

        let mut remote = fork(&local);
        remote
            .group_mut(group_id)
            .unwrap()
            .default_autotype_sequence = Some("{USERNAME}{TAB}swordfish{ENTER}".into());
        remote.group_mut(group_id).unwrap().times.last_modification = Some(tied);

        let conflict = diff(&local, &remote)
            .group_conflicts
            .into_iter()
            .next()
            .expect("two sequences, one second");
        let row = conflict
            .fields
            .iter()
            .find(|field| field.label == "Auto-Type sequence")
            .expect("the row is there");

        assert!(
            !row.local.contains("hunter2") && !row.remote.contains("swordfish"),
            "the literal is not shown: {row:?}"
        );
        assert!(
            row.local.contains("{USERNAME}") && row.local.contains("{TAB}"),
            "while the part that says what it does stays: {}",
            row.local
        );
        assert_ne!(row.local, row.remote, "and the two still read apart");
    }

    /// A move the clock can rank is not a question. Adding the location to the
    /// comparison must not turn every ordinary move into a prompt.
    #[test]
    fn a_ranked_entry_move_is_not_asked_about() {
        let earlier = keepass::db::Times::now() - chrono::TimeDelta::minutes(10);
        let mut local = Database::new();
        let id = add(&mut local, "Bank", "secret");
        let target = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Archive".into();
            group.id()
        };
        local.entry_mut(id).unwrap().times.location_changed = Some(earlier);

        let mut remote = fork(&local);
        {
            let mut entry = remote.entry_mut(id).unwrap();
            entry.move_to(target).unwrap();
            entry.times.location_changed = Some(keepass::db::Times::now());
        }

        let report = diff(&local, &remote);

        assert!(
            report.conflicts.is_empty(),
            "the clock ranks this move: {:?}",
            report.conflicts
        );
        let merged =
            apply_picks(&local, &remote, &Resolutions::default(), &report).expect("resolvable");
        assert_eq!(
            merged.entry(id).expect("the entry").parent().id(),
            target,
            "and the merge performs it"
        );
    }

    /// Plugin data counts as group content, so it can be the only reason a
    /// group is asked about. Without a row the screen showed an empty list
    /// and the answer discarded the other side's keys for good.
    #[test]
    fn a_group_plugin_data_conflict_says_what_differs() {
        use keepass::db::{CustomDataItem, CustomDataValue};
        let mut local = Database::new();
        let group_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Banking".to_string();
            group.id()
        };
        let tied = keepass::db::Times::now();
        local.group_mut(group_id).unwrap().times.last_modification = Some(tied);
        local.group_mut(group_id).unwrap().custom_data.insert(
            "Plugin".into(),
            CustomDataItem {
                value: Some(CustomDataValue::String("ours".into())),
                last_modification_time: None,
            },
        );

        let mut remote = fork(&local);
        remote.group_mut(group_id).unwrap().custom_data.insert(
            "Plugin".into(),
            CustomDataItem {
                value: Some(CustomDataValue::String("theirs".into())),
                last_modification_time: None,
            },
        );
        remote.group_mut(group_id).unwrap().times.last_modification = Some(tied);

        let report = diff(&local, &remote);
        let conflict = report
            .group_conflicts
            .first()
            .expect("a tied plugin value is still a tie");
        assert!(
            conflict
                .fields
                .iter()
                .any(|field| field.label == "Plugin data"
                    && field.local == "\"Plugin\" = ••• (4 chars)"
                    && field.remote == "\"Plugin\" = ••• (6 chars)"),
            "the user has to see that they differ, without being shown what \
             a plugin keeps there: {:?}",
            conflict.fields
        );
        assert!(
            !conflict
                .fields
                .iter()
                .any(|field| field.local.contains("ours") || field.remote.contains("theirs")),
            "and never the value itself"
        );
    }

    /// Where a group sits is decided on its own clock, and two moves can tie
    /// on it. The fork then kept this copy's placement and said nothing, so
    /// the move made on the other machine went back on the next upload.
    #[test]
    fn a_tied_group_move_is_asked_and_applied() {
        let mut base = Database::new();
        let (here, there, moved) = {
            let mut root = base.root_mut();
            let here = root.add_group().id();
            let there = root.add_group().id();
            let moved = root.add_group().id();
            (here, there, moved)
        };
        base.group_mut(here).unwrap().name = "Here".into();
        base.group_mut(there).unwrap().name = "There".into();
        base.group_mut(moved).unwrap().name = "Banking".into();

        let tied = keepass::db::Times::now();
        let mut local = fork(&base);
        local
            .group_mut(moved)
            .unwrap()
            .track_changes()
            .move_to(here)
            .unwrap();
        local.group_mut(moved).unwrap().times.location_changed = Some(tied);
        let mut remote = fork(&base);
        remote
            .group_mut(moved)
            .unwrap()
            .track_changes()
            .move_to(there)
            .unwrap();
        remote.group_mut(moved).unwrap().times.location_changed = Some(tied);

        let report = diff(&local, &remote);
        let conflict = report
            .group_conflicts
            .first()
            .expect("two moves, one second: the user has to choose");
        assert!(
            conflict.fields.iter().any(|field| field.label == "Location"
                && field.local == "Here"
                && field.remote == "There"),
            "and the row has to say where each side put it: {:?}",
            conflict.fields
        );

        let picks = Resolutions {
            entries: HashMap::new(),
            groups: HashMap::from([(moved.to_string(), Side::Remote)]),
            metadata: None,
        };
        let merged = apply_picks(&local, &remote, &picks, &report).expect("resolvable");
        assert_eq!(
            merged
                .group(moved)
                .expect("the group")
                .parent()
                .expect("it has a parent")
                .id(),
            there,
            "choosing theirs has to put it where they put it"
        );

        // The parent they chose can be a group only they have. The fork adds
        // those before it looks at moves, so the answer still lands.
        let mut local = fork(&base);
        local
            .group_mut(moved)
            .unwrap()
            .track_changes()
            .move_to(here)
            .unwrap();
        local.group_mut(moved).unwrap().times.location_changed = Some(tied);
        let mut remote = fork(&base);
        let theirs_only = remote.root_mut().add_group().id();
        remote.group_mut(theirs_only).unwrap().name = "Only there".into();
        remote
            .group_mut(moved)
            .unwrap()
            .track_changes()
            .move_to(theirs_only)
            .unwrap();
        remote.group_mut(moved).unwrap().times.location_changed = Some(tied);

        let report = diff(&local, &remote);
        let picks = Resolutions {
            entries: HashMap::new(),
            groups: HashMap::from([(moved.to_string(), Side::Remote)]),
            metadata: None,
        };
        let merged = apply_picks(&local, &remote, &picks, &report).expect("resolvable");
        assert_eq!(
            merged
                .group(moved)
                .expect("the group")
                .parent()
                .expect("it has a parent")
                .id(),
            theirs_only,
            "even into a group this copy had never seen"
        );
    }

    /// Choosing the remote group has to bring its picture with it.
    ///
    /// `set_icon_custom` clears the current icon and only then rejects an id
    /// this file does not hold yet, and the error was dropped: the group came
    /// out with no icon at all, and that result was uploaded. The image has
    /// to be adopted before the reference is written.
    #[test]
    fn choosing_a_remote_group_keeps_its_custom_image() {
        let mut local = Database::new();
        let group_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Banking".to_string();
            group.id()
        };
        let tied = keepass::db::Times::now();
        let ours = local
            .group_mut(group_id)
            .unwrap()
            .set_icon_custom_new(vec![0x89, b'P', b'N', b'G', 1])
            .id();
        local.group_mut(group_id).unwrap().times.last_modification = Some(tied);

        let mut remote = fork(&local);
        let theirs = remote
            .group_mut(group_id)
            .unwrap()
            .set_icon_custom_new(vec![0x89, b'P', b'N', b'G', 2])
            .id();
        remote.group_mut(group_id).unwrap().times.last_modification = Some(tied);
        assert_ne!(ours, theirs, "two different images, two different ids");

        let report = diff(&local, &remote);
        let conflict = report
            .group_conflicts
            .first()
            .expect("two images, one second: the user has to choose");
        assert!(
            conflict.fields.iter().any(|field| field.label == "Icon"),
            "and the row has to tell them apart rather than saying \
             \"Custom image\" twice: {:?}",
            conflict.fields
        );

        let picks = Resolutions {
            entries: HashMap::new(),
            groups: HashMap::from([(group_id.to_string(), Side::Remote)]),
            metadata: None,
        };
        let merged = apply_picks(&local, &remote, &picks, &report).expect("resolvable");

        assert_eq!(
            merged.group(group_id).unwrap().icon(),
            Some(&Icon::Custom(theirs)),
            "the group keeps the reference it was given"
        );
        assert_eq!(
            merged
                .custom_icon(theirs)
                .expect("and the image itself is here")
                .data,
            vec![0x89, b'P', b'N', b'G', 2]
        );
    }

    /// Two files can reach the same custom icon id for two different
    /// pictures. Taking the id at face value handed the group back the
    /// picture it already had, so keeping the remote group looked like it had
    /// been ignored, and the fork then pruned the remote image away.
    #[test]
    fn a_colliding_icon_id_still_yields_the_remote_picture() {
        let mut local = Database::new();
        let group_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Banking".to_string();
            group.id()
        };
        let tied = keepass::db::Times::now();
        let shared = local
            .group_mut(group_id)
            .unwrap()
            .set_icon_custom_new(vec![0x89, b'P', b'N', b'G', 1])
            .id();
        local.group_mut(group_id).unwrap().times.last_modification = Some(tied);

        // The same id, a different picture, and the same name on both, which
        // is what made the row compare equal and vanish.
        let mut remote = fork(&local);
        {
            let mut icon = remote.custom_icon_mut(shared).expect("the id is here");
            icon.data = vec![0x89, b'P', b'N', b'G', 2];
            icon.name = Some("Vault".into());
        }
        local.custom_icon_mut(shared).unwrap().name = Some("Vault".into());
        remote.group_mut(group_id).unwrap().notes = Some("edited there".into());
        remote.group_mut(group_id).unwrap().times.last_modification = Some(tied);

        let report = diff(&local, &remote);
        let conflict = report.group_conflicts.first().expect("a tie on one second");
        assert!(
            conflict.fields.iter().any(|field| field.label == "Icon"),
            "one name, two pictures: the row still has to appear: {:?}",
            conflict.fields
        );

        let picks = Resolutions {
            entries: HashMap::new(),
            groups: HashMap::from([(group_id.to_string(), Side::Remote)]),
            metadata: None,
        };
        let merged = apply_picks(&local, &remote, &picks, &report).expect("resolvable");

        let Some(Icon::Custom(shown)) = merged.group(group_id).unwrap().icon().cloned() else {
            panic!("the group still has a custom icon");
        };
        assert_eq!(
            merged.custom_icon(shown).expect("its image is here").data,
            vec![0x89, b'P', b'N', b'G', 2],
            "keeping their group has to show their picture"
        );
    }

    /// A bin the other copy knows and no longer designates is not the same
    /// thing as an independently created one, and nothing in either file says
    /// which it is: the other copy may have retired that group and be using
    /// it as an ordinary archive, or the two bins may have met in an earlier
    /// merge. Nesting it deletes a live archive, leaving it resurrects
    /// deletions, so it is asked. Answering for the copy that retired it
    /// leaves the archive alone.
    #[test]
    fn a_bin_the_other_copy_no_longer_designates_is_asked_about() {
        let mut base = Database::new();
        let archived = add(&mut base, "Still needed", "secret");
        let start = keepass::db::Times::now() - chrono::TimeDelta::minutes(30);
        let old_bin = {
            let mut root = base.root_mut();
            let mut group = root.add_group();
            group.name = "Recycle Bin".to_string();
            group.id()
        };
        base.meta.recyclebin_uuid = Some(old_bin.uuid());
        base.meta.recyclebin_enabled = Some(true);
        base.meta.recyclebin_changed = Some(start);
        {
            let mut entry = base.entry_mut(archived).unwrap();
            entry.move_to(old_bin).unwrap();
            entry.times.last_modification = Some(start);
            entry.times.location_changed = Some(start);
        }

        // This copy still calls that group the bin. The other made a new one
        // and kept the old group as an ordinary archive.
        let local = fork(&base);
        let mut remote = fork(&base);
        let new_bin = {
            let mut root = remote.root_mut();
            let mut group = root.add_group();
            group.name = "Recycle Bin".to_string();
            group.id()
        };
        remote.meta.recyclebin_uuid = Some(new_bin.uuid());
        remote.meta.recyclebin_changed = Some(keepass::db::Times::now());
        remote.group_mut(old_bin).unwrap().name = "Archive".into();
        remote.group_mut(old_bin).unwrap().times.last_modification =
            Some(keepass::db::Times::now());

        let report = diff(&local, &remote);
        let conflict = report
            .metadata_conflict
            .as_ref()
            .expect("neither file can say whether that group is still a bin");
        assert!(
            conflict
                .fields
                .iter()
                .any(|field| field.label == "Recycle bin" && field.local != field.remote),
            "and the row has to name both groups: {:?}",
            conflict.fields
        );

        let picks = Resolutions {
            entries: HashMap::new(),
            groups: HashMap::new(),
            metadata: Some(Side::Remote),
        };
        let merged = apply_picks(&local, &remote, &picks, &report)
            .expect("a retired bin is not a reason to refuse the merge");

        assert_eq!(
            merged
                .group(old_bin)
                .expect("the archive is still there")
                .parent()
                .expect("it has a parent")
                .id(),
            merged.root().id(),
            "the group they retired stays where they put it"
        );
        let live: Vec<String> = live_entries(&merged)
            .into_values()
            .map(|snapshot| snapshot.view.title)
            .collect();
        assert_eq!(
            live,
            vec!["Still needed".to_string()],
            "and what they kept in it is not deleted behind their back"
        );

        // With nothing in that group there is nothing at stake either way, so
        // the clock decides and the user is not stopped for it.
        let mut empty_local = fork(&local);
        let mut empty_remote = fork(&remote);
        for db in [&mut empty_local, &mut empty_remote] {
            let entries: Vec<EntryId> = db.iter_all_entries().map(|entry| entry.id()).collect();
            for id in entries {
                db.entry_mut(id).expect("entry").remove();
            }
        }
        assert!(
            diff(&empty_local, &empty_remote)
                .metadata_conflict
                .is_none(),
            "an empty group is not worth a prompt"
        );
    }

    /// After an earlier reunion the two bins are nested, and a copy that has
    /// not caught up still designates the inner one. Both copies call its
    /// contents deleted, so there is nothing to decide and nothing to stop
    /// the user for.
    #[test]
    fn a_bin_already_inside_the_other_one_is_not_asked_about() {
        let start = keepass::db::Times::now() - chrono::TimeDelta::minutes(30);
        let mut base = Database::new();
        let trashed = add(&mut base, "Deleted", "secret");
        let outer = {
            let mut root = base.root_mut();
            let mut group = root.add_group();
            group.name = "Recycle Bin".to_string();
            group.id()
        };
        let inner = {
            let mut root = base.root_mut();
            let mut group = root.add_group();
            group.name = "Recycle Bin".to_string();
            group.id()
        };
        base.meta.recyclebin_enabled = Some(true);
        {
            let mut entry = base.entry_mut(trashed).unwrap();
            entry.move_to(inner).unwrap();
            entry.times.last_modification = Some(start);
            entry.times.location_changed = Some(start);
        }

        // The copy that has not caught up: the inner group is still its bin,
        // and it sits at the root.
        let mut local = fork(&base);
        local.meta.recyclebin_uuid = Some(inner.uuid());
        local.meta.recyclebin_changed = Some(start);

        // The copy an earlier reunion already touched.
        let mut remote = fork(&base);
        remote.meta.recyclebin_uuid = Some(outer.uuid());
        remote.meta.recyclebin_changed = Some(keepass::db::Times::now());
        {
            let mut group = remote.group_mut(inner).unwrap();
            group.track_changes().move_to(outer).unwrap();
            group.times.location_changed = Some(start + chrono::TimeDelta::minutes(10));
        }

        assert!(
            diff(&local, &remote).metadata_conflict.is_none(),
            "both copies already call those entries deleted"
        );
    }

    /// Once one bin sits inside the other, a third copy can designate the
    /// outer one. The move that would repair that is a cycle, and giving up
    /// on it left the outer group holding entries that stopped counting as
    /// deleted the moment it was no longer the bin.
    #[test]
    fn a_bin_nested_the_other_way_round_is_lifted_out_rather_than_skipped() {
        let start = keepass::db::Times::now() - chrono::TimeDelta::minutes(30);
        let mut base = Database::new();
        let trashed = add(&mut base, "Deleted", "secret");
        // The other copy's bin, which both sides know.
        let inner = {
            let mut root = base.root_mut();
            let mut group = root.add_group();
            group.name = "Recycle Bin".to_string();
            group.times.location_changed = Some(start);
            group.id()
        };
        base.entry_mut(trashed).unwrap().times.last_modification = Some(start);
        base.entry_mut(trashed).unwrap().times.location_changed = Some(start);
        base.meta.recyclebin_enabled = Some(true);

        // This copy is the state a previous merge left behind: its own bin,
        // with the other copy's nested inside it.
        let mut local = fork(&base);
        let outer = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Recycle Bin".to_string();
            group.id()
        };
        local.meta.recyclebin_uuid = Some(outer.uuid());
        local.meta.recyclebin_changed = Some(start);
        let nested = start + chrono::TimeDelta::minutes(10);
        for (group, entry) in [(Some(inner), None), (None, Some(trashed))] {
            if let Some(group) = group {
                let mut group = local.group_mut(group).unwrap();
                group.track_changes().move_to(outer).unwrap();
                group.times.location_changed = Some(nested);
            }
            if let Some(entry) = entry {
                let mut entry = local.entry_mut(entry).unwrap();
                entry.move_to(outer).unwrap();
                entry.times.location_changed = Some(nested);
                // A real delete dates the entry too, which is what lets the
                // other side rank it rather than calling it a tie.
                entry.times.last_modification = Some(nested);
            }
        }

        // The other copy has never seen the outer group and has just made its
        // own the bin again.
        let mut remote = fork(&base);
        remote.meta.recyclebin_uuid = Some(inner.uuid());
        remote.meta.recyclebin_changed = Some(keepass::db::Times::now());

        let report = diff(&local, &remote);
        let merged =
            apply_picks(&local, &remote, &Resolutions::default(), &report).expect("resolvable");

        assert_eq!(
            merged.meta.recyclebin_uuid,
            Some(inner.uuid()),
            "the newer clock decides which group is the bin"
        );
        assert!(
            crate::keepass::document::group_is_within(
                &merged,
                group_with_uuid(&merged, outer.uuid()).expect("the outer group survives"),
                group_with_uuid(&merged, inner.uuid()).expect("and so does the bin"),
            ),
            "the bin was lifted out and now holds the other group"
        );
        let live: Vec<String> = live_entries(&merged)
            .into_values()
            .map(|snapshot| snapshot.view.title)
            .collect();
        assert!(
            live.is_empty(),
            "and what was thrown away stays thrown away: {live:?}"
        );
    }

    /// A settings clock nobody could have written yet decides nothing.
    ///
    /// Last-write-wins on the metadata block is a number inside the shared
    /// file, so anyone who can write it could stamp their database name with
    /// the year 2099 and have it replace everyone else's. It also reversed a
    /// decision already taken: the resolution is stamped with a time we
    /// believe, which the claimed one still outranks.
    #[test]
    fn a_future_dated_settings_clock_decides_nothing() {
        let mut local = Database::new();
        add(&mut local, "GitHub", "secret");
        local.meta.database_name = Some("Team vault".into());
        local.meta.database_name_changed = Some(keepass::db::Times::now());

        let mut remote = fork(&local);
        remote.meta.database_name = Some("Their vault".into());
        remote.meta.database_name_changed =
            Some(keepass::db::Times::now() + chrono::TimeDelta::days(365));

        let report = diff(&local, &remote);
        assert!(
            report.metadata_conflict.is_some(),
            "a claim about the future is not evidence, so it is asked"
        );

        let picks = Resolutions {
            entries: HashMap::new(),
            groups: HashMap::new(),
            metadata: Some(Side::Local),
        };
        let merged = apply_picks(&local, &remote, &picks, &report).expect("resolvable");
        assert_eq!(merged.meta.database_name.as_deref(), Some("Team vault"));

        // A third copy still holding the claimed date meets the decision as
        // an open question, not as a settled loss.
        assert!(
            diff(&merged, &remote).metadata_conflict.is_some(),
            "the future date must not quietly take the name back"
        );
    }

    /// Two copies can each have made their own recycle bin.
    ///
    /// The merge keeps both groups and can designate only one, and everything
    /// deleted into the other became live again: an ordinary group holding
    /// entries the user had thrown away, listed next to the ones they kept.
    #[test]
    fn two_recycle_bins_keep_both_sides_deletions() {
        // One shared starting point, then each side makes its own bin: the
        // shape two clients produce when each deletes something before they
        // ever meet.
        fn make_bin(db: &mut Database, trashed: EntryId, changed: NaiveDateTime) {
            let bin_id = {
                let mut root = db.root_mut();
                let mut bin = root.add_group();
                bin.name = "Recycle Bin".to_string();
                bin.id()
            };
            db.meta.recyclebin_uuid = Some(bin_id.uuid());
            db.meta.recyclebin_enabled = Some(true);
            db.meta.recyclebin_changed = Some(changed);
            let mut entry = db.entry_mut(trashed).unwrap();
            entry.move_to(bin_id).unwrap();
            // A real delete dates the move and the entry, which is what lets
            // the other side rank it rather than calling it a tie.
            entry.times.location_changed = Some(changed);
            entry.times.last_modification = Some(changed);
        }

        let mut base = Database::new();
        add(&mut base, "Kept", "secret");
        let here = add(&mut base, "Deleted here", "secret");
        let there = add(&mut base, "Deleted there", "secret");
        // Both deletes have to be strictly newer than the shared starting
        // point, or the untouched copy ties with the moved one.
        let ids: Vec<EntryId> = base.iter_all_entries().map(|entry| entry.id()).collect();
        let start = keepass::db::Times::now() - chrono::TimeDelta::minutes(30);
        for id in ids {
            let mut entry = base.entry_mut(id).unwrap();
            entry.times.last_modification = Some(start);
            entry.times.location_changed = Some(start);
        }

        let mut local = fork(&base);
        make_bin(
            &mut local,
            here,
            keepass::db::Times::now() - chrono::TimeDelta::minutes(5),
        );
        let mut remote = fork(&base);
        make_bin(&mut remote, there, keepass::db::Times::now());
        assert_ne!(
            local.meta.recyclebin_uuid, remote.meta.recyclebin_uuid,
            "two bins, made independently"
        );

        let report = diff(&local, &remote);
        let merged = apply_picks(&local, &remote, &Resolutions::default(), &report)
            .expect("different bins are not a reason to refuse the merge");

        let live: Vec<String> = live_entries(&merged)
            .into_values()
            .map(|snapshot| snapshot.view.title)
            .collect();
        assert!(
            !live.iter().any(|title| title.starts_with("Deleted")),
            "a deletion on either side stays a deletion: {live:?}"
        );
        assert_eq!(
            merged.iter_all_entries().count(),
            3,
            "and nothing is dropped either"
        );
        // The losing bin is kept as a group rather than emptied into the
        // other, so the user can still see where each deletion came from and
        // restore from either.
        assert_eq!(merged.iter_all_groups().count(), 3, "root plus both bins");
    }

    /// A settings difference no clock can rank is not ours to keep quietly.
    ///
    /// The tie alone raised nothing, so any other local contribution carried
    /// the merge to a silent upload and our retained database name went over
    /// theirs. It has to be asked, like a tied entry or a tied group.
    #[test]
    fn a_tied_database_name_is_a_conflict() {
        let mut local = Database::new();
        add(&mut local, "GitHub", "secret");
        let tied = keepass::db::Times::now();
        local.meta.database_name = Some("Team vault".into());
        local.meta.database_name_changed = Some(tied);

        let mut remote = fork(&local);
        remote.meta.database_name = Some("Their vault".into());
        remote.meta.database_name_changed = Some(tied);
        // The independent contribution that used to carry the silent upload.
        add(&mut local, "Bank", "secret");

        let report = diff(&local, &remote);

        let conflict = report
            .metadata_conflict
            .as_ref()
            .expect("a tied database name has to reach the user");
        assert!(
            conflict.fields.iter().any(|f| f.label == "Database name"
                && f.local == "Team vault"
                && f.remote == "Their vault"),
            "and the row has to say what each side holds: {:?}",
            conflict.fields
        );
        assert!(
            report.needs_a_decision(),
            "so the silent merge cannot apply the default"
        );
    }

    /// The gate has to ask the clock of the field that differs.
    ///
    /// Any newer metadata timestamp used to count as evidence for every
    /// metadata difference, so editing the history limit here made our tied
    /// database name look like ours to send.
    #[test]
    fn an_unrelated_local_edit_does_not_decide_a_tied_name() {
        let mut local = Database::new();
        add(&mut local, "GitHub", "secret");
        let tied = keepass::db::Times::now();
        local.meta.database_name = Some("Team vault".into());
        local.meta.database_name_changed = Some(tied);

        let mut remote = fork(&local);
        remote.meta.database_name = Some("Their vault".into());
        remote.meta.database_name_changed = Some(tied);

        // A settings-block edit that is genuinely ours, on its own clock.
        local.meta.history_max_items = Some(20);
        local.meta.settings_changed = Some(tied + chrono::TimeDelta::minutes(5));

        let report = diff(&local, &remote);

        assert!(
            report.structural_writeback_required,
            "the history limit is ours and has to reach them"
        );
        let conflict = report
            .metadata_conflict
            .as_ref()
            .expect("the name is still tied and still has to be asked");
        assert!(
            conflict.fields.iter().all(|f| f.label == "Database name"),
            "and only the tied field is asked: {:?}",
            conflict.fields
        );
    }

    /// Keeping their settings must keep only the ones nobody could rank.
    ///
    /// Copying the whole metadata block was the first attempt and reverted
    /// the description this copy demonstrably edited last, which is the loss
    /// the screen exists to prevent.
    #[test]
    fn choosing_their_settings_leaves_the_ranked_fields_alone() {
        let mut local = Database::new();
        add(&mut local, "GitHub", "secret");
        let tied = keepass::db::Times::now();
        local.meta.database_name = Some("Team vault".into());
        local.meta.database_name_changed = Some(tied);
        local.meta.database_description = Some("Shared credentials".into());
        local.meta.database_description_changed = Some(tied);

        let mut remote = fork(&local);
        remote.meta.database_name = Some("Their vault".into());
        remote.meta.database_name_changed = Some(tied);
        remote.meta.database_description = Some("Stale text".into());
        remote.meta.database_description_changed = Some(tied - chrono::TimeDelta::minutes(5));

        let report = diff(&local, &remote);
        let picks = Resolutions {
            entries: HashMap::new(),
            groups: HashMap::new(),
            metadata: Some(Side::Remote),
        };
        let merged = apply_picks(&local, &remote, &picks, &report).expect("resolvable");

        assert_eq!(
            merged.meta.database_name.as_deref(),
            Some("Their vault"),
            "the tied field follows the choice"
        );
        assert_eq!(
            merged.meta.database_description.as_deref(),
            Some("Shared credentials"),
            "the field we changed last is not part of that choice"
        );
    }

    /// The choice has to survive the merge that follows it and the next merge
    /// against a copy that still holds the other value. Without a stamp the
    /// fork ranked the two the same way it did before, and re-asked forever.
    #[test]
    fn a_settings_choice_outranks_both_sides() {
        let mut local = Database::new();
        add(&mut local, "GitHub", "secret");
        let tied = keepass::db::Times::now();
        local.meta.database_name = Some("Team vault".into());
        local.meta.database_name_changed = Some(tied);

        let mut remote = fork(&local);
        remote.meta.database_name = Some("Their vault".into());
        remote.meta.database_name_changed = Some(tied);

        let report = diff(&local, &remote);
        let picks = Resolutions {
            entries: HashMap::new(),
            groups: HashMap::new(),
            metadata: Some(Side::Local),
        };
        let merged = apply_picks(&local, &remote, &picks, &report).expect("resolvable");

        assert_eq!(
            merged.meta.database_name.as_deref(),
            Some("Team vault"),
            "the merge that runs after the choice must not undo it"
        );
        // A third copy that never saw the decision meets it as settled.
        let third = diff(&merged, &remote);
        assert!(
            third.metadata_conflict.is_none(),
            "the decision is not re-asked"
        );
        assert!(
            merged.meta.database_name_changed > Some(tied),
            "because it now outranks the timestamp it was tied with"
        );
    }

    /// Custom data is decided per key, on that key's own modification time.
    /// A key only one side carries is a union, not a disagreement; a tied
    /// value is a disagreement nobody can rank.
    #[test]
    fn plugin_data_is_asked_per_key_and_only_when_tied() {
        use keepass::db::{CustomDataItem, CustomDataValue};
        let item = |value: &str, at: Option<NaiveDateTime>| CustomDataItem {
            value: Some(CustomDataValue::String(value.into())),
            last_modification_time: at,
        };
        let tied = keepass::db::Times::now();

        let mut local = Database::new();
        add(&mut local, "GitHub", "secret");
        local
            .meta
            .custom_data
            .insert("Browser".into(), item("ours", Some(tied)));
        let mut remote = fork(&local);
        remote
            .meta
            .custom_data
            .insert("Browser".into(), item("theirs", Some(tied)));
        // A key only this copy has, and undated, which is how older clients
        // wrote them. The merge unions it, so nothing is lost and nothing is
        // asked, whatever the clocks say.
        local
            .meta
            .custom_data
            .insert("OnlyHere".into(), item("kept", None));

        let report = diff(&local, &remote);

        let conflict = report
            .metadata_conflict
            .as_ref()
            .expect("a tied plugin value has to be asked");
        assert_eq!(
            conflict
                .fields
                .iter()
                .map(|f| f.label.as_ref())
                .collect::<Vec<_>>(),
            vec!["Plugin data \"Browser\""],
            "only the tied key, and named so the user can tell which"
        );
        assert!(
            report.structural_writeback_required,
            "the key only we have still has to reach them"
        );

        // And the choice applies to that key alone.
        let picks = Resolutions {
            entries: HashMap::new(),
            groups: HashMap::new(),
            metadata: Some(Side::Remote),
        };
        let merged = apply_picks(&local, &remote, &picks, &report).expect("resolvable");
        assert_eq!(
            merged
                .meta
                .custom_data
                .get("Browser")
                .and_then(|i| i.value.clone()),
            Some(CustomDataValue::String("theirs".into()))
        );
        assert!(
            merged.meta.custom_data.contains_key("OnlyHere"),
            "the union is untouched by the choice"
        );
    }

    /// A setting the other copy changed last is theirs to send, not ours.
    /// Reporting every difference as ours meant a pull that only adopted
    /// their value still uploaded, on every tick.
    #[test]
    fn a_setting_they_changed_last_is_not_our_upload() {
        let mut local = Database::new();
        add(&mut local, "GitHub", "secret");
        let earlier = keepass::db::Times::now() - chrono::TimeDelta::minutes(5);
        local.meta.database_name = Some("Old name".into());
        local.meta.database_name_changed = Some(earlier);

        let mut remote = fork(&local);
        remote.meta.database_name = Some("New name".into());
        remote.meta.database_name_changed = Some(keepass::db::Times::now());

        let report = diff(&local, &remote);

        assert!(report.metadata_conflict.is_none(), "the clock ranks this");
        assert!(
            !report.structural_writeback_required,
            "adopting their name is a pull, not something to send back"
        );
    }

    /// Expiry lives in `times`, not in the field map, so the field diff never
    /// saw it. An entry whose only change was its expiry date read as no
    /// contribution at all: it was never uploaded, and the next edit from the
    /// other side erased it.
    #[test]
    fn an_expiry_only_edit_is_a_local_contribution() {
        let mut local = Database::new();
        let id = add(&mut local, "Bank", "secret");
        let group_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Live".into();
            group.id()
        };
        let remote = fork(&local);

        assert!(!diff(&local, &remote).has_local_contribution());

        local.entry_mut(id).unwrap().times.expiry =
            Some(keepass::db::Times::now() + chrono::TimeDelta::days(30));
        local.entry_mut(id).unwrap().times.expires = Some(true);
        assert!(
            diff(&local, &remote).has_local_contribution(),
            "an expiry set here has to reach the other copy"
        );

        let mut local = fork(&remote);
        local.group_mut(group_id).unwrap().times.expires = Some(true);
        local.group_mut(group_id).unwrap().times.expiry =
            Some(keepass::db::Times::now() + chrono::TimeDelta::days(30));
        assert!(
            diff(&local, &remote).has_local_contribution(),
            "and so does one set on a group"
        );
    }

    /// KeePassXC writes an explicit far-future date on objects it marks as
    /// never expiring, without touching their modification time, while this
    /// app leaves the field empty. Comparing the raw pair made every sync
    /// after a KeePassXC save look like a local change, forever. A date that
    /// is not in force says nothing.
    #[test]
    fn a_date_on_something_that_never_expires_is_not_a_change() {
        let mut local = Database::new();
        let id = add(&mut local, "Bank", "secret");
        let group_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Live".into();
            group.id()
        };
        let mut remote = fork(&local);

        // What the other client leaves behind on a non-expiring object.
        let far_future = keepass::db::Times::now() + chrono::TimeDelta::days(36_500);
        {
            let mut entry = remote.entry_mut(id).unwrap();
            entry.times.expiry = Some(far_future);
            entry.times.expires = Some(false);
        }
        {
            let mut group = remote.group_mut(group_id).unwrap();
            group.times.expiry = Some(far_future);
            group.times.expires = Some(false);
        }

        assert!(
            !diff(&local, &remote).has_local_contribution(),
            "a date that is not in force is not somebody's edit"
        );

        // A date that is in force still is.
        remote.entry_mut(id).unwrap().times.expires = Some(true);
        assert!(diff(&local, &remote).structural_writeback_required);
    }

    /// KeePassXC rewrites two custom data keys on every save and treats both
    /// as generated. Comparing them made every sync after a KeePassXC save
    /// look like a local change and ask for an upload, forever.
    #[test]
    fn the_keys_other_clients_regenerate_are_not_a_local_change() {
        use keepass::db::{CustomDataItem, CustomDataValue};
        let mut local = Database::new();
        add(&mut local, "Bank", "secret");
        let mut remote = fork(&local);

        for (db, slug) in [(&mut local, "ours"), (&mut remote, "theirs")] {
            db.meta.custom_data.insert(
                "KPXC_RANDOM_SLUG".into(),
                CustomDataItem {
                    value: Some(CustomDataValue::String(slug.into())),
                    last_modification_time: None,
                },
            );
            db.meta.custom_data.insert(
                "_LAST_MODIFIED".into(),
                CustomDataItem {
                    value: Some(CustomDataValue::String(slug.into())),
                    last_modification_time: None,
                },
            );
        }

        assert!(
            !diff(&local, &remote).structural_writeback_required,
            "a value the other client regenerates on every save is not our edit"
        );

        // A key someone actually set still counts. The merge unions custom
        // data, so a key only this copy carries is one only this copy can
        // send, whatever the clocks say.
        local.meta.custom_data.insert(
            "Plugin".into(),
            CustomDataItem {
                value: Some(CustomDataValue::String("configured".into())),
                last_modification_time: None,
            },
        );
        assert!(diff(&local, &remote).structural_writeback_required);
    }

    /// The outer configuration has no change time, so there is no evidence
    /// for whose version is newer. Adopting the other copy's was tried and
    /// silently re-encrypted the vault with whatever parameters that file
    /// carried, which can be weaker than the ones chosen here. Ours stays.
    #[test]
    fn our_own_file_settings_survive_a_merge() {
        let mut local = Database::new();
        let id = add(&mut local, "Bank", "secret");
        local.config.compression_config = keepass::config::CompressionConfig::None;
        let mut remote = fork(&local);
        remote.config.compression_config = keepass::config::CompressionConfig::GZip;
        remote
            .entry_mut(id)
            .unwrap()
            .set_unprotected(fields::NOTES, "edited remotely");
        remote.entry_mut(id).unwrap().times.last_modification =
            Some(keepass::db::Times::now() + chrono::TimeDelta::minutes(1));

        let report = diff(&local, &remote);
        let merged =
            apply_picks(&local, &remote, &Resolutions::default(), &report).expect("resolvable");

        assert_eq!(
            merged.config.compression_config,
            keepass::config::CompressionConfig::None,
            "a sync must not re-encrypt this vault with someone else's parameters"
        );
        assert_eq!(
            merged.entry(id).unwrap().get(fields::NOTES),
            Some("edited remotely"),
            "while their entry edit still lands"
        );
        merged
            .save(&mut std::io::Cursor::new(Vec::new()), test_key())
            .expect("and the result is writable");
    }

    fn test_key() -> keepass::DatabaseKey {
        keepass::DatabaseKey::new().with_password("pw")
    }

    /// The fork gives a version with no timestamp the epoch and unions it
    /// like any other. Skipping those here made this report disagree with the
    /// merge that follows it: the merge added a version the report said was
    /// not there, and a caller that trusted the report wrote without it.
    #[test]
    fn a_version_without_a_timestamp_still_counts() {
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "secret");
        let mut remote = fork(&local);

        // A version some other client wrote without a modification time.
        let mut version = remote.entry(id).expect("entry").deref().clone();
        version.set_unprotected(fields::NOTES, "no timestamp");
        version.times.last_modification = None;
        version.history = None;
        remote
            .entry_mut(id)
            .expect("entry")
            .history
            .get_or_insert_default()
            .add_entry(version);

        let report = diff(&local, &remote);

        assert!(
            report.remote_history_ahead,
            "the merge will add it, so the report has to say so"
        );
    }

    /// A vault set to keep no history at all trimmed everything by
    /// definition, so nothing this copy holds is evidence the other side
    /// never saw it. Reading an empty history as "they had room" asked for an
    /// upload on every sync.
    #[test]
    fn a_remote_that_keeps_no_history_is_not_behind() {
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "secret");
        add_history_at(
            &mut local,
            id,
            "kept here",
            keepass::db::Times::now() - chrono::TimeDelta::minutes(10),
        );

        // Set on both sides: a differing cap is itself a setting the merge
        // has to write back, which would mask what this test is about.
        local.meta.history_max_items = Some(0);
        let mut remote = fork(&local);
        remote.entry_mut(id).unwrap().history = None;

        let report = diff(&local, &remote);

        assert!(
            !report.local_history_ahead,
            "a vault that keeps nothing dropped it, so there is nothing to send"
        );
        assert!(!report.has_local_contribution());
    }

    /// Two different versions written in the same second are two versions.
    /// Identifying them by timestamp alone collapsed them into one, and then
    /// neither side counted as ahead of the other.
    #[test]
    fn two_versions_sharing_a_timestamp_are_not_one_version() {
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "secret");
        let same_second = keepass::db::Times::now() - chrono::TimeDelta::minutes(10);
        add_history_at(&mut local, id, "written here", same_second);

        let mut remote = fork(&local);
        remote.entry_mut(id).unwrap().history = None;
        add_history_at(&mut remote, id, "written there", same_second);

        let report = diff(&local, &remote);

        assert!(report.local_history_ahead, "our version is not theirs");
        assert!(report.remote_history_ahead, "and theirs is not ours");
    }

    /// Editing an entry and undoing the edit leaves the current fields exactly
    /// as the remote has them, and the intermediate version only here. With
    /// history out of the comparison entirely, the report said this copy
    /// contributed nothing and that version was never uploaded.
    #[test]
    fn a_version_the_remote_never_saw_is_uploaded() {
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "secret");
        let shared = keepass::db::Times::now() - chrono::TimeDelta::minutes(30);
        add_history_at(&mut local, id, "shared", shared);

        let remote = fork(&local);

        // Edited here after the last sync, then reverted: same current fields,
        // one more version.
        add_history_at(
            &mut local,
            id,
            "typo",
            keepass::db::Times::now() - chrono::TimeDelta::minutes(2),
        );

        let report = diff(&local, &remote);

        assert!(report.conflicts.is_empty(), "no current field differs");
        assert!(report.local_history_ahead);
        assert!(
            report.has_local_contribution(),
            "the version exists only here, so it has to go up"
        );
        assert!(
            !report.remote_history_ahead,
            "nothing to write back the other way"
        );
    }

    /// The mirror case decides whether the CLI writes the merged database to
    /// disk. Without it, versions pulled from the remote were merged and then
    /// dropped.
    #[test]
    fn a_version_only_the_remote_holds_forces_a_local_save() {
        let mut local = Database::new();
        let id = add(&mut local, "GitHub", "secret");
        let shared = keepass::db::Times::now() - chrono::TimeDelta::minutes(30);
        add_history_at(&mut local, id, "shared", shared);

        let mut remote = fork(&local);
        add_history_at(
            &mut remote,
            id,
            "their-edit",
            keepass::db::Times::now() - chrono::TimeDelta::minutes(2),
        );

        let report = diff(&local, &remote);

        assert!(report.remote_history_ahead);
        assert!(!report.local_history_ahead);
        assert!(
            !report.has_local_contribution(),
            "pulling their version is still a fast-forward"
        );
    }

    /// The silent auto-merge path runs when nothing needs deciding, and it
    /// applies the default side. Letting a group conflict through there
    /// would upload that default over the other machine's version with no
    /// overlay and no log line.
    #[test]
    fn a_group_conflict_alone_still_needs_a_decision() {
        let mut local = Database::new();
        let group_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Banking".to_string();
            group.id()
        };
        let tied = keepass::db::Times::now();
        local.group_mut(group_id).unwrap().times.last_modification = Some(tied);
        let mut remote = fork(&local);
        remote.group_mut(group_id).unwrap().name = "Finance".to_string();
        remote.group_mut(group_id).unwrap().times.last_modification = Some(tied);

        let report = diff(&local, &remote);

        assert!(report.conflicts.is_empty(), "no entry diverged");
        assert!(
            report.needs_a_decision(),
            "and the group still has to reach the user"
        );
    }

    /// A group renamed on both machines in the same second has no timestamp
    /// to decide by. The merge used to keep the local name and bump its
    /// timestamp, so the next upload deleted the other user's rename with
    /// only a line in a session-scoped log to show for it.
    #[test]
    fn a_tied_group_content_divergence_is_a_conflict_the_user_decides() {
        let mut local = Database::new();
        let group_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Banking".to_string();
            group.id()
        };
        let tied = keepass::db::Times::now();
        local.group_mut(group_id).unwrap().times.last_modification = Some(tied);

        let mut remote = fork(&local);
        remote.group_mut(group_id).unwrap().name = "Finance".to_string();
        remote.group_mut(group_id).unwrap().times.last_modification = Some(tied);

        let report = diff(&local, &remote);
        assert_eq!(report.group_conflicts.len(), 1);
        let conflict = &report.group_conflicts[0];
        assert_eq!(conflict.name, "Banking");
        assert!(
            conflict.fields.iter().any(|field| field.label == "Name"
                && field.local == "Banking"
                && field.remote == "Finance"),
            "the row shows what each side holds: {:?}",
            conflict.fields
        );

        // Keeping the remote name applies it, and the result outranks both.
        let picks = Resolutions {
            entries: HashMap::new(),
            groups: HashMap::from([(conflict.id.clone(), Side::Remote)]),
            metadata: None,
        };
        let merged = apply_picks(&local, &remote, &picks, &report).expect("resolvable");
        assert_eq!(merged.group(group_id).expect("group").name, "Finance");
        assert!(
            merged
                .group(group_id)
                .and_then(|group| group.times.last_modification)
                .is_some_and(|at| at > tied),
            "the resolution has to outrank the versions it replaces"
        );
    }

    /// The default side is Local, matching the entry rows, and it has to
    /// actually hold against the fork's timestamp-ranked group merge.
    #[test]
    fn an_unanswered_group_conflict_keeps_the_local_side() {
        let mut local = Database::new();
        let group_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Banking".to_string();
            group.id()
        };
        let tied = keepass::db::Times::now();
        local.group_mut(group_id).unwrap().times.last_modification = Some(tied);
        let mut remote = fork(&local);
        remote.group_mut(group_id).unwrap().name = "Finance".to_string();
        remote.group_mut(group_id).unwrap().times.last_modification = Some(tied);

        let report = diff(&local, &remote);
        let merged =
            apply_picks(&local, &remote, &Resolutions::default(), &report).expect("resolvable");

        assert_eq!(merged.group(group_id).expect("group").name, "Banking");
    }

    /// Group content has no last-write-wins path of its own: the fork ranks
    /// groups by timestamp, so anyone who can write the shared file could
    /// stamp a rename with the year 2099 and have it replace everyone
    /// else's, with nothing shown to anyone.
    #[test]
    fn a_group_timestamp_from_the_future_asks_instead_of_winning() {
        let mut local = Database::new();
        let group_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Banking".to_string();
            group.id()
        };
        local.group_mut(group_id).unwrap().times.last_modification =
            Some(keepass::db::Times::now() - chrono::TimeDelta::hours(1));

        let mut remote = fork(&local);
        remote.group_mut(group_id).unwrap().name = "Attacker".to_string();
        remote.group_mut(group_id).unwrap().times.last_modification =
            Some(keepass::db::Times::now() + chrono::TimeDelta::days(365));

        let report = diff(&local, &remote);
        assert_eq!(
            report.group_conflicts.len(),
            1,
            "an unbelievable timestamp must not decide a group either"
        );

        let merged =
            apply_picks(&local, &remote, &Resolutions::default(), &report).expect("resolvable");
        assert_eq!(
            merged.group(group_id).expect("group").name,
            "Banking",
            "and the default keeps this machine's version"
        );
        assert!(
            merged
                .group(group_id)
                .and_then(|group| group.times.last_modification)
                .is_some_and(|at| at <= keepass::db::Times::now() + MAX_CLOCK_SKEW),
            "without adopting the timestamp it refused as evidence"
        );
    }

    /// A group whose timestamps rank cleanly is the fork's business, not
    /// ours. Prompting for those would make every ordinary rename a
    /// question.
    #[test]
    fn a_group_with_a_newer_side_is_left_to_the_fork() {
        let mut local = Database::new();
        let group_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Banking".to_string();
            group.id()
        };
        local.group_mut(group_id).unwrap().times.last_modification =
            Some(keepass::db::Times::now() - chrono::TimeDelta::hours(1));

        let mut remote = fork(&local);
        remote.group_mut(group_id).unwrap().name = "Finance".to_string();
        remote.group_mut(group_id).unwrap().times.last_modification =
            Some(keepass::db::Times::now() - chrono::TimeDelta::minutes(1));

        let report = diff(&local, &remote);

        assert!(report.group_conflicts.is_empty(), "the newer side decides");
        let merged =
            apply_picks(&local, &remote, &Resolutions::default(), &report).expect("resolvable");
        assert_eq!(merged.group(group_id).expect("group").name, "Finance");
    }

    /// A group's icon and its expiry are things a person picked, and both
    /// were missing from the content comparison. A tie on either was resolved
    /// in this copy's favour and uploaded over the other one, with no overlay
    /// and no way back: KDBX archives entry versions, not group versions.
    #[test]
    fn a_tied_group_icon_or_expiry_is_a_conflict_too() {
        let build = |icon: usize, expiry_days: i64| {
            let mut db = Database::new();
            let id = {
                let mut root = db.root_mut();
                let mut group = root.add_group();
                group.name = "Banking".to_string();
                group.id()
            };
            {
                let mut group = db.group_mut(id).unwrap();
                group.set_icon_builtin(icon);
                group.times.expiry =
                    Some(keepass::db::Times::now() + chrono::TimeDelta::days(expiry_days));
                group.times.expires = Some(true);
            }
            (db, id)
        };

        let (mut local, group_id) = build(3, 10);
        let tied = keepass::db::Times::now();
        local.group_mut(group_id).unwrap().times.last_modification = Some(tied);

        // Same group, different icon, same second.
        let mut remote = fork(&local);
        remote.group_mut(group_id).unwrap().set_icon_builtin(9);
        remote.group_mut(group_id).unwrap().times.last_modification = Some(tied);
        let report = diff(&local, &remote);
        assert_eq!(
            report.group_conflicts.len(),
            1,
            "a tied icon has to reach the user"
        );
        assert!(
            report.group_conflicts[0]
                .fields
                .iter()
                .any(|field| field.label == "Icon"),
            "and the row has to say what each side holds: {:?}",
            report.group_conflicts[0].fields
        );
        let picks = Resolutions {
            entries: HashMap::new(),
            groups: HashMap::from([(group_id.to_string(), Side::Remote)]),
            metadata: None,
        };
        let merged = apply_picks(&local, &remote, &picks, &report).expect("resolvable");
        assert_eq!(
            merged.group(group_id).unwrap().icon(),
            Some(&keepass::db::Icon::BuiltIn(9)),
            "choosing theirs has to apply theirs"
        );

        // Same group, different expiry, same second.
        let mut remote = fork(&local);
        remote.group_mut(group_id).unwrap().times.expiry =
            Some(keepass::db::Times::now() + chrono::TimeDelta::days(20));
        remote.group_mut(group_id).unwrap().times.last_modification = Some(tied);
        let report = diff(&local, &remote);
        assert_eq!(
            report.group_conflicts.len(),
            1,
            "and so does a tied expiry date"
        );
        assert!(
            report.group_conflicts[0]
                .fields
                .iter()
                .any(|field| field.label == "Expires")
        );
        let picks = Resolutions {
            entries: HashMap::new(),
            groups: HashMap::from([(group_id.to_string(), Side::Remote)]),
            metadata: None,
        };
        let merged = apply_picks(&local, &remote, &picks, &report).expect("resolvable");
        assert_eq!(
            merged.group(group_id).unwrap().times.expiry,
            remote.group(group_id).unwrap().times.expiry,
            "choosing theirs has to apply theirs"
        );
    }

    /// A collapse toggle is written without bumping the modification time, so
    /// ties on it are routine. Those must stay silent or the sync log fills
    /// with noise nobody can act on.
    #[test]
    fn a_tied_group_view_state_divergence_is_not_reported() {
        let mut local = Database::new();
        let group_id = {
            let mut root = local.root_mut();
            let mut group = root.add_group();
            group.name = "Banking".to_string();
            group.is_expanded = true;
            group.id()
        };
        let tied = keepass::db::Times::now();
        local.group_mut(group_id).unwrap().times.last_modification = Some(tied);

        let mut remote = fork(&local);
        remote.group_mut(group_id).unwrap().is_expanded = false;
        remote.group_mut(group_id).unwrap().times.last_modification = Some(tied);

        let report = diff(&local, &remote);
        assert!(
            report.group_conflicts.is_empty(),
            "a collapse toggle is not somebody's edit and must not be a question"
        );
    }

    /// The conflict overlay asks the user to pick a side. Rendering the same
    /// change list in both columns asked them to choose between two identical
    /// cells.
    #[test]
    fn the_entry_settings_row_shows_what_each_side_holds() {
        let mut local = Database::new();
        let id = add(&mut local, "Site", "pw");
        local.entry_mut(id).unwrap().set_icon_builtin(3);
        let mut remote = fork(&local);
        remote.entry_mut(id).unwrap().set_icon_builtin(9);

        let report = diff(&local, &remote);
        let conflict = report.conflicts.first().expect("tied timestamps conflict");
        let settings = conflict
            .fields
            .iter()
            .find(|field| field.label == "Entry settings")
            .expect("settings row");

        assert_ne!(
            settings.local, settings.remote,
            "the two columns have to differ if the row says they do"
        );
        assert!(settings.local.contains('3'), "{}", settings.local);
        assert!(settings.remote.contains('9'), "{}", settings.remote);
    }

    /// A missing modification timestamp on an object that is byte-identical
    /// on both sides is harmless, and blocking on it would wedge sync forever
    /// against a vault some other client wrote. A missing timestamp on an
    /// object that diverged is not, because the substitute decides a
    /// comparison the file did not.
    ///
    /// The classifier used to read the fork's sentences positionally, so a
    /// reworded warning would have flipped one of those verdicts silently.
    #[test]
    fn a_missing_timestamp_is_judged_by_the_object_not_the_wording() {
        let mut local = Database::new();
        let id = add(&mut local, "Shared", "secret");
        let mut remote = fork(&local);
        // A client that wrote the entry without a modification time. The
        // content is identical, so the fork's warning is harmless and the
        // merge has to go through.
        remote.entry_mut(id).unwrap().times.last_modification = None;

        let report = diff(&local, &remote);
        apply_picks(&local, &remote, &Resolutions::default(), &report)
            .expect("a missing timestamp on an otherwise identical entry is harmless");

        let warning = MergeWarning::MissingEntryTimestamp {
            side: keepass::db::MergeSide::Source,
            entry: id,
        };
        assert!(warning_is_harmless(&warning, &local, &remote));

        // The same warning about an entry that actually diverged stays fatal.
        remote
            .entry_mut(id)
            .unwrap()
            .set_protected(fields::PASSWORD, "changed remotely");
        assert!(
            !warning_is_harmless(&warning, &local, &remote),
            "a substitute timestamp must not decide a real divergence"
        );
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

        let merged = apply_picks(
            &local,
            &remote,
            &Resolutions::default(),
            &diff(&local, &remote),
        )
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
        let merged_fp = apply_picks(&local_fp, &cloud, &Resolutions::default(), &report)
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
        // so KP2's diff should be clean - no new entries to import,
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
            let _ = bin;
            let _ = root;
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
        let merged = apply_picks(&local, &remote, &Resolutions::default(), &report)
            .expect("recycle-bin collision should merge without resurrection");
        // Entry still exists exactly once - in the recycle bin.
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

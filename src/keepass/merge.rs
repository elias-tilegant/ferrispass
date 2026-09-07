//! Pure-data diff and three-way merge over keepass `Database`s, used by the
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
    collections::{BTreeSet, HashMap, HashSet},
    fmt,
    ops::Deref,
};

use chrono::NaiveDateTime;
use keepass::db::{
    AutoType, Color, CustomDataItem, Database, Entry, EntryId, EntryRef, GroupId, GroupRef, Icon,
    MergeWarning, Times, Value, fields,
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
#[derive(Clone, PartialEq, Eq)]
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
    icon: Option<Icon>,
    quality_check: Option<bool>,
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
    /// Names of groups that differ in user-authored content on both sides
    /// with the same modification timestamp. Groups have no conflict overlay
    /// and no timestamp to decide by, so there is no way to merge these
    /// without discarding one side's edit. `apply_picks` refuses rather than
    /// picking; the names are here so the refusal can say which group.
    pub groups_tied_and_diverged: Vec<String>,
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
            && self.remote_only.is_empty()
            && self.auto_resolved.is_empty()
            && !self.structural_writeback_required
            && !self.local_history_ahead
            && !self.remote_history_ahead
        // `local_only` doesn't dirty the merge: those entries are already in
        // the local DB we'll start the merge from.
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
    #[error(
        "group {names} changed on both machines with the same timestamp; \
         rename or edit it here, then sync again"
    )]
    GroupTieDiverged { names: String },
}

impl ApplyError {
    /// True when retrying the same two files cannot produce a different
    /// result. Auto-sync's failure recovery re-uploads the whole vault every
    /// tick, which for these costs a full upload and a full download to fail
    /// in exactly the same way. Only a local change can resolve them.
    pub fn needs_a_local_change(&self) -> bool {
        matches!(self, ApplyError::GroupTieDiverged { .. })
    }
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
        let fields = field_diffs(l, r);
        if !fields.iter().any(|f| f.differs) {
            continue;
        }
        // KeePass-style last-write-wins: when one side's `last_modification`
        // is strictly newer, take that side automatically. The overlay is
        // reserved for the genuinely ambiguous cases (timestamps tied or
        // missing) - pre-v0.4 every field-level divergence forced a prompt
        // even when the user had clearly saved one side later than the
        // other, which made benign sync round-trips noisy.
        match timestamp_winner(l.view.modified, r.view.modified) {
            Some(winner) => auto_resolved.push(AutoResolved {
                id: (*id).clone(),
                winner,
                remote: r.view.clone(),
            }),
            None => {
                if [l.view.modified, r.view.modified]
                    .into_iter()
                    .flatten()
                    .any(is_future_dated)
                {
                    future_dated.push((*id).clone());
                }
                conflicts.push(EntryConflict {
                    id: (*id).clone(),
                    local: l.view.clone(),
                    remote: r.view.clone(),
                    fields,
                });
            }
        }
    }

    let groups_tied_and_diverged = tied_group_content_divergences(local, remote);
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
        groups_tied_and_diverged,
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
            || !icons_equivalent(local_group.icon(), remote_group.icon(), DEFAULT_GROUP_ICON)
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
        {
            return true;
        }
    }

    for local_entry in local.iter_all_entries() {
        let Some(remote_entry) = remote.entry(local_entry.id()) else {
            continue;
        };
        // Only relocation counts here. Histories are compared separately and
        // directionally by `history_divergence`: comparing them as one
        // undirected "differs" bit made `has_local_contribution`
        // unconditionally true, because trimming alone diverges them.
        if local_entry.parent().id() != remote_entry.parent().id() {
            return true;
        }
    }

    false
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
    let local_cap = crate::keepass::document::history_cap(local);
    let remote_cap = crate::keepass::document::history_cap(remote);
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
    their_cap: Option<usize>,
) -> bool {
    let their_oldest = theirs.first().map(|version| version.at);
    let they_are_full = their_cap.is_some_and(|cap| theirs.len() >= cap);
    mine.iter().any(|version| {
        if theirs.contains(version) {
            return false;
        }
        match their_oldest {
            Some(oldest) if they_are_full => version.at > oldest,
            _ => true,
        }
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
}

fn history_versions(entry: &EntryRef<'_>) -> BTreeSet<HistoryVersion> {
    entry
        .history
        .iter()
        .flat_map(|history| history.get_entries())
        .filter_map(|version| {
            // Versions without a timestamp are skipped: the fork substitutes
            // the epoch for those, which would compare equal across unrelated
            // versions.
            let at = version.times.last_modification?;
            Some(HistoryVersion {
                at,
                content: history_version_digest(version),
            })
        })
        .collect()
}

/// A digest over the fields a version actually carries. Cheap, order-stable,
/// and only ever compared against another digest, never stored or shown.
fn history_version_digest(version: &Entry) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let mut hasher = Sha256::new();
    let mut fields: Vec<(&String, &Value<String>)> = version.fields.iter().collect();
    fields.sort_by_key(|(key, _)| *key);
    for (key, value) in fields {
        hasher.update(key.as_bytes());
        hasher.update([0]);
        hasher.update(value.get().as_bytes());
        hasher.update([u8::from(value.is_protected())]);
    }
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
    picks: &HashMap<String, Side>,
    report: &ConflictReport,
) -> Result<Database, ApplyError> {
    preflight_fidelity(local, remote)?;
    // No overlay, no timestamp, no way to keep both: merging here would
    // overwrite the other machine's group name, notes, tags or settings on
    // the next upload, and the user would never learn it happened.
    if !report.groups_tied_and_diverged.is_empty() {
        return Err(ApplyError::GroupTieDiverged {
            names: report
                .groups_tied_and_diverged
                .iter()
                .map(|name| format!("\"{name}\""))
                .collect::<Vec<_>>()
                .join(", "),
        });
    }

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
    reconcile_unsurfaced_metadata(&mut merged, &mut source);

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

fn icons_equivalent(local: Option<&Icon>, remote: Option<&Icon>, default_index: usize) -> bool {
    fn norm(icon: Option<&Icon>, default_index: usize) -> Option<&Icon> {
        match icon {
            Some(Icon::BuiltIn(index)) if *index == default_index => None,
            other => other,
        }
    }
    norm(local, default_index) == norm(remote, default_index)
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
    previous_group_uuids_equivalent(local.map(|id| id.uuid()), remote.map(|id| id.uuid()))
}

fn previous_group_uuids_equivalent(local: Option<uuid::Uuid>, remote: Option<uuid::Uuid>) -> bool {
    fn norm(group: Option<uuid::Uuid>) -> Option<uuid::Uuid> {
        group.filter(|id| !id.is_nil())
    }
    norm(local) == norm(remote)
}

fn entry_content_eq(local: &EntryRef<'_>, remote: &EntryRef<'_>) -> bool {
    entry_location_is_resolved(local, remote)
        && local.fields == remote.fields
        && local.autotype == remote.autotype
        && local.tags == remote.tags
        && local.custom_data == remote.custom_data
        && icons_equivalent(local.icon(), remote.icon(), DEFAULT_ENTRY_ICON)
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
        && icons_equivalent(local.icon(), remote.icon(), DEFAULT_GROUP_ICON)
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
/// Groups whose user-authored content differs on both sides with the same
/// modification timestamp. There is nothing to decide by, so the merge
/// refuses instead of silently keeping one side.
fn tied_group_content_divergences(local: &Database, remote: &Database) -> Vec<String> {
    let mut names: Vec<String> = local
        .iter_all_groups()
        .filter_map(|local_group| {
            let remote_group = remote.group(local_group.id())?;
            let tied = local_group.times.last_modification == remote_group.times.last_modification;
            (tied && group_content_differs(&local_group, &remote_group))
                .then(|| local_group.name.clone())
        })
        .collect();
    names.sort();
    names
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
}

fn reconcile_unsurfaced_metadata(merged: &mut Database, source: &mut Database) {
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
        let source_icon = source_entry.icon().cloned();
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

        if merged_icon != source_icon
            && icons_equivalent(
                merged_icon.as_ref(),
                source_icon.as_ref(),
                DEFAULT_ENTRY_ICON,
            )
            && let Some(mut entry) = source.entry_mut(id)
        {
            match merged_icon {
                Some(Icon::BuiltIn(index)) => entry.set_icon_builtin(index),
                None => entry.set_icon_none(),
                // Equivalent-but-different never involves custom icons.
                Some(Icon::Custom(_)) => {}
            }
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
        let source_icon = source_group.icon().cloned();
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

        if merged_icon != source_icon
            && icons_equivalent(
                merged_icon.as_ref(),
                source_icon.as_ref(),
                DEFAULT_GROUP_ICON,
            )
            && let Some(mut group) = source.group_mut(id)
        {
            match merged_icon {
                Some(Icon::BuiltIn(index)) => group.set_icon_builtin(index),
                None => group.set_icon_none(),
                Some(Icon::Custom(_)) => {}
            }
        }

        // View state only. A KeePassXC IsExpanded toggle, an icon respelling
        // or a previous-parent normalisation is written without bumping the
        // modification time, so ties on those are routine, carry no user
        // intent, and would still trip the fork's fail-closed divergence
        // check. Break those in local's favour; the bumped timestamp also
        // carries fork-private view state (LastTopVisibleEntry) past the
        // check.
        //
        // A tie on the name, notes, tags or settings is somebody's real edit
        // and is never resolved here: `apply_picks` has already refused the
        // whole merge, because keeping one side would delete the other on the
        // next upload with nothing shown to either user.
        let still_diverged = match (merged.group(id), source.group(id)) {
            (Some(merged_group), Some(source_group)) => {
                merged_group.icon() != source_group.icon()
                    || merged_group.is_expanded != source_group.is_expanded
                    || merged_group.previous_parent_group != source_group.previous_parent_group
            }
            _ => false,
        };
        if still_diverged {
            let winner_time = resolution_time([
                merged
                    .group(id)
                    .and_then(|group| group.times.last_modification),
                None,
            ]);
            if let Some(mut group) = merged.group_mut(id) {
                group.times.last_modification = Some(winner_time);
            }
            if let Some(mut group) = source.group_mut(id) {
                group.times.last_modification = Some(Times::epoch());
            }
        }
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
        icon: e.icon().cloned(),
        quality_check: e.quality_check,
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

    diffs.push(FieldDiff {
        label: "Attachments",
        local: attachment_summary(local.attachments.len()),
        remote: attachment_summary(remote.attachments.len()),
        differs: local.attachments != remote.attachments,
    });

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
        // Per-side values, not a shared change list. The row is flagged as
        // differing, so rendering the same string in both columns asked the
        // user to choose between two identical cells.
        diffs.push(FieldDiff {
            label: "Entry settings",
            local: metadata
                .iter()
                .map(|difference| difference.local.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            remote: metadata
                .iter()
                .map(|difference| difference.remote.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            differs: true,
        });
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

/// One entry-setting difference, with what each side actually holds.
struct MetadataDifference {
    local: String,
    remote: String,
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
        note(
            describe("Auto-Type", &local.view.autotype),
            describe("Auto-Type", &remote.view.autotype),
        );
    }
    if local.view.custom_data != remote.view.custom_data {
        note(
            format!("{} custom values", local.view.custom_data.len()),
            format!("{} custom values", remote.view.custom_data.len()),
        );
    }
    if !icons_equivalent(
        local.icon.as_ref(),
        remote.icon.as_ref(),
        DEFAULT_ENTRY_ICON,
    ) {
        note(
            icon_label(local.icon.as_ref()),
            icon_label(remote.icon.as_ref()),
        );
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

fn icon_label(icon: Option<&Icon>) -> String {
    match icon {
        Some(Icon::BuiltIn(index)) => format!("built-in icon {index}"),
        Some(Icon::Custom(_)) => "custom icon".to_string(),
        None => "default icon".to_string(),
    }
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
        assert!(previous_group_uuids_equivalent(
            Some(uuid::Uuid::nil()),
            None,
        ));

        let actual_group = uuid::Uuid::new_v4();
        assert!(!previous_group_uuids_equivalent(Some(actual_group), None));
        assert!(!previous_group_uuids_equivalent(
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
        let merged = apply_picks(&local, &remote, &HashMap::new(), &report)
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
        apply_picks(&local, &remote, &HashMap::new(), &report)
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
        let merged = apply_picks(&local, &remote, &HashMap::new(), &report)
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
        let merged = apply_picks(&local, &remote, &picks, &report)
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
        let merged = apply_picks(&local, &remote, &picks, &report)
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
        let merged = apply_picks(&local, &remote, &picks, &report)
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
        assert!(log.warnings.contains(&MergeWarning::MissingEntryTimestamp {
            side: keepass::db::MergeSide::Source,
            entry: entry_id,
        }));

        let error = apply_picks(&local, &remote, &HashMap::new(), &diff(&local, &remote))
            .expect_err("an unorderable move must remain fail-closed");
        assert!(matches!(
            error,
            ApplyError::DatabaseMergeWarnings(message)
                if message.contains(&entry_id.to_string())
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
        assert!(log.warnings.contains(&MergeWarning::MissingGroupTimestamp {
            side: keepass::db::MergeSide::Source,
            group: group_id,
        }));

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
        assert!(log.warnings.contains(&MergeWarning::MissingGroupTimestamp {
            side: keepass::db::MergeSide::Source,
            group: group_id,
        }));

        let error = apply_picks(&local, &remote, &HashMap::new(), &diff(&local, &remote))
            .expect_err("divergent current group content must stay fatal");
        assert!(matches!(
            error,
            ApplyError::DatabaseMergeWarnings(message)
                if message.contains(&group_id.to_string())
        ));

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

        let merged = apply_picks(&local, &remote, &HashMap::new(), &diff(&local, &remote))
            .expect("a remote icon is merged, not refused");

        let entry = merged.entry(id).expect("entry survives");
        assert_eq!(
            entry.custom_icon().expect("icon resolves").data,
            image,
            "the image travelled with the reference"
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
        let merged = apply_picks(&local, &remote, &picks, &report).expect("resolvable");

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
        let merged = apply_picks(&local, &remote, &picks, &report).expect("resolvable");

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

        // The other client keeps one version, and is at that limit, so the
        // older one is demonstrably something it dropped rather than
        // something it never had.
        let mut remote = fork(&local);
        remote.meta.history_max_items = Some(1);
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

    /// A group renamed on both machines in the same second has no overlay and
    /// no timestamp to decide by. The merge used to keep the local name and
    /// bump its timestamp, so the next upload deleted the other user's rename
    /// with only a line in a session-scoped log to show for it.
    #[test]
    fn a_tied_group_content_divergence_blocks_the_merge() {
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
        assert_eq!(report.groups_tied_and_diverged, vec!["Banking".to_string()]);

        let error = apply_picks(&local, &remote, &HashMap::new(), &report)
            .expect_err("neither name may be discarded");
        assert!(matches!(error, ApplyError::GroupTieDiverged { .. }));
        assert!(
            error.to_string().contains("Banking"),
            "the message names the group the user has to resolve: {error}"
        );
        assert!(
            error.needs_a_local_change(),
            "retrying the same two files fails identically"
        );
    }

    /// Breaking the tie is the user's way out, so an edit on either side has
    /// to unblock the merge without further ceremony.
    #[test]
    fn editing_the_group_locally_unblocks_the_merge() {
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

        // What renaming the group in FerrisPass does: a newer timestamp.
        local.group_mut(group_id).unwrap().times.last_modification =
            Some(tied + chrono::TimeDelta::seconds(1));

        let report = diff(&local, &remote);
        assert!(report.groups_tied_and_diverged.is_empty());
        let merged = apply_picks(&local, &remote, &HashMap::new(), &report)
            .expect("the newer side decides once there is one");
        assert_eq!(merged.group(group_id).expect("group").name, "Banking");
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
            report.groups_tied_and_diverged.is_empty(),
            "a collapse toggle is not somebody's edit and must not block sync"
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
        apply_picks(&local, &remote, &HashMap::new(), &report)
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
        let merged = apply_picks(&local, &remote, &HashMap::new(), &report)
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

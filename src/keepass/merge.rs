//! Pure-data diff and three-way merge over keepass `Database`s, used by the
//! sync conflict resolution flow. No GPUI dependencies — fully unit-testable.
//!
//! Scope:
//! - Entry-grain diff over UUID-identified entries. Title / Username /
//!   Password / URL / Notes / Tags are surfaced in the conflict UI as
//!   side-by-side rows. Custom-data, AutoType, and colors are preserved
//!   silently (replaced when user picks Remote, kept when user picks Local)
//!   without rendering as diff rows — they're rarely user-visible in
//!   normal vault use, so surfacing each one would be UI noise.
//! - Remote-only entries are imported with their **original UUID** preserved
//!   via `Group::add_entry_with_id`. This is essential for cross-client sync:
//!   without it, every merge cycle re-randomises UUIDs, and other clients
//!   (KeePass2, KeePassXC) treat the entry as new on their side — leading
//!   to exponential entry duplication on each round trip.
//! - Recycle-bin entries are filtered out — they're effectively deleted from
//!   the user's perspective. Edge case "X live on one side, X recycled on
//!   the other" therefore presents as a one-sided add (the live side wins
//!   silently). Acceptable; refining requires surfacing delete-vs-edit
//!   conflicts, which is its own project.
//! - Group additions are not detected. A remote-only entry lands in the
//!   merged vault under the local root group, regardless of where it sat in
//!   the remote tree. Documented limitation.
//! - **Not preserved** across a merge: icon bytes, attachments, history.
//!   Both icon and attachments are accessed through private fields in
//!   keepass-rs; exposing them is a separate fork-patch chunk. History
//!   is intentionally reset because the merge itself is a fresh write.
//! - Passwords are compared in cleartext (necessarily — both sides are
//!   already decrypted) but the displayed `FieldDiff.local`/`.remote` for
//!   the Password row is redacted to `"••• (N chars)"` so the conflict
//!   screen is screen-sharing-safe.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDateTime;
use keepass::db::{
    AutoType, Color, CustomDataItem, Database, EntryId, EntryMut, EntryRef, GroupId, Times, fields,
};

use crate::domain::CustomField;
use crate::keepass::repository::{STANDARD_FIELDS, collect_custom_fields};

/// Value snapshot of an entry at the moment of diffing — owned, no borrows
/// of the source `Database`. Safe to keep around in UI state for as long as
/// the user is reviewing the conflict.
///
/// Carries the full set of entry fields the merge round-trips, not just the
/// five visible-in-UI ones. When the user picks "Remote" for a conflict, all
/// these fields get transplanted onto the local entry — partial transplants
/// were the source of a silent-data-loss bug pre-v0.2.1.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// One field's local-vs-remote comparison. `local` and `remote` are the
/// strings the UI should render directly — for the Password row those are
/// pre-redacted; for the rest they're the cleartext field values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDiff {
    pub label: &'static str,
    pub local: String,
    pub remote: String,
    pub differs: bool,
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
}

impl ConflictReport {
    /// True when no user decisions are required — diff was clean. Caller
    /// can skip the Conflict overlay entirely and just upload `apply_picks`
    /// with an empty pick map.
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty() && self.remote_only.is_empty() && self.auto_resolved.is_empty()
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
        !self.local_only.is_empty()
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
        match timestamp_winner(l.modified, r.modified) {
            Some(winner) => auto_resolved.push(AutoResolved {
                id: (*id).clone(),
                winner,
                remote: r.clone(),
            }),
            None => conflicts.push(EntryConflict {
                id: (*id).clone(),
                local: l.clone(),
                remote: r.clone(),
                fields,
            }),
        }
    }

    let mut local_only: Vec<EntryView> = local_ids
        .difference(&remote_ids)
        .map(|id| local_map[*id].clone())
        .collect();
    let mut remote_only: Vec<EntryView> = remote_ids
        .difference(&local_ids)
        .map(|id| remote_map[*id].clone())
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
    }
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

/// Build a merged `Database` from `local` + the user's resolution choices.
///
/// - For each conflict in `report`: if the pick is `Remote`, replace the
///   entry's standard fields in the merged copy with the remote view.
///   `Local` (or unmapped) → keep as-is.
/// - For each `remote_only` entry: add a fresh entry to the merged root
///   with those fields. The original remote `EntryId` is *not* preserved
///   (no public API to insert with a specific id) — documented MVP loss.
/// - `local_only` entries are inherited from the cloned base; nothing to do.
///
/// Modification times are stamped to "now" on any entry we touch so other
/// clients see the merge as a fresh write.
pub fn apply_picks(
    local: &Database,
    _remote: &Database,
    picks: &HashMap<String, Side>,
    report: &ConflictReport,
) -> Database {
    let mut merged = local.clone();

    // We can't reconstruct EntryId from the string in `report.conflicts[].id`
    // because `EntryId::from_uuid` is `pub(crate)` in keepass-rs. Instead we
    // walk the cloned database once and build a string→EntryId lookup, then
    // use it to drive the per-conflict replacements.
    let id_lookup: HashMap<String, EntryId> = merged
        .iter_all_entries()
        .map(|e| (e.id().to_string(), e.id()))
        .collect();

    for conflict in &report.conflicts {
        let Some(&entry_id) = id_lookup.get(&conflict.id) else {
            continue;
        };
        let side = picks.get(&conflict.id).copied().unwrap_or(Side::Local);
        if side == Side::Remote {
            replace_entry_fields(&mut merged, entry_id, &conflict.remote);
        }
    }

    // Auto-resolved entries (timestamp-based last-write-wins) get applied
    // unconditionally — they never appear in `picks` because the user was
    // never asked. Local-winners are no-ops since `merged` started as a
    // clone of local; only Remote-winners need their fields transplanted.
    for resolved in &report.auto_resolved {
        if resolved.winner != Side::Remote {
            continue;
        }
        let Some(&entry_id) = id_lookup.get(&resolved.id) else {
            continue;
        };
        replace_entry_fields(&mut merged, entry_id, &resolved.remote);
    }

    let root_id = merged.root().id();
    for view in &report.remote_only {
        add_entry_under(&mut merged, root_id, view);
    }

    merged
}

// ---------- internals ----------

fn live_entries(db: &Database) -> HashMap<String, EntryView> {
    let recycle_bin_id: Option<GroupId> = db.recycle_bin().map(|g| g.id());
    db.iter_all_entries()
        .filter(|e| {
            // "Live" = not directly inside the recycle bin. We don't recurse
            // into recycle-bin subgroups because (a) they're rare and (b)
            // surfacing those as conflicts is more annoying than helpful.
            recycle_bin_id.map_or(true, |bin| e.parent().id() != bin)
        })
        .map(|e| {
            let view = entry_to_view(&e);
            (view.id.clone(), view)
        })
        .collect()
}

fn entry_to_view(e: &EntryRef<'_>) -> EntryView {
    EntryView {
        id: e.id().to_string(),
        title: e.get(fields::TITLE).unwrap_or("").to_string(),
        username: e.get(fields::USERNAME).unwrap_or("").to_string(),
        password: e.get(fields::PASSWORD).unwrap_or("").to_string(),
        url: e.get(fields::URL).unwrap_or("").to_string(),
        notes: e.get(fields::NOTES).unwrap_or("").to_string(),
        modified: e.times.last_modification,
        // EntryRef Derefs to Entry, so the public fields below are reached
        // straight through. Cloning here is cheap relative to KDF cost on
        // any subsequent save.
        tags: e.tags.clone(),
        custom_data: e.custom_data.clone(),
        custom_fields: collect_custom_fields(e),
        autotype: e.autotype.clone(),
        foreground_color: e.foreground_color.clone(),
        background_color: e.background_color.clone(),
        override_url: e.override_url.clone(),
    }
}

fn field_diffs(local: &EntryView, remote: &EntryView) -> Vec<FieldDiff> {
    vec![
        plain_diff("Title", &local.title, &remote.title),
        plain_diff("Username", &local.username, &remote.username),
        password_diff(&local.password, &remote.password),
        plain_diff("URL", &local.url, &remote.url),
        plain_diff("Notes", &local.notes, &remote.notes),
        tags_diff(&local.tags, &remote.tags),
    ]
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

fn plain_diff(label: &'static str, local: &str, remote: &str) -> FieldDiff {
    FieldDiff {
        label,
        local: local.to_string(),
        remote: remote.to_string(),
        differs: local != remote,
    }
}

fn password_diff(local: &str, remote: &str) -> FieldDiff {
    FieldDiff {
        label: "Password",
        local: redact(local),
        remote: redact(remote),
        // Compare cleartext for the differs flag; only the displayed strings
        // get redacted so the screen is safe to share.
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

/// Copy every field from `view` onto `entry`. Shared by the conflict-pick
/// path (`replace_entry_fields`) and the remote-only-import path
/// (`add_entry_under`) so they stay in lockstep — pre-v0.2.1 they each had
/// their own field list and drifted, causing tags/custom-data to silently
/// not be transplanted when the user picked Remote.
fn populate_from_view(entry: &mut EntryMut<'_>, view: &EntryView) {
    entry.set_unprotected(fields::TITLE, &view.title);
    entry.set_unprotected(fields::USERNAME, &view.username);
    entry.set_protected(fields::PASSWORD, &view.password);
    entry.set_unprotected(fields::URL, &view.url);
    entry.set_unprotected(fields::NOTES, &view.notes);
    entry.tags = view.tags.clone();
    entry.custom_data = view.custom_data.clone();
    // Replace non-standard fields wholesale: drop everything outside the
    // standard set, then re-write from the view. Pre-fix this step was
    // missing entirely and "pick remote" silently dropped any custom
    // fields off the local entry — the SAP launcher's whole config
    // would have evaporated on the next conflict resolution.
    let drop: Vec<String> = entry
        .fields
        .keys()
        .filter(|k| !STANDARD_FIELDS.contains(&k.as_str()))
        .cloned()
        .collect();
    for key in drop {
        entry.fields.remove(&key);
    }
    for cf in &view.custom_fields {
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
    entry.autotype = view.autotype.clone();
    entry.foreground_color = view.foreground_color.clone();
    entry.background_color = view.background_color.clone();
    entry.override_url = view.override_url.clone();
}

fn replace_entry_fields(db: &mut Database, id: EntryId, view: &EntryView) {
    let Some(mut entry) = db.entry_mut(id) else {
        return;
    };
    populate_from_view(&mut entry, view);
    entry.times.last_modification = Some(Times::now());
}

fn add_entry_under(db: &mut Database, group_id: GroupId, view: &EntryView) {
    // Re-hydrate the original remote-side EntryId from its UUID string so
    // the imported entry keeps the identity other KeePass clients know it
    // by. Falls back to a fresh-UUID add if the string isn't a valid UUID
    // (shouldn't happen — `entry_to_view` produces these via
    // `EntryId::to_string` — but stay defensive rather than panic).
    let entry_id = match uuid::Uuid::parse_str(&view.id) {
        Ok(uuid) => Some(EntryId::from_uuid(uuid)),
        Err(_) => {
            eprintln!(
                "merge: remote entry UUID unparseable, importing with fresh UUID: {}",
                view.id
            );
            None
        }
    };

    // `remote_only` is computed from the *live* view of both sides — entries
    // inside the recycle bin are filtered out of `live_entries`. An entry the
    // local user trashed (still in their bin) AND that the remote has live
    // would otherwise land here with an id that *does* collide with a row
    // already in the cloned `merged` database. `add_entry_with_id` panics on
    // that, so guard explicitly: if we already know this id, the local user
    // has expressed an intent for it (trash). Skip the re-import rather than
    // resurrect, matching the "last writer in *this* client wins" stance.
    if let Some(id) = entry_id
        && db.iter_all_entries().any(|e| e.id() == id)
    {
        return;
    }

    let Some(mut group) = db.group_mut(group_id) else {
        return;
    };
    let mut entry = match entry_id {
        Some(id) => group.add_entry_with_id(id),
        None => group.add_entry(),
    };
    populate_from_view(&mut entry, view);
    entry.times.last_modification = Some(Times::now());
    entry.times.creation = Some(Times::now());
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

        let merged = apply_picks(&local, &remote, &HashMap::new(), &report);
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

        let merged = apply_picks(&local, &remote, &HashMap::new(), &report);
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
        let merged = apply_picks(&local, &remote, &HashMap::new(), &report);
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

        let merged = apply_picks(&local, &remote, &picks, &report);
        let entry = merged.entry(id).unwrap();
        assert_eq!(entry.get_password(), Some("remote-pw"));
    }

    #[test]
    fn apply_picks_adds_remote_only_entries_to_root() {
        let local = Database::new();
        let mut remote = fork(&local);
        let remote_id = add(&mut remote, "NewRemote", "remote-secret");

        let report = diff(&local, &remote);
        let merged = apply_picks(&local, &remote, &HashMap::new(), &report);

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
        let merged = apply_picks(&local, &remote, &picks, &report);

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
        let merged = apply_picks(&local, &remote, &picks, &report);
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
        let merged_fp = apply_picks(&local_fp, &cloud, &HashMap::new(), &report);
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

    /// Regression: local has an entry in its recycle bin while remote still
    /// has the same id live. `live_entries` filters bin rows on both sides,
    /// so the id appears in `remote_only` — but `merged = local.clone()`
    /// preserves the bin row, and an unchecked `add_entry_with_id` would
    /// panic with "Entry with ID ... already exists". The guard in
    /// `add_entry_under` must catch this and skip the import.
    #[test]
    fn apply_picks_does_not_panic_when_remote_only_collides_with_local_bin() {
        // Build local with one entry, then trash it (recycle bin) so it
        // disappears from the live view but stays in `db.entries`.
        let mut local = Database::new();
        let id = add(&mut local, "WasTrashed", "x");
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

        // Remote still has the same entry live — fork before the local
        // trash, give it some content so the diff has something to do.
        let mut remote = Database::new();
        {
            let mut root = remote.root_mut();
            let mut e = root.add_entry_with_id(id);
            e.set_unprotected(fields::TITLE, "WasTrashed");
            e.set_unprotected(fields::USERNAME, "user");
            e.set_protected(fields::PASSWORD, "x");
        }

        let report = diff(&local, &remote);
        // Bug precondition: local has it in the bin (filtered), remote has
        // it live → diff classifies as remote_only.
        assert_eq!(report.remote_only.len(), 1);
        assert_eq!(report.remote_only[0].id, id.to_string());

        // Must not panic. The collision check skips the import, preserving
        // the local user's "I trashed this" intent.
        let merged = apply_picks(&local, &remote, &HashMap::new(), &report);
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

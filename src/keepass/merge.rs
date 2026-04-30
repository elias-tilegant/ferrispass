//! Pure-data diff and three-way merge over keepass `Database`s, used by the
//! sync conflict resolution flow. No GPUI dependencies — fully unit-testable.
//!
//! Scope (MVP):
//! - Entry-grain diff. Title / Username / Password / URL / Notes are
//!   compared; everything else (icons, tags, custom fields, attachments,
//!   AutoType, group hierarchy) is preserved from whichever side wins per
//!   entry but not surfaced in the conflict UI.
//! - Recycle-bin entries are filtered out — they're effectively deleted from
//!   the user's perspective. Edge case "X live on one side, X recycled on
//!   the other" therefore presents as a one-sided add (the live side wins
//!   silently). That's acceptable for MVP; refining requires surfacing
//!   delete-vs-edit conflicts, which is its own project.
//! - Group additions are not detected. A remote-only entry lands in the
//!   merged vault under the local root group, regardless of where it sat in
//!   the remote tree. Documented limitation.
//! - Passwords are compared in cleartext (necessarily — both sides are
//!   already decrypted) but the displayed `FieldDiff.local`/`.remote` for
//!   the Password row is redacted to `"••• (N chars)"` so the conflict
//!   screen is screen-sharing-safe.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDateTime;
use keepass::db::{fields, Database, EntryId, EntryRef, GroupId, Times};

/// Value snapshot of an entry at the moment of diffing — owned, no borrows
/// of the source `Database`. Safe to keep around in UI state for as long as
/// the user is reviewing the conflict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryView {
    /// EntryId stringified to its UUID. Stable across diff/apply.
    pub id: String,
    pub title: String,
    pub username: String,
    /// Cleartext. Only ever rendered through `FieldDiff` (which redacts) or
    /// in the entry detail view after explicit reveal.
    pub password: String,
    pub url: String,
    pub notes: String,
    pub modified: Option<NaiveDateTime>,
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

/// The full picture handed to the Conflict overlay. `conflicts` is the list
/// the user must resolve; `local_only` / `remote_only` are auto-merged.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ConflictReport {
    pub conflicts: Vec<EntryConflict>,
    pub local_only: Vec<EntryView>,
    pub remote_only: Vec<EntryView>,
}

impl ConflictReport {
    /// True when no user decisions are required — diff was clean. Caller
    /// can skip the Conflict overlay entirely and just upload `apply_picks`
    /// with an empty pick map.
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty() && self.remote_only.is_empty()
        // `local_only` doesn't dirty the merge: those entries are already in
        // the local DB we'll start the merge from.
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
    for id in local_ids.intersection(&remote_ids) {
        let l = &local_map[*id];
        let r = &remote_map[*id];
        let fields = field_diffs(l, r);
        if fields.iter().any(|f| f.differs) {
            conflicts.push(EntryConflict {
                id: (*id).clone(),
                local: l.clone(),
                remote: r.clone(),
                fields,
            });
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

    ConflictReport {
        conflicts,
        local_only,
        remote_only,
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
    }
}

fn field_diffs(local: &EntryView, remote: &EntryView) -> Vec<FieldDiff> {
    vec![
        plain_diff("Title", &local.title, &remote.title),
        plain_diff("Username", &local.username, &remote.username),
        password_diff(&local.password, &remote.password),
        plain_diff("URL", &local.url, &remote.url),
        plain_diff("Notes", &local.notes, &remote.notes),
    ]
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

fn replace_entry_fields(db: &mut Database, id: EntryId, view: &EntryView) {
    let Some(mut entry) = db.entry_mut(id) else { return };
    entry.set_unprotected(fields::TITLE, &view.title);
    entry.set_unprotected(fields::USERNAME, &view.username);
    entry.set_protected(fields::PASSWORD, &view.password);
    entry.set_unprotected(fields::URL, &view.url);
    entry.set_unprotected(fields::NOTES, &view.notes);
    entry.times.last_modification = Some(Times::now());
}

fn add_entry_under(db: &mut Database, group_id: GroupId, view: &EntryView) {
    let Some(mut group) = db.group_mut(group_id) else { return };
    let mut entry = group.add_entry();
    entry.set_unprotected(fields::TITLE, &view.title);
    entry.set_unprotected(fields::USERNAME, &view.username);
    entry.set_protected(fields::PASSWORD, &view.password);
    entry.set_unprotected(fields::URL, &view.url);
    entry.set_unprotected(fields::NOTES, &view.notes);
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
        assert!(report.local_only.is_empty(),
            "recycle-bin entries must be filtered out, got {:?}", report.local_only);
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
        add(&mut remote, "NewRemote", "remote-secret");

        let report = diff(&local, &remote);
        let merged = apply_picks(&local, &remote, &HashMap::new(), &report);

        // Find by title since the remote entry's UUID isn't preserved through
        // apply_picks (documented MVP limitation — no public add-with-id API).
        let added = merged
            .iter_all_entries()
            .find(|e| e.get_title() == Some("NewRemote"))
            .expect("remote-only entry should be in merged result");
        assert_eq!(added.get_password(), Some("remote-secret"));
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
        let titles: Vec<&str> = report.remote_only.iter().map(|v| v.title.as_str()).collect();
        assert_eq!(titles, vec!["Alpha", "Mango", "Zebra"]);
    }
}

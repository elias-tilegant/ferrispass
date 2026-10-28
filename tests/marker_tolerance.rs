//! Markers: the plugin data a newer version of this app keeps under
//! `FerrisPass.` keys, on entries, on groups and on the database.
//!
//! This version writes none and reads none, so a difference there is never
//! something its user did. It is never reported, and a merge keeps the
//! markers of both copies whichever copy wins: a key only one copy holds
//! stays, and two values under one key go by the later item time, or to the
//! remote copy where the item times cannot decide.

use std::{ops::Deref as _, sync::OnceLock};

use chrono::{NaiveDateTime, TimeDelta};
use ferrispass::keepass::merge::{self, ConflictReport, FieldDiff, Resolutions, Side};
use keepass::{
    Database,
    db::{CustomDataItem, CustomDataValue, EntryId, GroupId, Times, fields},
};

/// The three places a marker can sit.
#[derive(Clone, Copy, Debug)]
enum Place {
    Entry,
    Group,
    Database,
}

const PLACES: [Place; 3] = [Place::Entry, Place::Group, Place::Database];

/// One entry in one group of a vault, everything last changed at
/// `base_time()`.
struct Vault {
    entry: EntryId,
    group: GroupId,
}

fn base_time() -> NaiveDateTime {
    days_ago(3)
}

/// Relative to one moment per test run. KDBX times have whole seconds, and
/// two copies are only tied when both were stamped with the same one, which
/// separate calls to the clock do not promise.
fn days_ago(days: i64) -> NaiveDateTime {
    static NOW: OnceLock<NaiveDateTime> = OnceLock::new();
    *NOW.get_or_init(Times::now) - TimeDelta::days(days)
}

fn vault() -> (Database, Vault) {
    let at = base_time();
    let mut db = Database::new();
    let group = {
        let mut root = db.root_mut();
        let mut group = root.add_group();
        group.name = "Customers".into();
        group.times.last_modification = Some(at);
        group.id()
    };
    let entry = {
        let mut group = db.group_mut(group).expect("group");
        let mut entry = group.add_entry();
        entry.set_unprotected(fields::TITLE, "SAP PRD");
        entry.set_unprotected(fields::USERNAME, "ddic");
        entry.set_protected(fields::PASSWORD, "throwaway");
        entry.times.last_modification = Some(at);
        entry.id()
    };
    db.meta.database_name = Some("Team".into());
    db.meta.database_name_changed = Some(at);
    (db, Vault { entry, group })
}

fn marker(value: &str, at: Option<NaiveDateTime>) -> CustomDataItem {
    CustomDataItem {
        value: Some(CustomDataValue::String(value.into())),
        last_modification_time: at,
    }
}

impl Vault {
    /// A kind on the entry, a role on the group, and in the database a
    /// favourite, which names the person's list and the entry.
    fn key(&self, place: Place) -> String {
        match place {
            Place::Entry => "FerrisPass.Type".into(),
            Place::Group => "FerrisPass.Role".into(),
            Place::Database => format!("FerrisPass.Fav.team-a.{}", self.entry),
        }
    }

    fn held(&self, db: &Database, place: Place) -> Option<CustomDataItem> {
        let key = self.key(place);
        match place {
            Place::Entry => db.entry(self.entry)?.custom_data.get(&key).cloned(),
            Place::Group => db.group(self.group)?.custom_data.get(&key).cloned(),
            Place::Database => db.meta.custom_data.get(&key).cloned(),
        }
    }

    /// The way a newer version writes a marker: nothing else moves.
    fn mark(&self, db: &mut Database, place: Place, item: CustomDataItem) {
        let key = self.key(place);
        match place {
            Place::Entry => {
                let mut entry = db.entry_mut(self.entry).expect("entry");
                entry.custom_data.insert(key, item);
            }
            Place::Group => {
                let mut group = db.group_mut(self.group).expect("group");
                group.custom_data.insert(key, item);
            }
            Place::Database => {
                db.meta.custom_data.insert(key, item);
            }
        }
    }

    /// An edit somebody made: the username, the group's name or the
    /// database's name, changed at `at`.
    fn edit(&self, db: &mut Database, place: Place, text: &str, at: NaiveDateTime) {
        match place {
            Place::Entry => {
                let mut entry = db.entry_mut(self.entry).expect("entry");
                entry.set_unprotected(fields::USERNAME, text);
                entry.times.last_modification = Some(at);
            }
            Place::Group => {
                let mut group = db.group_mut(self.group).expect("group");
                group.name = text.into();
                group.times.last_modification = Some(at);
            }
            Place::Database => {
                db.meta.database_name = Some(text.into());
                db.meta.database_name_changed = Some(at);
            }
        }
    }

    fn edited(&self, db: &Database, place: Place) -> String {
        match place {
            Place::Entry => db
                .entry(self.entry)
                .and_then(|entry| entry.get(fields::USERNAME).map(str::to_string))
                .unwrap_or_default(),
            Place::Group => db
                .group(self.group)
                .map(|group| group.name.clone())
                .unwrap_or_default(),
            Place::Database => db.meta.database_name.clone().unwrap_or_default(),
        }
    }

    /// Archive the entry as it stands, with other notes, dated `at` and
    /// carrying `marker` if there is one.
    fn archive(
        &self,
        db: &mut Database,
        notes: &str,
        at: NaiveDateTime,
        marker: Option<CustomDataItem>,
    ) {
        let mut version = db.entry(self.entry).expect("entry").deref().clone();
        version.set_unprotected(fields::NOTES, notes);
        version.times.last_modification = Some(at);
        version.history = None;
        if let Some(marker) = marker {
            version.custom_data.insert(self.key(Place::Entry), marker);
        }
        let mut entry = db.entry_mut(self.entry).expect("entry");
        entry.history.get_or_insert_default().add_entry(version);
    }
}

/// Diff, then merge with `winner` taking every question asked.
fn merge(local: &Database, remote: &Database, winner: Side) -> Database {
    let report = merge::diff(local, remote);
    let mut picks = Resolutions::default();
    for conflict in &report.conflicts {
        picks.entries.insert(conflict.id.clone(), winner);
    }
    for conflict in &report.group_conflicts {
        picks.groups.insert(conflict.id.clone(), winner);
    }
    if report.metadata_conflict.is_some() {
        picks.metadata = Some(winner);
    }
    merge::apply_picks(local, remote, &picks, &report).expect("the two copies merge")
}

fn other(side: Side) -> Side {
    match side {
        Side::Local => Side::Remote,
        Side::Remote => Side::Local,
    }
}

/// Both ways round, because the next sync runs from whichever copy saved
/// last.
fn reports(a: &Database, b: &Database) -> [ConflictReport; 2] {
    [merge::diff(a, b), merge::diff(b, a)]
}

/// Nothing to ask, nothing to take and nothing to send.
fn assert_nothing_reported(what: &str, a: &Database, b: &Database) {
    for report in reports(a, b) {
        assert_eq!(report, ConflictReport::default(), "{what}");
        assert!(!report.has_local_contribution(), "{what}");
    }
}

/// The labels of the rows a question shows, and whether any row, label or
/// value, says anything about a marker.
fn rows(fields: &[FieldDiff]) -> (Vec<&str>, bool) {
    let labels = fields
        .iter()
        .filter(|field| field.differs)
        .map(|field| field.label.as_ref())
        .collect();
    let mentions_markers = fields.iter().any(|field| {
        [
            field.label.as_ref(),
            field.local.as_str(),
            field.remote.as_str(),
        ]
        .iter()
        .any(|text| text.contains("FerrisPass"))
    });
    (labels, mentions_markers)
}

#[test]
fn markers_never_appear_in_the_report() {
    for at in [Some(days_ago(2)), None] {
        let (ours, vault) = vault();

        let mut theirs = ours.clone();
        for place in PLACES {
            vault.mark(&mut theirs, place, marker("sap", at));
        }
        let what = format!("markers on one side, item time {at:?}");
        assert_nothing_reported(&what, &ours, &theirs);

        let mut ours = ours;
        for place in PLACES {
            vault.mark(&mut ours, place, marker("hana", at));
        }
        let what = format!("two values, item time {at:?}");
        assert_nothing_reported(&what, &ours, &theirs);
    }

    // A real difference beside a marker is still asked about, and the
    // question is about that difference alone.
    let (ours, vault) = vault();
    let mut theirs = ours.clone();
    for place in PLACES {
        vault.mark(&mut theirs, place, marker("sap", Some(days_ago(2))));
        vault.edit(&mut theirs, place, "carol", base_time());
    }
    for report in reports(&ours, &theirs) {
        let [entry] = report.conflicts.as_slice() else {
            panic!("the tied username is asked: {report:?}");
        };
        assert_eq!(rows(&entry.fields), (vec!["Username"], false));
        let [group] = report.group_conflicts.as_slice() else {
            panic!("the tied group name is asked: {report:?}");
        };
        assert_eq!(rows(&group.fields), (vec!["Name"], false));
        let settings = report
            .metadata_conflict
            .as_ref()
            .expect("the tied database name is asked");
        assert_eq!(rows(&settings.fields), (vec!["Database name"], false));
    }
}

/// Two copies hold the same archived versions, and only one copy's carry
/// markers. Neither copy holds a version the other lacks.
///
/// Nor does a merge drop anything there. The fork keeps two versions of one
/// second that differ as two, so each copy's version stays as it was
/// archived. An archived version is a record, and nothing settles which of
/// its two values is right.
#[test]
fn markers_on_archived_versions_never_appear_in_the_report() {
    let (mut ours, vault) = vault();
    let mut theirs = ours.clone();
    let versions = [("first", days_ago(5)), ("second", days_ago(4))];
    for (notes, at) in versions {
        vault.archive(&mut ours, notes, at, None);
        let marked = Some(marker("sap", Some(at)));
        vault.archive(&mut theirs, notes, at, marked);
    }
    assert_nothing_reported("archived versions", &ours, &theirs);

    let key = vault.key(Place::Entry);
    for (local, remote) in [(&ours, &theirs), (&theirs, &ours)] {
        let merged = merge(local, remote, Side::Local);
        let entry = merged.entry(vault.entry).expect("entry");
        let archived = entry.history.as_ref().expect("history").get_entries();
        for (notes, at) in versions {
            let kept: Vec<Option<&CustomDataItem>> = archived
                .iter()
                .filter(|version| version.get(fields::NOTES) == Some(notes))
                .map(|version| version.custom_data.get(&key))
                .collect();
            assert!(kept.contains(&None), "{notes}: the version without");
            assert!(
                kept.contains(&Some(&marker("sap", Some(at)))),
                "{notes}: the version with the marker"
            );
        }
    }
}

/// Whether the other copy could have trimmed a version away is decided on
/// what its history holds against what it may hold. The markers on its
/// versions must not tip that either way.
///
/// Here this copy holds an older version the other one lacks, and the
/// other copy's size limit is set exactly where that version still fits:
/// it never received it and has to be sent it. Counted, the markers on its
/// versions made that history look full, the missing version look trimmed,
/// and the upload never happened.
#[test]
fn markers_do_not_count_against_a_history_size_limit() {
    let (mut ours, vault) = vault();
    let mut theirs = ours.clone();
    let mut marked = ours.clone();
    vault.archive(&mut ours, "zero", days_ago(6), None);
    for (notes, days) in [("first", 5), ("second", 4)] {
        vault.archive(&mut ours, notes, days_ago(days), None);
        vault.archive(&mut theirs, notes, days_ago(days), None);
        let item = Some(marker("a value long enough to matter", None));
        vault.archive(&mut marked, notes, days_ago(days), item);
    }

    let with_limit = |db: &Database, limit: isize| {
        let mut db = db.clone();
        db.meta.history_max_size = Some(limit);
        db
    };
    let ahead = |limit: isize| {
        merge::diff(&with_limit(&ours, limit), &with_limit(&theirs, limit)).local_history_ahead
    };
    let limits: Vec<isize> = (0..100_000).collect();
    let limit = limits[limits.partition_point(|limit| !ahead(*limit))];
    assert!(ahead(limit) && !ahead(limit - 1), "a limit at the edge");

    let (ours, theirs, marked) = (
        with_limit(&ours, limit),
        with_limit(&theirs, limit),
        with_limit(&marked, limit),
    );
    assert_eq!(reports(&ours, &marked), reports(&ours, &theirs));
}

/// (a) and (b): a marker one copy holds survives when the other copy wins,
/// whether it wins by being newer or because the user picked it.
#[test]
fn markers_survive_when_the_other_side_wins() {
    for place in PLACES {
        for marked in [Side::Local, Side::Remote] {
            for picked in [false, true] {
                for at in [Some(days_ago(2)), None] {
                    let how = if picked { "picked" } else { "newer" };
                    let what = format!(
                        "{place:?} marked on {marked:?}, other side {how}, item time {at:?}"
                    );
                    let (base, vault) = vault();
                    let (mut local, mut remote) = (base.clone(), base);
                    let (marked_copy, winning_copy) = match marked {
                        Side::Local => (&mut local, &mut remote),
                        Side::Remote => (&mut remote, &mut local),
                    };
                    let item = marker("sap", at);
                    vault.mark(marked_copy, place, item.clone());
                    if picked {
                        vault.edit(marked_copy, place, "here", base_time());
                        vault.edit(winning_copy, place, "carol", base_time());
                    } else {
                        vault.edit(winning_copy, place, "carol", days_ago(0));
                    }

                    let merged = merge(&local, &remote, other(marked));
                    assert_eq!(vault.edited(&merged, place), "carol", "{what}: who won");
                    assert_eq!(vault.held(&merged, place), Some(item), "{what}");
                }
            }
        }
    }
}

/// How the object carrying two marker values is settled: not at all,
/// because only the markers differ, or by one copy's newer edit.
const SETTLED: [Option<Side>; 3] = [None, Some(Side::Local), Some(Side::Remote)];

/// Two values under one marker, `ours` on the local copy and `theirs` on the
/// remote one. Returns the marker the merge kept, after checking that the
/// edit, where there was one, came from where it was made.
fn merge_two_values(
    place: Place,
    settled: Option<Side>,
    ours: CustomDataItem,
    theirs: CustomDataItem,
) -> Option<CustomDataItem> {
    let (base, vault) = vault();
    let (mut local, mut remote) = (base.clone(), base);
    vault.mark(&mut local, place, ours);
    vault.mark(&mut remote, place, theirs);
    match settled {
        Some(Side::Local) => vault.edit(&mut local, place, "carol", days_ago(0)),
        Some(Side::Remote) => vault.edit(&mut remote, place, "carol", days_ago(0)),
        None => assert_nothing_reported(&format!("{place:?}"), &local, &remote),
    }
    let merged = merge(&local, &remote, Side::Local);
    if settled.is_some() {
        assert_eq!(vault.edited(&merged, place), "carol", "{place:?}");
    }
    vault.held(&merged, place)
}

/// (c): two values, two item times, and the later one stays, whichever copy
/// holds it and whichever copy won the object. A time nobody could have
/// written yet decides nothing, and the remote copy's value stays. A few
/// minutes ahead is clock drift between two machines, the same allowance
/// every other time in a merge gets, and still counts.
#[test]
fn newer_marker_wins_by_item_time() {
    let older = marker("sap", Some(days_ago(2)));
    let newer = marker("hana", Some(days_ago(1)));
    let future = marker("bw", Some(days_ago(-365)));
    let drifted = marker("bw", Some(days_ago(0) + TimeDelta::minutes(5)));
    for place in PLACES {
        for settled in SETTLED {
            let what = format!("{place:?}, settled by {settled:?}");
            assert_eq!(
                merge_two_values(place, settled, newer.clone(), older.clone()),
                Some(newer.clone()),
                "{what}: the newer one here"
            );
            assert_eq!(
                merge_two_values(place, settled, older.clone(), newer.clone()),
                Some(newer.clone()),
                "{what}: the newer one there"
            );
            assert_eq!(
                merge_two_values(place, settled, future.clone(), older.clone()),
                Some(older.clone()),
                "{what}: a future time here"
            );
            assert_eq!(
                merge_two_values(place, settled, drifted.clone(), older.clone()),
                Some(drifted.clone()),
                "{what}: a drifted clock here"
            );
        }
    }
}

/// (d): two values and no item time on either side: the remote copy's
/// stays.
#[test]
fn remote_marker_wins_without_item_time() {
    let (sap, hana) = (marker("sap", None), marker("hana", None));
    for place in PLACES {
        for settled in SETTLED {
            let what = format!("{place:?}, settled by {settled:?}");
            assert_eq!(
                merge_two_values(place, settled, sap.clone(), hana.clone()),
                Some(hana.clone()),
                "{what}"
            );
            assert_eq!(
                merge_two_values(place, settled, hana.clone(), sap.clone()),
                Some(sap.clone()),
                "{what}"
            );
        }
    }
}

/// Two values and an item time on one side only. That is what a KeePassXC
/// save leaves behind, since it drops these times: the remote copy's value
/// stays, dated or not.
#[test]
fn remote_marker_wins_when_one_side_lacks_item_time() {
    let dated = marker("sap", Some(days_ago(1)));
    let undated = marker("hana", None);
    for place in PLACES {
        for settled in SETTLED {
            let what = format!("{place:?}, settled by {settled:?}");
            assert_eq!(
                merge_two_values(place, settled, dated.clone(), undated.clone()),
                Some(undated.clone()),
                "{what}: dated here"
            );
            assert_eq!(
                merge_two_values(place, settled, undated.clone(), dated.clone()),
                Some(dated.clone()),
                "{what}: dated there"
            );
        }
    }
}

//! Session-scoped log of entries that changed locally as a result of a
//! sync operation. Surfaced in Settings → Sync as a "Recent activity"
//! section so the user can see, after the fact, what flowed in from
//! remote — useful both for debugging unexpected merges and for the
//! ordinary "what did the sync just do?" sanity check.
//!
//! ## Why in-memory only
//!
//! Entry titles are sensitive — they're part of the encrypted vault
//! payload on disk. Persisting them in plaintext to `app-support/`
//! would be a Privacy regression even with a small cap, so we
//! intentionally keep this log session-scoped: it clears on lock,
//! on disconnect, and on app restart. The trade-off is acceptable
//! because the log's value is operational ("what just happened?"),
//! not auditable history.

use std::collections::HashMap;

use chrono::{DateTime, Local};

use crate::keepass::merge::{ConflictReport, Side};

/// Cap on retained entries. Older ones are dropped when this is exceeded —
/// per-vault sessions don't realistically accumulate more than this
/// in normal use, so the cap is a defence against pathological cases.
pub const MAX_SYNC_HISTORY: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncChangeKind {
    /// Remote had an entry that didn't exist locally. Now it does.
    AddedFromRemote,
    /// Both sides had the entry but remote's `last_modification` was
    /// strictly newer, so the merge replaced the local copy silently.
    UpdatedFromRemote,
    /// User saw the Conflict overlay and clicked "Keep remote" for this
    /// entry — local copy was overwritten with remote.
    ResolvedKeptRemote,
    /// User saw the Conflict overlay and clicked "Keep local". The local
    /// DB didn't actually change, but the divergence itself is worth
    /// surfacing so the user can trace their own decision later.
    ResolvedKeptLocal,
}

#[derive(Clone, PartialEq, Eq)]
pub struct SyncHistoryEntry {
    pub at: DateTime<Local>,
    pub kind: SyncChangeKind,
    /// Snapshot of the entry's title at sync time. Future edits to the
    /// entry won't rewrite the log line — that's intentional, the log
    /// records what happened, not what the entry looks like now.
    pub entry_title: String,
}

impl std::fmt::Debug for SyncHistoryEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SyncHistoryEntry")
            .field("at", &self.at)
            .field("kind", &self.kind)
            .field("title_chars", &self.entry_title.chars().count())
            .finish()
    }
}

/// Turn a merge report (plus the resolution picks, empty for the silent
/// auto-merge path) into the set of history entries that should be
/// appended. Pure function — no `AppState`, fully unit-testable.
pub fn entries_from_report(
    report: &ConflictReport,
    picks: &HashMap<String, Side>,
    now: DateTime<Local>,
) -> Vec<SyncHistoryEntry> {
    let mut out: Vec<SyncHistoryEntry> = Vec::new();

    for view in &report.remote_only {
        out.push(SyncHistoryEntry {
            at: now,
            kind: SyncChangeKind::AddedFromRemote,
            entry_title: view.title.clone(),
        });
    }

    for resolved in &report.auto_resolved {
        // Local-wins auto-resolves don't change the local DB — skip them
        // to keep the log focused on entries the user might want to
        // verify. (A future "show all divergences" toggle could surface
        // local-wins for completeness; deferred.)
        if matches!(resolved.winner, Side::Remote) {
            out.push(SyncHistoryEntry {
                at: now,
                kind: SyncChangeKind::UpdatedFromRemote,
                entry_title: resolved.remote.title.clone(),
            });
        }
    }

    for conflict in &report.conflicts {
        // Default-to-Local mirrors the picks-map initialisation in
        // `handle_remote_conflict_for` — a missing key means the user
        // didn't move the toggle off the default, which is `Local`.
        // Title comes from the *chosen* side: if the user kept remote
        // and remote had a different title, the log line should show
        // the title the entry actually has post-merge.
        let (kind, entry_title) = match picks.get(&conflict.id).copied().unwrap_or(Side::Local) {
            Side::Remote => (
                SyncChangeKind::ResolvedKeptRemote,
                conflict.remote.title.clone(),
            ),
            Side::Local => (
                SyncChangeKind::ResolvedKeptLocal,
                conflict.local.title.clone(),
            ),
        };
        out.push(SyncHistoryEntry {
            at: now,
            kind,
            entry_title,
        });
    }

    out
}

/// Append a batch and enforce the cap. Drops the oldest items if the
/// total would exceed `MAX_SYNC_HISTORY`.
pub fn append_capped(log: &mut Vec<SyncHistoryEntry>, mut new_entries: Vec<SyncHistoryEntry>) {
    if new_entries.is_empty() {
        return;
    }
    log.append(&mut new_entries);
    if log.len() > MAX_SYNC_HISTORY {
        let drop = log.len() - MAX_SYNC_HISTORY;
        log.drain(0..drop);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keepass::merge::{AutoResolved, EntryConflict, EntryView};
    use chrono::TimeZone;

    fn view(id: &str, title: &str) -> EntryView {
        EntryView {
            id: id.into(),
            title: title.into(),
            username: String::new(),
            password: String::new(),
            url: String::new(),
            notes: String::new(),
            modified: None,
            tags: Vec::new(),
            custom_data: HashMap::new(),
            custom_fields: Vec::new(),
            autotype: None,
            foreground_color: None,
            background_color: None,
            override_url: None,
        }
    }

    fn fixed_now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 5, 12, 9, 30, 0).unwrap()
    }

    #[test]
    fn history_debug_omits_entry_title() {
        let title = "sensitive-entry-title";
        let entry = SyncHistoryEntry {
            at: fixed_now(),
            kind: SyncChangeKind::AddedFromRemote,
            entry_title: title.into(),
        };

        let rendered = format!("{entry:?}");

        assert!(!rendered.contains(title));
        assert!(rendered.contains("AddedFromRemote"));
    }

    #[test]
    fn remote_only_yields_added() {
        let report = ConflictReport {
            remote_only: vec![view("a", "Bank")],
            ..Default::default()
        };
        let entries = entries_from_report(&report, &HashMap::new(), fixed_now());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, SyncChangeKind::AddedFromRemote);
        assert_eq!(entries[0].entry_title, "Bank");
    }

    #[test]
    fn auto_resolved_only_records_remote_winners() {
        let report = ConflictReport {
            auto_resolved: vec![
                AutoResolved {
                    id: "x".into(),
                    winner: Side::Remote,
                    remote: view("x", "Email"),
                },
                AutoResolved {
                    id: "y".into(),
                    winner: Side::Local,
                    remote: view("y", "Should-not-appear"),
                },
            ],
            ..Default::default()
        };
        let entries = entries_from_report(&report, &HashMap::new(), fixed_now());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, SyncChangeKind::UpdatedFromRemote);
        assert_eq!(entries[0].entry_title, "Email");
    }

    #[test]
    fn conflicts_branch_on_pick() {
        // Use deliberately divergent titles so a regression that picks
        // the wrong side surfaces — the previous version of this test
        // had local==remote and silently passed even when the title
        // was sourced from the wrong column.
        let report = ConflictReport {
            conflicts: vec![
                EntryConflict {
                    id: "c1".into(),
                    local: view("c1", "Alpha (local)"),
                    remote: view("c1", "Alpha (remote)"),
                    fields: Vec::new(),
                },
                EntryConflict {
                    id: "c2".into(),
                    local: view("c2", "Beta (local)"),
                    remote: view("c2", "Beta (remote)"),
                    fields: Vec::new(),
                },
            ],
            ..Default::default()
        };
        let picks = HashMap::from([
            ("c1".to_string(), Side::Remote),
            ("c2".to_string(), Side::Local),
        ]);
        let entries = entries_from_report(&report, &picks, fixed_now());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, SyncChangeKind::ResolvedKeptRemote);
        assert_eq!(entries[0].entry_title, "Alpha (remote)");
        assert_eq!(entries[1].kind, SyncChangeKind::ResolvedKeptLocal);
        assert_eq!(entries[1].entry_title, "Beta (local)");
    }

    #[test]
    fn missing_pick_defaults_to_local() {
        let report = ConflictReport {
            conflicts: vec![EntryConflict {
                id: "c1".into(),
                local: view("c1", "Default"),
                remote: view("c1", "Default"),
                fields: Vec::new(),
            }],
            ..Default::default()
        };
        let entries = entries_from_report(&report, &HashMap::new(), fixed_now());
        assert_eq!(entries[0].kind, SyncChangeKind::ResolvedKeptLocal);
    }

    #[test]
    fn append_capped_drops_oldest() {
        let mut log: Vec<SyncHistoryEntry> = (0..(MAX_SYNC_HISTORY - 2))
            .map(|i| SyncHistoryEntry {
                at: fixed_now(),
                kind: SyncChangeKind::AddedFromRemote,
                entry_title: format!("old-{i}"),
            })
            .collect();
        let new = (0..5)
            .map(|i| SyncHistoryEntry {
                at: fixed_now(),
                kind: SyncChangeKind::AddedFromRemote,
                entry_title: format!("new-{i}"),
            })
            .collect::<Vec<_>>();
        append_capped(&mut log, new);
        assert_eq!(log.len(), MAX_SYNC_HISTORY);
        // The first three "old-" entries should have been dropped (we had
        // MAX-2 + 5 = MAX+3, so 3 drop from the front).
        assert_eq!(log[0].entry_title, "old-3");
        assert!(log.last().unwrap().entry_title.starts_with("new-"));
    }

    #[test]
    fn empty_batch_is_noop() {
        let mut log = vec![SyncHistoryEntry {
            at: fixed_now(),
            kind: SyncChangeKind::AddedFromRemote,
            entry_title: "keep".into(),
        }];
        append_capped(&mut log, Vec::new());
        assert_eq!(log.len(), 1);
    }
}

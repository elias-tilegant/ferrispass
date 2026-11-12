//! The order of the quick access panel's list.
//!
//! With something typed, the panel lists what [`rank`] lists, in the same
//! order. With the search empty, [`rank`] lists nothing, and the panel shows
//! what the user is most likely after without typing: the favorites, then
//! the entries used recently, then the rest of the vault. An entry comes up
//! once, in the first of those groups it belongs to, so a favorite used a
//! minute ago stands among the favorites. Deleted entries never show up.
//!
//! The groups carry the tiers [`rank`] uses for them, `Favorite`,
//! `RecentlyUsed` and `Text`, so the panel can tell them apart; the list
//! shows no headings for them yet.

use std::collections::HashMap;

use chrono::{DateTime, Local};

use super::by_title;
use super::rank::{RankInput, Ranked, Tier, rank};
use crate::domain::{VaultEntry, VaultSnapshot};

/// The panel's list for `query`: [`rank`]'s order for a query with words,
/// favorites, then recently used entries, then the rest for an empty one.
/// Pure: it reads the snapshot and the times of last use, and nothing else.
pub fn rank_for_panel<'a>(
    snapshot: &'a VaultSnapshot,
    query: &'a str,
    last_used: &'a HashMap<String, DateTime<Local>>,
) -> Vec<Ranked<'a>> {
    if !query.trim().is_empty() {
        return rank(RankInput::new(snapshot, query, last_used));
    }
    let live = snapshot.live_entries();
    let ranked = |entry: &'a VaultEntry, tier: Tier| Ranked {
        entry,
        tier,
        score: 0,
    };

    let mut favorites: Vec<&VaultEntry> = live.iter().copied().filter(|e| e.starred).collect();
    favorites.sort_by(|a, b| by_title(a, b));

    let mut recent: Vec<&VaultEntry> = live
        .iter()
        .copied()
        .filter(|entry| !entry.starred && last_used.contains_key(&entry.id))
        .collect();
    // The most recent first; the title settles two uses at the same moment.
    recent.sort_by(|a, b| {
        last_used
            .get(&b.id)
            .cmp(&last_used.get(&a.id))
            .then_with(|| by_title(a, b))
    });

    let mut rest: Vec<&VaultEntry> = live
        .iter()
        .copied()
        .filter(|entry| !entry.starred && !last_used.contains_key(&entry.id))
        .collect();
    rest.sort_by(|a, b| by_title(a, b));

    favorites
        .into_iter()
        .map(|entry| ranked(entry, Tier::Favorite))
        .chain(
            recent
                .into_iter()
                .map(|entry| ranked(entry, Tier::RecentlyUsed)),
        )
        .chain(rest.into_iter().map(|entry| ranked(entry, Tier::Text)))
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::domain::VaultGroup;

    fn entry(id: &str, title: &str) -> VaultEntry {
        VaultEntry {
            id: id.to_string(),
            title: title.to_string(),
            ..VaultEntry::default()
        }
    }

    fn starred(entry: VaultEntry) -> VaultEntry {
        VaultEntry {
            starred: true,
            ..entry
        }
    }

    /// A vault with `entries` at its root and `deleted` in its Recycle Bin.
    fn vault(entries: Vec<VaultEntry>, deleted: Vec<VaultEntry>) -> VaultSnapshot {
        let bin = VaultGroup {
            id: "bin".to_string(),
            name: "Recycle Bin".to_string(),
            in_recycle_bin: true,
            entries: deleted
                .into_iter()
                .map(|entry| VaultEntry {
                    in_recycle_bin: true,
                    ..entry
                })
                .collect(),
            ..VaultGroup::default()
        };
        VaultSnapshot {
            recycle_bin_id: Some("bin".to_string()),
            ..VaultSnapshot::new(VaultGroup {
                id: "root".to_string(),
                name: "Root".to_string(),
                groups: vec![bin],
                entries,
                ..VaultGroup::default()
            })
        }
    }

    fn rows<'a>(ranked: &[Ranked<'a>]) -> Vec<(&'a str, Tier)> {
        ranked
            .iter()
            .map(|ranked| (ranked.entry.id.as_str(), ranked.tier))
            .collect()
    }

    #[test]
    fn an_empty_search_lists_favourites_then_recent_then_the_rest() {
        let snapshot = vault(
            vec![
                entry("zeta", "Zeta wiki"),
                starred(entry("salesforce", "Salesforce")),
                starred(entry("bank", "Bank")),
                entry("nhq", "NHQ 100 QAS"),
                entry("alpha", "Alpha mail"),
                entry("nhp", "NHP 100 PRD"),
            ],
            vec![starred(entry("deleted", "Deleted favorite"))],
        );
        let now = Local::now();
        let last_used = HashMap::from([
            // A favorite used a minute ago stays among the favorites, once.
            ("salesforce".to_string(), now - Duration::minutes(1)),
            ("nhp".to_string(), now - Duration::minutes(30)),
            ("nhq".to_string(), now - Duration::minutes(5)),
            ("deleted".to_string(), now),
        ]);

        for query in ["", "   "] {
            assert_eq!(
                rows(&rank_for_panel(&snapshot, query, &last_used)),
                [
                    ("bank", Tier::Favorite),
                    ("salesforce", Tier::Favorite),
                    ("nhq", Tier::RecentlyUsed),
                    ("nhp", Tier::RecentlyUsed),
                    ("alpha", Tier::Text),
                    ("zeta", Tier::Text),
                ],
                "query {query:?}"
            );
        }

        // Something typed: the same list as the search everywhere else.
        for query in ["nh", "100", "sales", "nhp 100"] {
            assert_eq!(
                rows(&rank_for_panel(&snapshot, query, &last_used)),
                rows(&rank(RankInput::new(&snapshot, query, &last_used))),
                "query {query:?}"
            );
        }
        assert!(!rank_for_panel(&snapshot, "nh", &last_used).is_empty());
    }

    #[test]
    fn an_empty_vault_lists_nothing() {
        let snapshot = vault(Vec::new(), vec![entry("deleted", "Deleted")]);
        assert!(rank_for_panel(&snapshot, "", &HashMap::new()).is_empty());
    }
}

//! The order search results are listed in.
//!
//! [`super`] decides whether an entry matches a query and how well its text
//! matches. This module sorts the matches into four tiers, so the entry the
//! user is plainly asking for comes first however well other entries match
//! the words:
//!
//! 1. **Exact identifier.** The query is the SID and client of an SAP system,
//!    "nhp 100" for NHP 100, or the whole title of an entry. Case and runs of
//!    whitespace do not matter. A SID alone is not an identifier: "nhp" names
//!    every NHP system, so it ranks like any other text.
//! 2. **Favourites** whose text matches.
//! 3. **Recently used** entries whose text matches, the most recent first.
//! 4. **Text quality**: every other match, best match first, in exactly the
//!    order the text search alone gives.
//!
//! Inside a tier, favourites come first, then recently used entries (the
//! most recent first), then entries by title, then by id. The text score
//! only orders the last tier. The first three hold entries the user has
//! singled out by naming, starring or using them; they keep an order the
//! user can learn instead of reshuffling with every letter typed.
//!
//! An entry can be an exact identifier without its text matching at all: an
//! entry titled "Nordhafen production" with the custom fields `SAP_SID` NHP
//! and `SAP_CLIENT` 100 is found by "nhp 100". The SID and client come from
//! [`kinds::classify`], so a title like "NHP 100 PRD" counts the same as the
//! fields. Custom fields are still never search text, and a protected one
//! counts for nothing (see [`kind_without_protected_fields`]).
//!
//! Two results can stand for the same system, typically the same SID and
//! client at two customers. [`tie_count`] says how many results share the
//! first row's identifier or title.

use std::collections::HashMap;

use chrono::{DateTime, Local};

use crate::app::kinds::{self, EntryKind, SapSystem};
use crate::domain::{VaultEntry, VaultSnapshot};

use super::{TextQuery, by_title};

/// What [`rank`] needs to order the results of one query.
#[derive(Clone, Copy)]
pub struct RankInput<'a> {
    pub snapshot: &'a VaultSnapshot,
    pub query: &'a str,
    /// When each entry was last used in this session, by entry id.
    pub last_used: &'a HashMap<String, DateTime<Local>>,
    /// Works out the kind an exact identifier is read from.
    pub kind_of: fn(&VaultEntry) -> EntryKind,
}

impl<'a> RankInput<'a> {
    /// The input the app ranks with, reading kinds through
    /// [`kind_without_protected_fields`].
    pub fn new(
        snapshot: &'a VaultSnapshot,
        query: &'a str,
        last_used: &'a HashMap<String, DateTime<Local>>,
    ) -> Self {
        Self {
            snapshot,
            query,
            last_used,
            kind_of: kind_without_protected_fields,
        }
    }
}

/// The tiers, best first. The derived order is the ranking order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    ExactIdentifier,
    Favorite,
    RecentlyUsed,
    Text,
}

/// One result: the entry, its tier and how well its text matched.
#[derive(Clone, Copy, Debug)]
pub struct Ranked<'a> {
    pub entry: &'a VaultEntry,
    pub tier: Tier,
    /// The text score; 0 for an entry found only by its identifier.
    pub score: u32,
}

/// Every live entry that matches the query, in the order the app lists them.
/// Deleted entries never show up, not even by their identifier.
pub fn rank(input: RankInput<'_>) -> Vec<Ranked<'_>> {
    let Some(mut text) = TextQuery::new(input.query) else {
        return Vec::new();
    };
    let query = normalize(input.query);

    let mut ranked: Vec<Ranked<'_>> = Vec::new();
    for entry in input.snapshot.live_entries() {
        let score = text.score(entry);
        let tier = if exact_identifier(entry, input.kind_of, &query) {
            Tier::ExactIdentifier
        } else if score.is_none() {
            continue;
        } else if entry.starred {
            Tier::Favorite
        } else if input.last_used.contains_key(&entry.id) {
            Tier::RecentlyUsed
        } else {
            Tier::Text
        };
        ranked.push(Ranked {
            entry,
            tier,
            score: score.unwrap_or(0),
        });
    }

    let last_used = input.last_used;
    ranked.sort_by(|a, b| {
        a.tier
            .cmp(&b.tier)
            .then_with(|| b.entry.starred.cmp(&a.entry.starred))
            // `None` sorts before any time, so comparing b with a puts the
            // most recent use first and entries never used last.
            .then_with(|| last_used.get(&b.entry.id).cmp(&last_used.get(&a.entry.id)))
            .then_with(|| match a.tier {
                Tier::Text => b.score.cmp(&a.score),
                _ => std::cmp::Ordering::Equal,
            })
            .then_with(|| by_title(a.entry, b.entry))
    });
    ranked
}

/// How many results stand for the same thing as the first row, the first row
/// included: the same SID and client, or the same title, compared the way
/// identifiers are. 0 for no results, 1 when the first row is unique, so a
/// footer can warn when the row Return would act on has a twin.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the main window does not show ties; the quick access panel will"
    )
)]
pub fn tie_count(ranked: &[Ranked<'_>]) -> usize {
    let Some(first) = ranked.first() else {
        return 0;
    };
    let identity = |entry: &VaultEntry| {
        (
            normalize(&entry.title),
            system_identifier(&kind_without_protected_fields(entry)),
        )
    };
    let (title, system) = identity(first.entry);
    ranked
        .iter()
        .filter(|other| {
            let (other_title, other_system) = identity(other.entry);
            other_title == title || (system.is_some() && other_system == system)
        })
        .count()
}

/// The kind an identifier is read from: [`kinds::classify`] on a copy of the
/// entry without its protected custom fields. A protected `SAP_SID` or
/// `SAP_CLIENT` then counts as not set. Otherwise a search would tell which
/// entry holds a protected value: type guesses and watch which entry turns
/// up as an exact match.
pub fn kind_without_protected_fields(entry: &VaultEntry) -> EntryKind {
    if !entry.custom_fields.iter().any(|field| field.protected) {
        return kinds::classify(entry, None);
    }
    kinds::classify(&identifier_view(entry), None)
}

/// A copy of `entry` without its notes and its protected custom fields.
/// It is built field by field, so neither is ever copied, not even for a
/// moment. The pattern lists every field on purpose: a new one does not
/// compile here until someone decides whether an identifier may read it.
fn identifier_view(entry: &VaultEntry) -> VaultEntry {
    let VaultEntry {
        id,
        title,
        username,
        url,
        notes: _,
        has_password,
        password_length,
        has_otp,
        updated,
        tags,
        starred,
        favicon,
        strength,
        group_path,
        in_recycle_bin,
        auto_type_enabled,
        auto_type_windows,
        custom_fields,
    } = entry;
    VaultEntry {
        id: id.clone(),
        title: title.clone(),
        username: username.clone(),
        url: url.clone(),
        notes: String::new(),
        has_password: *has_password,
        password_length: *password_length,
        has_otp: *has_otp,
        updated: updated.clone(),
        tags: tags.clone(),
        starred: *starred,
        favicon: favicon.clone(),
        strength: *strength,
        group_path: group_path.clone(),
        in_recycle_bin: *in_recycle_bin,
        auto_type_enabled: *auto_type_enabled,
        auto_type_windows: auto_type_windows.clone(),
        custom_fields: custom_fields
            .iter()
            .filter(|field| !field.protected)
            .cloned()
            .collect(),
    }
}

/// Whether `query`, already passed through [`normalize`], names `entry`
/// exactly: its whole title, or the SID and client of the SAP system it is.
fn exact_identifier(
    entry: &VaultEntry,
    kind_of: fn(&VaultEntry) -> EntryKind,
    query: &str,
) -> bool {
    if normalize(&entry.title) == query {
        return true;
    }
    // A SID and a client are at least two words. A one-word query can never
    // name a system, so the kind need not be worked out for it.
    if !query.contains(' ') {
        return false;
    }
    system_identifier(&kind_of(entry)).is_some_and(|system| system == query)
}

/// "nhp 100" for an SAP system with SID NHP and client 100, in the form
/// [`normalize`] gives. `None` unless the kind has both.
fn system_identifier(kind: &EntryKind) -> Option<String> {
    let EntryKind::Sap(SapSystem {
        sid: Some(sid),
        client: Some(client),
        ..
    }) = kind
    else {
        return None;
    };
    Some(normalize(&format!("{} {client}", sid.as_str())))
}

/// Lowercase words joined by single spaces, so neither case nor runs of
/// whitespace decide whether two texts are the same.
fn normalize(text: &str) -> String {
    text.split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests;

//! Local fuzzy entry search.
//!
//! Matches an entry's title, URL host, full URL, username, tags, and the
//! names of its ancestor groups (its `group_path`) — so searching a folder
//! name like "acme" surfaces every entry filed under that group and its
//! subgroups. Metadata-only: never reads `password`, `notes`, or
//! `custom_fields` — those are either secret or noisy enough that searching
//! them would surface results the user didn't expect. The opposite of a
//! server-side index: in-memory, per-keystroke, no plaintext metadata ever
//! touches disk.
//!
//! Match classes by needle length, ordered by priority:
//!
//! | needle chars | exact | prefix | substring | typo  | fuzzy |
//! |--------------|:-----:|:------:|:---------:|:-----:|:-----:|
//! | 1-2          |  ✓    |   ✓    |     —     |   —   |   —   |
//! | 3            |  ✓    |   ✓    |     ✓     |   —   |   —   |
//! | 4..=8        |  ✓    |   ✓    |     ✓     | 1 ed  |   ✓   |
//! | 9+           |  ✓    |   ✓    |     ✓     | 2 ed  |   ✓   |
//!
//! Two complementary approximate-match passes sit between substring and
//! fuzzy:
//!
//! - **Typo / edit-distance** catches *substitutions* — `tilegane` → the
//!   term `tilegant`, `microsotf` → `microsoft`. Runs **per term** (split
//!   on non-alphanumeric chars): edit-distance against the whole title
//!   `"elias tilegant"` would never fit in budget; against the single
//!   term `"tilegant"` it does. Budget scales with needle length:
//!   `4..=8` chars get 1 edit, `9+` chars get 2.
//!
//! - **Nucleo fuzzy** catches *deletions* — `gthb` → `github`, `tlgnt` →
//!   `tilegant`. fzf-style subsequence, not Levenshtein, so it can NOT
//!   handle substitutions on its own — that's the gap the typo pass
//!   fills. Fuzzy additionally requires (a) a score floor relative to
//!   the needle length and (b) a span gate — matched indices in the
//!   haystack must lie within `needle_len * 2` positions of each other.
//!
//! Both approximate passes are disabled against the full URL field —
//! URLs are too long/structured to be useful approximate haystacks.
//!
//! No multi-token "fallback" with permitted misses: if any token fails to
//! match, the entry drops out. False matches feel worse than empty results
//! when the user can just edit the query.

use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::domain::{VaultEntry, VaultSnapshot};

const SCORE_EXACT: u32 = 1000;
const SCORE_PREFIX: u32 = 700;
const SCORE_SUBSTRING: u32 = 400;
/// Typo-pass score for distance 1. Stays below `SCORE_SUBSTRING` so a real
/// substring hit always wins, but above `FUZZY_MAX` so a clean typo
/// outranks a stretchy fzf subsequence on the same field.
const SCORE_TYPO_BASE: u32 = 380;
/// Each extra edit costs this much off `SCORE_TYPO_BASE`. With base 380
/// and decrement 30, distance 2 lands at 350 — still above `FUZZY_MAX`.
const SCORE_TYPO_PER_EDIT: u32 = 30;
/// Hard ceiling on the fuzzy contribution after capping. Stays below
/// `SCORE_TYPO_BASE` so a fuzzy hit can never outrank a typo hit on the
/// same field.
const FUZZY_MAX: u32 = 300;
/// Minimum needle length for any approximate matching (typo or fuzzy).
/// Below this, both degenerate to "matches anything" — useless noise.
const APPROX_MIN_NEEDLE: usize = 4;
/// Maximum needle length that's treated as "too short for substring".
/// 1-2 char needles only match via exact/prefix; their substring hits
/// flood the result list with incidental letter pairs.
const SHORT_TOKEN_LEN: usize = 2;

#[derive(Clone, Copy)]
enum Field {
    Title,
    UrlHost,
    UrlFull,
    Username,
    Tag,
    Group,
}

impl Field {
    fn weight(self) -> f32 {
        match self {
            Field::Title => 1.00,
            Field::UrlHost => 0.85,
            Field::UrlFull => 0.65,
            Field::Username => 0.55,
            // Folder/group names: a deliberate categorisation signal —
            // slightly above a free-form tag, still below any direct
            // field hit so an entry literally named for the query wins.
            Field::Group => 0.45,
            Field::Tag => 0.40,
        }
    }

    /// Whether approximate matching (both typo and fuzzy) is allowed for
    /// this field. `UrlFull` opts out: scheme + host + path + query
    /// strings are long enough that nucleo finds some subsequence in
    /// nearly every entry, and per-term edit-distance on URL fragments
    /// would also produce noise.
    fn allow_approximate(self) -> bool {
        !matches!(self, Field::UrlFull)
    }
}

pub(crate) fn ranked_entries<'a>(snapshot: &'a VaultSnapshot, query: &str) -> Vec<&'a VaultEntry> {
    let tokens: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if tokens.is_empty() {
        return Vec::new();
    }

    let mut matcher = Matcher::new(Config::DEFAULT);
    let entries = snapshot.entries_recursive();
    let mut scored: Vec<(u32, &VaultEntry)> = Vec::new();
    for entry in &entries {
        let haystacks = build_haystacks(entry);
        if let Some(score) = score_entry(&tokens, &haystacks, &mut matcher) {
            scored.push((score, *entry));
        }
    }

    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.title.to_lowercase().cmp(&b.1.title.to_lowercase()))
            .then_with(|| a.1.id.cmp(&b.1.id))
    });

    scored.into_iter().map(|(_, e)| e).collect()
}

struct Haystacks {
    title: String,
    url_host: Option<String>,
    url_full: String,
    username: String,
    tags: Vec<String>,
    /// Lowercased ancestor group names (the entry's `group_path`). Each
    /// segment is matched independently — like tags — so an entry in
    /// `Customers/Globex/Web` is found by "customers", "globex" *or* "web".
    group_segments: Vec<String>,
}

fn build_haystacks(entry: &VaultEntry) -> Haystacks {
    Haystacks {
        title: entry.title.to_lowercase(),
        url_host: extract_host(&entry.url),
        url_full: entry.url.to_lowercase(),
        username: entry.username.to_lowercase(),
        tags: entry
            .tags
            .iter()
            .map(|t| t.to_lowercase())
            .collect::<Vec<String>>(),
        group_segments: entry
            .group_path
            .iter()
            .map(|g| g.to_lowercase())
            .collect::<Vec<String>>(),
    }
}

/// Pulls the host portion out of an entry URL. KeePass stores URLs in
/// many shapes — `https://github.com/foo`, `github.com/foo`,
/// `mailto:user@x` — and the user expects "github" to match both of the
/// first two. `Url::parse` rejects schemeless inputs, so we retry with a
/// synthetic `https://` prefix.
fn extract_host(url: &str) -> Option<String> {
    if url.trim().is_empty() {
        return None;
    }
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some(h) = parsed.host_str() {
            return Some(h.to_lowercase());
        }
    }
    url::Url::parse(&format!("https://{url}"))
        .ok()
        .and_then(|u| u.host_str().map(str::to_lowercase))
}

/// Every token must score above 0; otherwise the entry drops.
fn score_entry(tokens: &[String], h: &Haystacks, matcher: &mut Matcher) -> Option<u32> {
    let mut total: u32 = 0;
    for token in tokens {
        let token_score = best_token_score(token, h, matcher);
        if token_score == 0 {
            return None;
        }
        total = total.saturating_add(token_score);
    }
    if total == 0 { None } else { Some(total) }
}

fn best_token_score(token: &str, h: &Haystacks, matcher: &mut Matcher) -> u32 {
    let needle_len = token.chars().count();
    let mut best: u32 = 0;

    best = best.max(weighted(token, &h.title, Field::Title, matcher, needle_len));
    if let Some(host) = h.url_host.as_deref() {
        best = best.max(weighted(token, host, Field::UrlHost, matcher, needle_len));
    }
    best = best.max(weighted(
        token,
        &h.url_full,
        Field::UrlFull,
        matcher,
        needle_len,
    ));
    best = best.max(weighted(
        token,
        &h.username,
        Field::Username,
        matcher,
        needle_len,
    ));
    for tag in &h.tags {
        best = best.max(weighted(token, tag, Field::Tag, matcher, needle_len));
    }
    for segment in &h.group_segments {
        best = best.max(weighted(token, segment, Field::Group, matcher, needle_len));
    }
    best
}

fn weighted(
    token: &str,
    haystack: &str,
    field: Field,
    matcher: &mut Matcher,
    needle_len: usize,
) -> u32 {
    if haystack.is_empty() {
        return 0;
    }
    let raw = if haystack == token {
        SCORE_EXACT
    } else if haystack.starts_with(token) {
        SCORE_PREFIX
    } else if needle_len <= SHORT_TOKEN_LEN {
        return 0;
    } else if haystack.contains(token) {
        SCORE_SUBSTRING
    } else if needle_len < APPROX_MIN_NEEDLE || !field.allow_approximate() {
        return 0;
    } else if let Some(score) = typo_score(token, haystack, needle_len) {
        score
    } else {
        fuzzy_score(token, haystack, matcher, needle_len)
    };
    if raw == 0 {
        return 0;
    }
    ((raw as f32) * field.weight()).round() as u32
}

/// Edit-distance budget by needle length. Matches the policy in the
/// module-level table: short enough that random pairs don't match,
/// generous enough that real typos in long names land.
fn edit_budget(needle_len: usize) -> usize {
    if needle_len >= 9 { 2 } else { 1 }
}

/// Splits `s` on every non-alphanumeric char and yields the non-empty
/// pieces. Used so that typo matching runs term-by-term: edit-distance
/// against `"elias tilegant"` as one blob would never fit budget; against
/// the individual term `"tilegant"` it does. Same logic for host labels
/// (`login.example.com` → [`login`, `example`, `com`]).
fn split_terms(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
}

/// Per-term Damerau-free Levenshtein with a budget. Returns `None` if no
/// term of the haystack is within the edit budget; otherwise the score
/// for the closest term.
fn typo_score(token: &str, haystack: &str, needle_len: usize) -> Option<u32> {
    let budget = edit_budget(needle_len);
    let mut best: Option<usize> = None;
    for term in split_terms(haystack) {
        if let Some(d) = levenshtein_within(token, term, budget) {
            best = Some(match best {
                Some(prev) => prev.min(d),
                None => d,
            });
            if best == Some(0) {
                break;
            }
        }
    }
    let d = best?;
    let penalty = (d as u32).saturating_mul(SCORE_TYPO_PER_EDIT);
    let raw = SCORE_TYPO_BASE.saturating_sub(penalty);
    // Floor above FUZZY_MAX so a typo hit never falls into fuzzy territory.
    Some(raw.max(FUZZY_MAX + 10))
}

/// Classic Levenshtein with row-min cutoff and length-diff early-bail.
/// Returns `Some(distance)` iff `distance <= max`, else `None`. Used only
/// for short strings (needles 4..16 chars, terms typically the same), so
/// the O(n*m) table stays tiny.
fn levenshtein_within(a: &str, b: &str, max: usize) -> Option<usize> {
    let av: Vec<char> = a.chars().collect();
    let bv: Vec<char> = b.chars().collect();
    let n = av.len();
    let m = bv.len();
    if n.abs_diff(m) > max {
        return None;
    }
    if n == 0 {
        return if m <= max { Some(m) } else { None };
    }
    if m == 0 {
        return if n <= max { Some(n) } else { None };
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr: Vec<usize> = vec![0; m + 1];
    for i in 1..=n {
        curr[0] = i;
        let mut row_min = curr[0];
        for j in 1..=m {
            let cost = if av[i - 1] == bv[j - 1] { 0 } else { 1 };
            let v = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
            curr[j] = v;
            if v < row_min {
                row_min = v;
            }
        }
        // Whole-row floor: if every cell in this row already exceeds the
        // budget, no extension can recover. Lets us short-circuit on
        // mismatched strings without filling the full table.
        if row_min > max {
            return None;
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    let d = prev[m];
    if d <= max { Some(d) } else { None }
}

/// Nucleo's `fuzzy_indices` returns both the score and the positions where
/// each needle char matched. We use the positions to enforce a span gate:
/// the matched indices must lie within `needle_len * 2` of each other.
/// That's the robust filter against scattered-subsequence noise — it
/// argues geometrically about the match's compactness instead of relying
/// on score thresholds that drift between nucleo versions.
fn fuzzy_score(token: &str, haystack: &str, matcher: &mut Matcher, needle_len: usize) -> u32 {
    let mut needle_buf: Vec<char> = Vec::new();
    let mut hay_buf: Vec<char> = Vec::new();
    let needle = Utf32Str::new(token, &mut needle_buf);
    let hay = Utf32Str::new(haystack, &mut hay_buf);

    let mut indices: Vec<u32> = Vec::with_capacity(needle_len);
    let raw = match matcher.fuzzy_indices(hay, needle, &mut indices) {
        Some(s) => s,
        None => return 0,
    };

    // Score floor: roughly 12 per matched char. Tight runs at word
    // boundaries score 16+ per char, so this keeps real typo matches
    // (`gthb` → `github`) while dropping low-quality hits.
    let threshold = needle_len.saturating_mul(12).min(u16::MAX as usize) as u16;
    if raw < threshold {
        return 0;
    }

    // Span gate: reject scattered matches even if the raw score happened
    // to clear the floor.
    if let (Some(&first), Some(&last)) = (indices.first(), indices.last()) {
        let span = (last - first) as usize + 1;
        if span > needle_len.saturating_mul(2) {
            return 0;
        }
    }

    (raw as u32).min(FUZZY_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{CustomField, VaultEntry, VaultGroup, VaultSnapshot};

    fn entry(id: &str, title: &str) -> VaultEntry {
        VaultEntry {
            id: id.to_string(),
            title: title.to_string(),
            ..VaultEntry::default()
        }
    }

    fn snapshot(entries: Vec<VaultEntry>) -> VaultSnapshot {
        let root = VaultGroup {
            id: "root".to_string(),
            name: "Root".to_string(),
            entries,
            ..VaultGroup::default()
        };
        VaultSnapshot {
            root,
            ..VaultSnapshot::default()
        }
    }

    fn ids(result: &[&VaultEntry]) -> Vec<String> {
        result.iter().map(|e| e.id.clone()).collect()
    }

    #[test]
    fn empty_query_returns_nothing() {
        let s = snapshot(vec![entry("a", "GitHub")]);
        assert!(ranked_entries(&s, "").is_empty());
        assert!(ranked_entries(&s, "   ").is_empty());
    }

    #[test]
    fn exact_title_beats_other_classes() {
        let s = snapshot(vec![entry("a", "github"), entry("b", "github-cli")]);
        let result = ranked_entries(&s, "github");
        assert_eq!(ids(&result).first().map(String::as_str), Some("a"));
    }

    #[test]
    fn host_search_finds_subdomain_url() {
        let mut e = entry("a", "Work login");
        e.url = "https://login.example.com/path".to_string();
        let s = snapshot(vec![e]);

        assert_eq!(ranked_entries(&s, "example").len(), 1);
        assert_eq!(ranked_entries(&s, "login").len(), 1);
        assert_eq!(ranked_entries(&s, "example.com").len(), 1);
    }

    #[test]
    fn schemeless_url_still_extracts_host() {
        // Vaults often store bare `github.com/foo` without a scheme. The
        // host extraction must handle that — otherwise "github" against
        // such an entry only scores the lower-weight `UrlFull` match.
        let mut with_scheme = entry("a", "");
        with_scheme.url = "https://github.com/repo".to_string();
        let mut without_scheme = entry("b", "");
        without_scheme.url = "github.com/repo".to_string();
        let s = snapshot(vec![with_scheme, without_scheme]);

        let result = ranked_entries(&s, "github");
        // Both entries must show up, with comparable rank — the
        // schemeless one mustn't be silently demoted.
        let result_ids = ids(&result);
        assert!(result_ids.contains(&"a".to_string()));
        assert!(result_ids.contains(&"b".to_string()));
    }

    #[test]
    fn multi_token_matches_across_fields() {
        let mut e = entry("a", "GitHub");
        e.username = "elias".to_string();
        let mut other = entry("b", "GitLab");
        other.username = "bob".to_string();
        let s = snapshot(vec![e, other]);

        let result = ranked_entries(&s, "github eli");
        assert_eq!(ids(&result), vec!["a"]);
    }

    #[test]
    fn nonmatching_token_drops_entry() {
        // No fallback: if even one token fails to match anywhere, the
        // entry drops out. Predictability > recall for a password manager.
        let mut e = entry("a", "GitHub");
        e.username = "elias".to_string();
        let s = snapshot(vec![e]);

        let result = ranked_entries(&s, "github nonexistenttoken");
        assert!(result.is_empty());
    }

    #[test]
    fn strict_pass_filters_correctly_with_multiple_tokens() {
        let mut a = entry("a", "GitHub");
        a.username = "elias".to_string();
        let mut b = entry("b", "Bitbucket");
        b.username = "carol".to_string();
        let s = snapshot(vec![a, b]);

        let result = ranked_entries(&s, "github elias");
        assert_eq!(ids(&result), vec!["a"]);
    }

    #[test]
    fn deletion_typo_matches_via_fuzzy() {
        // `gthb` → `github`: nucleo skips the missing chars in the
        // haystack. Edit-distance rejects this (length diff 2 exceeds
        // budget 1 for a 4-char needle), so it correctly falls through
        // to the nucleo fuzzy pass.
        let s = snapshot(vec![entry("a", "github")]);
        let result = ranked_entries(&s, "gthb");
        assert_eq!(ids(&result), vec!["a"]);
    }

    #[test]
    fn substitution_typo_matches_via_edit_distance() {
        // Nucleo's subsequence matcher can't substitute chars (`githab`
        // can't align with `github` because there's no `a` in the
        // haystack). The Levenshtein pass fills that gap: same length,
        // distance 1, budget 1 → match.
        let s = snapshot(vec![entry("a", "github")]);
        let result = ranked_entries(&s, "githab");
        assert_eq!(ids(&result), vec!["a"]);
    }

    #[test]
    fn long_substitution_typo_uses_distance_two_budget() {
        // 9-char needle gets a budget of 2. `microsotf` → `microsoft` is
        // a transposition that classic Levenshtein scores as 2 edits.
        let s = snapshot(vec![entry("a", "microsoft")]);
        let result = ranked_entries(&s, "microsotf");
        assert_eq!(ids(&result), vec!["a"]);
    }

    #[test]
    fn typo_runs_against_individual_terms() {
        // `tilegane` vs the whole title `elias tilegant` would never fit
        // in budget. Per-term Levenshtein against the second term
        // `tilegant` matches at distance 1.
        let s = snapshot(vec![entry("a", "elias tilegant")]);
        let result = ranked_entries(&s, "tilegane");
        assert_eq!(ids(&result), vec!["a"]);
    }

    #[test]
    fn typo_runs_against_host_labels() {
        // Host labels `[login, example, com]` — `exampla` matches
        // `example` at distance 1.
        let mut e = entry("a", "Work");
        e.url = "https://login.example.com/path".to_string();
        let s = snapshot(vec![e]);

        let result = ranked_entries(&s, "exampla");
        assert_eq!(ids(&result), vec!["a"]);
    }

    #[test]
    fn two_edit_typo_rejected_on_short_needle() {
        // 8-char needle has budget 1. `bilegane` differs from
        // `tilegant` at 2 positions (b/t at start, e/t at end) — must
        // not match.
        let s = snapshot(vec![entry("a", "tilegant")]);
        assert!(ranked_entries(&s, "bilegane").is_empty());
    }

    #[test]
    fn approximate_disabled_on_full_url() {
        // Both typo and fuzzy must be off for the full URL field: it's
        // too long/structured. Construct an entry whose only possible
        // match path is via UrlFull (host has no overlapping chars,
        // title/username/tags empty). The needle would otherwise fuzzy-
        // match a scattered subsequence inside the URL path.
        let mut e = entry("a", "");
        e.url = "https://acme.org/projects/atelier-glance-network-tab".to_string();
        let s = snapshot(vec![e]);

        assert!(ranked_entries(&s, "tilegant").is_empty());
    }

    #[test]
    fn scattered_subsequence_rejected_on_short_fields() {
        // Even on a fuzzy-eligible field (title), a needle whose matched
        // chars sprawl too wide must be rejected by the span gate.
        // Title "alpha-beta-charlie-delta" has `a-b-c-d-e` as a scattered
        // subsequence — match positions ~[0, 6, 11, 19, 22], span 23 vs
        // needle_len*2 = 10. Span gate kicks in.
        let s = snapshot(vec![entry("a", "alpha-beta-charlie-delta")]);
        assert!(ranked_entries(&s, "abcde").is_empty());
    }

    #[test]
    fn short_query_disables_fuzzy() {
        // 3-char needle: no fuzzy, only substring and below. `gth` is a
        // valid fzf subsequence of `growth` but should NOT match — only
        // 4+ char needles get fuzzy.
        let s = snapshot(vec![entry("a", "growth")]);
        assert!(ranked_entries(&s, "gth").is_empty());
    }

    #[test]
    fn two_char_token_only_exact_or_prefix() {
        // `xy` matches `xyz` (prefix) but not `boxy` (substring).
        let s = snapshot(vec![entry("a", "boxy"), entry("b", "xyz")]);
        let result = ranked_entries(&s, "xy");
        assert_eq!(ids(&result), vec!["b"]);
    }

    #[test]
    fn single_char_token_only_exact_or_prefix() {
        let s = snapshot(vec![entry("a", "box"), entry("b", "xyz")]);
        let result = ranked_entries(&s, "x");
        assert_eq!(ids(&result), vec!["b"]);
    }

    #[test]
    fn notes_and_custom_fields_do_not_match() {
        let mut e = entry("a", "Random title");
        e.notes = "secretphrase appears here".to_string();
        e.custom_fields = vec![CustomField {
            key: "k".into(),
            value: "secretphrase".into(),
            protected: false,
        }];
        let s = snapshot(vec![e]);

        assert!(ranked_entries(&s, "secretphrase").is_empty());
    }

    #[test]
    fn password_field_is_not_in_snapshot() {
        // VaultEntry exposes only `has_password` / `password_length`. This
        // test pins the invariant: no query string can reach a password
        // because the snapshot type never holds one.
        let e = entry("a", "Title");
        let s = snapshot(vec![e]);
        assert!(ranked_entries(&s, "hunter2").is_empty());
    }

    #[test]
    fn tags_have_lower_priority_than_title() {
        let by_title = entry("a", "alpha");
        let mut by_tag = entry("b", "Other");
        by_tag.tags = vec!["alpha".into()];
        let s = snapshot(vec![by_title, by_tag]);

        let result = ranked_entries(&s, "alpha");
        assert_eq!(ids(&result), vec!["a", "b"]);
    }

    #[test]
    fn group_path_is_searched() {
        // The entry's folder names are matchable: someone who files
        // everything for a client under one group can pull it all up by the
        // group name even when that name appears in no other field.
        let mut e = entry("a", "Random title");
        e.group_path = vec!["Acme".into(), "Work".into()];
        let s = snapshot(vec![e]);

        assert_eq!(ids(&ranked_entries(&s, "acme")), vec!["a"]);
        assert_eq!(ids(&ranked_entries(&s, "work")), vec!["a"]);
    }

    #[test]
    fn group_name_matches_subgroup_entries() {
        // A mid-path ancestor must match, not just the immediate parent —
        // an entry in `Customers/Globex/Web` is "a Globex entry".
        let mut e = entry("a", "Router admin");
        e.group_path = vec!["Customers".into(), "Globex".into(), "Web".into()];
        let s = snapshot(vec![e]);

        assert_eq!(ids(&ranked_entries(&s, "globex")), vec!["a"]);
    }

    #[test]
    fn group_match_ranks_below_title() {
        // An entry literally titled for the query outranks one that merely
        // lives in a folder of that name.
        let by_title = entry("a", "globex");
        let mut by_group = entry("b", "Other");
        by_group.group_path = vec!["globex".into()];
        let s = snapshot(vec![by_title, by_group]);

        let result = ranked_entries(&s, "globex");
        assert_eq!(ids(&result), vec!["a", "b"]);
    }

    #[test]
    fn group_plus_title_token_combo() {
        // The all-tokens-must-match rule composes group + title for free:
        // "globex vpn" narrows to the VPN entry inside the Globex group.
        let mut vpn = entry("a", "VPN");
        vpn.group_path = vec!["Globex".into()];
        let mut mail = entry("b", "Mail");
        mail.group_path = vec!["Globex".into()];
        let s = snapshot(vec![vpn, mail]);

        assert_eq!(ids(&ranked_entries(&s, "globex vpn")), vec!["a"]);
    }

    #[test]
    fn deterministic_tie_breaker() {
        let s = snapshot(vec![entry("z", "github"), entry("a", "github")]);
        let result = ranked_entries(&s, "github");
        assert_eq!(ids(&result), vec!["a", "z"]);
    }
}

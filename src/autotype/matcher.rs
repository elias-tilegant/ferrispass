//! Score vault entries against a foreground window.
//!
//! KeePassXC's auto-type matches an entry to a window by checking the
//! foreground window's title against the entry's URL hostname and title.
//! We mirror that, lightly:
//!
//! - `https://login.example.com/path` → hostname `example.com` (the
//!   eTLD+1 isn't worth importing PSL data for; full host is fine and
//!   string-contains matching against the title generally absorbs the
//!   subdomain noise).
//! - Each entry is scored. The top score wins. Ties resolve in entry-
//!   id order to keep the result deterministic (so the toast naming
//!   the chosen entry is stable across runs).
//!
//! Why string-contains rather than regex / exact match: browsers
//! decorate window titles inconsistently (`example.com — Google Chrome`
//! vs `example.com - Mozilla Firefox` vs `Sign in to Example`), and
//! sites occasionally embed the host inside a tab title. A substring
//! check absorbs the noise without needing per-site rules.

use crate::autotype::window::ForegroundInfo;
use crate::domain::{VaultEntry, VaultSnapshot};

/// One ranked match. `score` is a relative number; only ordering
/// matters (no calibration against an absolute "good vs. bad"
/// threshold). A score of `0` means no signal — the matcher drops
/// those before returning, so an empty result means "nothing
/// matched".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchedEntry {
    pub id: String,
    pub title: String,
    pub score: u32,
}

const SCORE_HOSTNAME_IN_TITLE: u32 = 100;
const SCORE_REGISTRABLE_IN_TITLE: u32 = 80;
const SCORE_TITLE_IN_FOREGROUND: u32 = 40;
const SCORE_APP_NAME_MATCH: u32 = 20;

/// Rank every entry in the snapshot against the foreground window.
/// Entries in the Recycle Bin are skipped — surfacing a trashed
/// credential as a credible match would be confusing.
pub fn rank(snapshot: &VaultSnapshot, foreground: &ForegroundInfo) -> Vec<MatchedEntry> {
    let title_haystack = foreground.window_title.to_lowercase();
    let app_haystack = foreground.app_name.to_lowercase();

    let mut matches: Vec<MatchedEntry> = snapshot
        .entries_recursive()
        .into_iter()
        .filter(|e| !e.in_recycle_bin)
        .filter_map(|entry| {
            let score = score_entry(entry, &title_haystack, &app_haystack);
            if score == 0 {
                None
            } else {
                Some(MatchedEntry {
                    id: entry.id.clone(),
                    title: entry.title.clone(),
                    score,
                })
            }
        })
        .collect();

    // Stable secondary sort by id so a tie always resolves the same
    // way across runs — important so the "Auto-typed Foo" toast names
    // the same entry every time the user repeats the action on the
    // same login page.
    matches.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
    matches
}

fn score_entry(entry: &VaultEntry, title_haystack: &str, app_haystack: &str) -> u32 {
    let mut score = 0u32;

    if let Some(host) = host_of(&entry.url) {
        let host_lower = host.to_lowercase();
        if !host_lower.is_empty() && title_haystack.contains(&host_lower) {
            score += SCORE_HOSTNAME_IN_TITLE;
        }
        // Also try the "registrable" form (everything from the last
        // two dot-separated labels). Helps when the URL is
        // `accounts.google.com` but the title only says `Google`.
        // Crude — we don't want a PSL crate dep for this — but the
        // false-positive cost is bounded by the lower score weight.
        if let Some(short) = trim_to_registrable(&host_lower)
            && short != host_lower
            && title_haystack.contains(&short)
        {
            score += SCORE_REGISTRABLE_IN_TITLE;
        }
    }

    let title_lower = entry.title.to_lowercase();
    if !title_lower.is_empty() && title_haystack.contains(&title_lower) {
        score += SCORE_TITLE_IN_FOREGROUND;
    }

    if !title_lower.is_empty() && app_haystack.contains(&title_lower) {
        score += SCORE_APP_NAME_MATCH;
    }

    score
}

/// Extract a host string from an entry URL. Handles three input shapes:
///
/// 1. Fully-qualified URLs: `https://login.example.com/path`
/// 2. Scheme-less URLs the user typed by hand: `example.com`,
///    `www.example.com/foo`
/// 3. Garbage / non-URL text: returns `None`.
///
/// We use `url::Url` for case 1 and a manual best-effort split for
/// case 2 to avoid the silent failure where `Url::parse("example.com")`
/// returns `Err(RelativeUrlWithoutBase)` and KeePassXC users would
/// suddenly have no matches.
pub fn host_of(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(url) = url::Url::parse(trimmed)
        && let Some(h) = url.host_str()
    {
        return Some(strip_www(h).to_string());
    }
    // No scheme — split off path/query manually, then strip leading
    // user@ so URLs like `user@host.com` still resolve to the host.
    let after_at = trimmed.rsplit('@').next().unwrap_or(trimmed);
    let host_only = after_at
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_at)
        .trim();
    if host_only.is_empty() || !host_only.contains('.') {
        return None;
    }
    Some(strip_www(host_only).to_string())
}

fn strip_www(host: &str) -> &str {
    host.strip_prefix("www.").unwrap_or(host)
}

/// Crude best-effort "registrable host" (last two dot-separated labels).
/// Returns `None` for hosts with fewer than three labels (i.e. no
/// subdomain to trim away). We deliberately avoid a PSL dep — a
/// `co.uk`-style two-part TLD will produce `co.uk` from this function,
/// but downstream we treat the result as one possible match signal,
/// not authority. The hostname-in-title check already covers the
/// authoritative path.
fn trim_to_registrable(host: &str) -> Option<String> {
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 3 {
        return None;
    }
    Some(labels[labels.len() - 2..].join("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{VaultEntry, VaultGroup, VaultSnapshot};

    fn entry(id: &str, title: &str, username: &str, url: &str) -> VaultEntry {
        VaultEntry::new(id, title, username, url, true)
    }

    fn snapshot_with(entries: Vec<VaultEntry>) -> VaultSnapshot {
        VaultSnapshot::new(VaultGroup::new("root", "Root", Vec::new(), entries))
    }

    fn fg(app: &str, title: &str) -> ForegroundInfo {
        ForegroundInfo {
            app_name: app.into(),
            window_title: title.into(),
            process_path: std::path::PathBuf::new(),
        }
    }

    #[test]
    fn extracts_host_from_full_url() {
        assert_eq!(
            host_of("https://login.example.com/path?x=1").as_deref(),
            Some("login.example.com"),
        );
    }

    #[test]
    fn extracts_host_from_schemeless_url() {
        // Common in KeePass vaults — users type `github.com`, not
        // `https://github.com`. Url::parse rejects it, so the fallback
        // splitter has to handle it.
        assert_eq!(host_of("github.com").as_deref(), Some("github.com"));
        assert_eq!(
            host_of("github.com/login?next=/").as_deref(),
            Some("github.com"),
        );
    }

    #[test]
    fn strips_leading_www() {
        // `www.example.com` typically matches a title that just says
        // `example.com`; the `www.` is rarely in the page title.
        assert_eq!(
            host_of("https://www.example.com/").as_deref(),
            Some("example.com"),
        );
    }

    #[test]
    fn rejects_garbage_url() {
        assert_eq!(host_of(""), None);
        assert_eq!(host_of("not a url"), None);
        // No dot → not a host. Avoids matching every single-word title.
        assert_eq!(host_of("localhost"), None);
    }

    #[test]
    fn ranks_hostname_match_above_title_match() {
        let snap = snapshot_with(vec![
            entry("a", "GitHub", "alice", "https://github.com/login"),
            entry("b", "Login", "bob", ""),
        ]);
        let ranked = rank(&snap, &fg("Safari", "Sign in to GitHub · github.com"));
        // Hostname match beats title-substring match.
        assert_eq!(ranked[0].id, "a");
        assert_eq!(ranked[0].title, "GitHub");
        // The bare title-only entry shouldn't outrank a URL-bearing one.
        assert!(ranked[0].score > ranked.get(1).map(|m| m.score).unwrap_or(0));
    }

    #[test]
    fn no_match_returns_empty() {
        let snap = snapshot_with(vec![entry("a", "GitHub", "u", "https://github.com")]);
        let ranked = rank(&snap, &fg("TextEdit", "Untitled.txt"));
        assert!(ranked.is_empty(), "should not surface a credible match");
    }

    #[test]
    fn recycle_bin_entries_are_skipped() {
        // Surfacing a trashed credential as a match would be confusing —
        // and a security footgun if the user thinks they deleted a
        // credential but auto-type still offers it.
        let mut trashed = entry("a", "Old", "u", "https://github.com");
        trashed.in_recycle_bin = true;
        let snap = snapshot_with(vec![trashed]);
        let ranked = rank(&snap, &fg("Safari", "Sign in to GitHub"));
        assert!(ranked.is_empty());
    }

    #[test]
    fn subdomain_url_still_matches_short_title() {
        // KeePass entry `accounts.google.com` vs window title `Google`.
        // The registrable-form match (`google.com`) doesn't trigger
        // because the title is shorter, but the entry's title `Google`
        // matches the foreground title — the cheap title-substring
        // signal catches it.
        let snap = snapshot_with(vec![entry(
            "g",
            "Google",
            "u",
            "https://accounts.google.com/signin",
        )]);
        let ranked = rank(&snap, &fg("Chrome", "Google Account · Sign in"));
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].id, "g");
    }

    #[test]
    fn ties_resolve_by_id_deterministically() {
        // Two entries with the same URL and same title produce the same
        // score. The toast that names the typed entry must always
        // surface the same one, so users don't see different choices
        // on identical input.
        let snap = snapshot_with(vec![
            entry("zz", "Mail", "u1", "https://mail.example.com"),
            entry("aa", "Mail", "u2", "https://mail.example.com"),
        ]);
        let ranked = rank(&snap, &fg("Safari", "Inbox · mail.example.com"));
        assert_eq!(ranked[0].id, "aa", "lowest id wins on score tie");
    }
}

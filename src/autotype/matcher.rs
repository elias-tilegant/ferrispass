//! Score vault entries against a foreground window.
//!
//! Two signals qualify an entry for unattended typing:
//!
//! 1. An explicit KeePass `AutoType/Association/Window` pattern matching the
//!    foreground window title. The user wrote the pattern themselves (in any
//!    KeePass client), which makes it a deliberate trust decision — it
//!    therefore also applies in browsers, exactly like KeePass 2.x/XC.
//! 2. Exact equality between the app name and the entry URL's full hostname.
//!    We do not shorten hosts to an assumed registrable domain and do not use
//!    substring matches: `notgithub.com`, `github.com.evil`, and unrelated
//!    `*.co.uk` hosts must never qualify for `github.com` or `amazon.co.uk`
//!    credentials. Browser window titles are page-controlled and cannot
//!    authenticate the receiving site, so this *derived* signal fails closed
//!    in browsers.
//!
//! Entries whose KeePass Auto-Type is disabled never match. The explicit
//! in-app "type selected entry" route does not use this matcher.

use crate::autotype::window::ForegroundInfo;
use crate::domain::{VaultEntry, VaultSnapshot};

/// One ranked match. Scores are calibrated against
/// [`MIN_AUTOMATIC_SCORE`]; callers must use [`select_automatic`] rather than
/// blindly taking the first row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchedEntry {
    pub id: String,
    pub title: String,
    pub score: u32,
}

const SCORE_EXACT_APP_HOST: u32 = 200;
const SCORE_EXPLICIT_ASSOCIATION: u32 = 300;

/// Hard floor for unattended entry selection. Keeping this separate from the
/// current signal score makes future, weaker match signals fail closed unless
/// their author deliberately decides they are safe for automatic typing.
pub const MIN_AUTOMATIC_SCORE: u32 = SCORE_EXACT_APP_HOST;

/// Rank every entry in the snapshot against the foreground window.
/// Entries in the Recycle Bin are skipped — surfacing a trashed
/// credential as a credible match would be confusing.
pub fn rank(snapshot: &VaultSnapshot, foreground: &ForegroundInfo) -> Vec<MatchedEntry> {
    let mut matches: Vec<MatchedEntry> = snapshot
        .entries_recursive()
        .into_iter()
        .filter(|e| !e.in_recycle_bin)
        .filter_map(|entry| {
            let score = score_entry(entry, foreground);
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

    // The stable secondary ordering is useful for diagnostics and a future
    // chooser. It is deliberately not an ambiguity tie-breaker:
    // `select_automatic` rejects more than one credible candidate.
    matches.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.id.cmp(&b.id)));
    matches
}

/// Return the sole candidate that is strong enough for unattended typing.
/// Multiple candidates are ambiguous even when sorting made one deterministic;
/// account choice must be explicit in that case.
pub fn select_automatic(matches: &[MatchedEntry]) -> Option<&MatchedEntry> {
    let [only] = matches else {
        return None;
    };
    (only.score >= MIN_AUTOMATIC_SCORE).then_some(only)
}

fn score_entry(entry: &VaultEntry, foreground: &ForegroundInfo) -> u32 {
    if !entry.auto_type_enabled {
        return 0;
    }

    // User-authored association patterns are a deliberate trust decision
    // and apply everywhere, including browsers (KeePass 2.x/XC semantics).
    if entry
        .auto_type_windows
        .iter()
        .any(|pattern| window_pattern_matches(pattern, &foreground.window_title))
    {
        return SCORE_EXPLICIT_ASSOCIATION;
    }

    // A page can choose any title it wants. Without an extension/native-
    // messaging bridge we have no trustworthy site identity in a browser,
    // so the derived app-name signal fails closed there.
    if foreground.is_browser() {
        return 0;
    }

    let Some(host) = host_of(&entry.url) else {
        return 0;
    };

    if foreground.app_name.trim().eq_ignore_ascii_case(&host) {
        SCORE_EXACT_APP_HOST
    } else {
        0
    }
}

/// KeePass window-association matching: case-insensitive, `*` matches any
/// run of characters, `?` matches exactly one. The whole title must match
/// (KeePass anchors patterns; users write `*Sign in*` when they want
/// substring behavior). A blank pattern never matches.
// ponytail: KeePass's `//regex//` association syntax is not supported —
// add it if a vault with regex associations ever shows up.
fn window_pattern_matches(pattern: &str, title: &str) -> bool {
    let pattern: Vec<char> = pattern.trim().to_lowercase().chars().collect();
    if pattern.is_empty() {
        return false;
    }
    let title: Vec<char> = title.to_lowercase().chars().collect();
    glob_match(&pattern, &title)
}

/// Classic two-pointer wildcard match with `*` backtracking.
fn glob_match(pattern: &[char], text: &[char]) -> bool {
    let (mut p, mut t) = (0usize, 0usize);
    let mut backtrack: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            backtrack = Some((p, t));
            p += 1;
        } else if let Some((star_p, star_t)) = backtrack {
            p = star_p + 1;
            t = star_t + 1;
            backtrack = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// Extract a host string from an entry URL. Handles three input shapes:
///
/// 1. Fully-qualified URLs: `https://login.example.com/path`
/// 2. Scheme-less URLs the user typed by hand: `example.com`,
///    `www.example.com/foo`
/// 3. Garbage / non-URL text: returns `None`.
///
/// Scheme-less values are parsed by adding an `https://` base. Using `Url` for
/// both shapes rejects spaces, malformed ports and user-info edge cases that a
/// manual splitter can accidentally reinterpret as a host.
pub fn host_of(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let parsed = match url::Url::parse(trimmed) {
        Ok(url) if url.host_str().is_some() => url,
        // Do not reinterpret hostless explicit schemes such as `mailto:` or
        // `javascript:` as scheme-less web addresses.
        Ok(_) => return None,
        Err(url::ParseError::RelativeUrlWithoutBase) => {
            url::Url::parse(&format!("https://{trimmed}")).ok()?
        }
        Err(_) => return None,
    };
    let host = parsed
        .host_str()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let unbracketed = host.trim_start_matches('[').trim_end_matches(']');
    let is_ip_address = unbracketed.parse::<std::net::IpAddr>().is_ok();

    // Single-label names (for example `localhost`) do not provide enough
    // identity for unattended matching. Explicit in-app auto-type remains
    // available for local and intranet services.
    (host.contains('.') || is_ip_address).then_some(host)
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
    fn preserves_full_www_host() {
        // `www.example.com` and `example.com` can be different security
        // principals. Do not silently collapse them for credential matching.
        assert_eq!(
            host_of("https://www.example.com/").as_deref(),
            Some("www.example.com"),
        );
    }

    #[test]
    fn rejects_garbage_url() {
        assert_eq!(host_of(""), None);
        assert_eq!(host_of("not a url"), None);
        // No dot → not a host. Avoids matching every single-word title.
        assert_eq!(host_of("localhost"), None);
        assert_eq!(host_of("mailto:user@example.com"), None);
        assert_eq!(host_of("javascript:alert@github.com"), None);
    }

    fn entry_with_association(id: &str, url: &str, windows: &[&str]) -> VaultEntry {
        let mut e = entry(id, id, "u", url);
        e.auto_type_windows = windows.iter().map(|w| (*w).to_string()).collect();
        e
    }

    #[test]
    fn explicit_association_matches_by_window_title() {
        let snap = snapshot_with(vec![entry_with_association(
            "sap",
            "",
            &["SAP Logon ?60*"],
        )]);
        let ranked = rank(&snap, &fg("SAP Logon", "SAP Logon 760 — PRD"));
        assert_eq!(ranked.len(), 1);
        assert_eq!(
            select_automatic(&ranked).map(|m| m.id.as_str()),
            Some("sap")
        );
        assert!(rank(&snap, &fg("SAP Logon", "SAP Logon — PRD")).is_empty());
    }

    #[test]
    fn explicit_association_applies_in_browsers() {
        // The pattern is user-authored — a deliberate KeePass-standard trust
        // decision — so the browser gate does not apply to it.
        let snap = snapshot_with(vec![entry_with_association(
            "g",
            "https://github.com",
            &["*· github.com*"],
        )]);
        let ranked = rank(&snap, &fg("Safari", "Sign in · github.com — Safari"));
        assert_eq!(select_automatic(&ranked).map(|m| m.id.as_str()), Some("g"));
    }

    #[test]
    fn association_pattern_is_anchored() {
        let snap = snapshot_with(vec![entry_with_association("a", "", &["Sign in"])]);
        assert!(rank(&snap, &fg("App", "Sign in to Evil")).is_empty());
        assert_eq!(rank(&snap, &fg("App", "sign IN")).len(), 1);
    }

    #[test]
    fn blank_association_never_matches() {
        let snap = snapshot_with(vec![entry_with_association("a", "", &["", "   "])]);
        assert!(rank(&snap, &fg("App", "Anything")).is_empty());
    }

    #[test]
    fn disabled_auto_type_excludes_entry_from_all_signals() {
        let mut e = entry_with_association("s", "https://slack.com", &["*Slack*"]);
        e.auto_type_enabled = false;
        let snap = snapshot_with(vec![e]);
        assert!(rank(&snap, &fg("slack.com", "Slack — #general")).is_empty());
    }

    #[test]
    fn glob_edge_cases() {
        let cases = [
            ("*", "anything", true),
            ("a*b*c", "aXXbYYc", true),
            ("a*b*c", "aXXbYY", false),
            ("?", "x", true),
            ("?", "", false),
            ("*end", "the end", true),
            ("*end", "the ending", false),
        ];
        for (pattern, text, expected) in cases {
            let p: Vec<char> = pattern.chars().collect();
            let t: Vec<char> = text.chars().collect();
            assert_eq!(glob_match(&p, &t), expected, "{pattern} vs {text}");
        }
    }

    #[test]
    fn spoofed_browser_title_never_qualifies() {
        let snap = snapshot_with(vec![
            entry("a", "GitHub", "alice", "https://github.com/login"),
            entry("b", "Login", "bob", ""),
        ]);
        let ranked = rank(&snap, &fg("Safari", "Sign in to GitHub · github.com"));
        assert!(
            ranked.is_empty(),
            "page-controlled browser titles must not authorize auto-type"
        );
    }

    #[test]
    fn generic_title_only_entry_is_never_matched() {
        // Regression: a vault row called `Login` / `Mail` / `Admin`
        // (very common — users name catch-all secrets that way) used
        // to score against any window containing that word, typing
        // credentials into the wrong site.
        let snap = snapshot_with(vec![
            entry("a", "Login", "alice", ""),
            entry("b", "Mail", "bob", ""),
            entry("c", "Admin", "carol", ""),
        ]);
        for window_title in &[
            "Sign in to GitHub",
            "Login — Acme Corp",
            "Inbox · Mail",
            "Admin Panel — Stripe",
        ] {
            let ranked = rank(&snap, &fg("Safari", window_title));
            assert!(
                ranked.is_empty(),
                "title-only auto-type must not fire on '{window_title}', got {ranked:?}",
            );
        }
    }

    #[test]
    fn multiple_exact_candidates_are_ambiguous() {
        let snap = snapshot_with(vec![
            entry("z", "Personal", "alice", "https://github.com/login"),
            entry("y", "Work", "alice@corp", "https://github.com/login"),
        ]);
        let ranked = rank(&snap, &fg("github.com", "Work account"));
        assert_eq!(ranked.len(), 2);
        assert!(
            select_automatic(&ranked).is_none(),
            "sorting must never silently resolve an account ambiguity"
        );
    }

    #[test]
    fn exact_hostname_app_identity_qualifies() {
        let snap = snapshot_with(vec![entry("s", "Slack", "u", "https://slack.com")]);
        let ranked = rank(&snap, &fg("slack.com", "Untitled window"));
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].id, "s");
        assert_eq!(select_automatic(&ranked).map(|m| m.id.as_str()), Some("s"));
    }

    #[test]
    fn hostname_substrings_do_not_qualify() {
        let snap = snapshot_with(vec![entry("g", "GitHub", "u", "https://github.com")]);
        for app_name in ["notgithub.com", "github.com.evil", "foo-github.com"] {
            let ranked = rank(&snap, &fg(app_name, "github.com — Sign in"));
            assert!(ranked.is_empty(), "'{app_name}' must not match github.com");
        }
    }

    #[test]
    fn multipart_public_suffixes_are_not_collapsed() {
        let snap = snapshot_with(vec![entry(
            "a",
            "Amazon UK",
            "u",
            "https://www.amazon.co.uk/sign-in",
        )]);
        let ranked = rank(
            &snap,
            &fg("tesco.co.uk", "Amazon UK — www.amazon.co.uk — Sign in"),
        );
        assert!(
            ranked.is_empty(),
            "amazon.co.uk credentials must not qualify for another co.uk app"
        );
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
    fn full_subdomain_must_match_exactly() {
        let snap = snapshot_with(vec![entry(
            "g",
            "Google",
            "u",
            "https://accounts.google.com/signin",
        )]);
        let ranked = rank(&snap, &fg("accounts.google.com", "Sign in"));
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].id, "g");
        assert!(rank(&snap, &fg("google.com", "accounts.google.com")).is_empty());
    }

    #[test]
    fn below_minimum_score_is_rejected() {
        let weak = [MatchedEntry {
            id: "a".into(),
            title: "Weak future signal".into(),
            score: MIN_AUTOMATIC_SCORE - 1,
        }];
        assert!(select_automatic(&weak).is_none());
    }
}

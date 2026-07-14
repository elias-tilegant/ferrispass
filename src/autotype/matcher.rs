//! Score vault entries against a foreground window.
//!
//! Browser window titles are controlled by the page and therefore cannot
//! authenticate the site receiving a password. Until FerrisPass can read a
//! browser's active-tab URL through a trusted integration, automatic matching
//! in browsers fails closed. The explicit in-app "type selected entry" route
//! does not use this matcher.
//!
//! For non-browser applications the only current automatic signal is an exact
//! equality between the app name and the entry URL's full hostname. We do not
//! shorten hosts to an assumed registrable domain and do not use substring
//! matches: `notgithub.com`, `github.com.evil`, and unrelated `*.co.uk` hosts
//! must never qualify for `github.com` or `amazon.co.uk` credentials.

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

/// Hard floor for unattended entry selection. Keeping this separate from the
/// current signal score makes future, weaker match signals fail closed unless
/// their author deliberately decides they are safe for automatic typing.
pub const MIN_AUTOMATIC_SCORE: u32 = SCORE_EXACT_APP_HOST;

/// Rank every entry in the snapshot against the foreground window.
/// Entries in the Recycle Bin are skipped — surfacing a trashed
/// credential as a credible match would be confusing.
pub fn rank(snapshot: &VaultSnapshot, foreground: &ForegroundInfo) -> Vec<MatchedEntry> {
    // A page can choose any title it wants. Without an extension/native-
    // messaging bridge we have no trustworthy site identity in a browser.
    if foreground.is_browser() {
        return Vec::new();
    }

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
    let Some(host) = host_of(&entry.url) else {
        return 0;
    };

    if foreground.app_name.trim().eq_ignore_ascii_case(&host) {
        SCORE_EXACT_APP_HOST
    } else {
        0
    }
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

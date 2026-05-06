//! Favicon fetcher — pulls a small icon from DuckDuckGo's free icon
//! service for a given entry URL. The bytes are then written into the
//! KeePass database as a `Custom Icon` (see `VaultDocument::
//! set_entry_custom_icon`), so subsequent renders of the entry pick up
//! the real site icon instead of the synthesized colored letter.
//!
//! Why DuckDuckGo and not the site's own `/favicon.ico`:
//! - One CDN, one TLS handshake — much faster for a batch
//! - DDG normalises sizes / formats and serves a sensible default
//! - No `<link rel="icon">` HTML scraping required
//!
//! Privacy note: every URL hostname in the user's vault gets sent to
//! `icons.duckduckgo.com`. Acceptable for an explicit, user-initiated
//! "Download favicons" action; we don't run this in the background.
//!
//! Hard limits enforced here, not at callsite:
//! - 5 s timeout per host (favicons aren't worth blocking longer)
//! - 256 KiB max response (anything bigger is suspect)
//! - 100 byte minimum (sub-100 byte responses are usually transparent
//!   placeholders, not real icons)

use std::io::Read as _;
use std::time::Duration;

use thiserror::Error;
use url::Url;

const TIMEOUT: Duration = Duration::from_secs(5);
const MAX_BYTES: usize = 256 * 1024;
const MIN_BYTES: usize = 100;

#[derive(Debug, Error)]
pub enum FaviconError {
    #[error("entry has no URL or its URL is unparseable")]
    NoHost,
    #[error("network error: {0}")]
    Network(String),
    #[error("server returned status {0}")]
    Status(u16),
    #[error("response too small ({0} bytes); likely a placeholder")]
    Empty(usize),
    #[error("response too large ({0} bytes); aborted to keep DB lean")]
    Oversized(usize),
}

/// Fetch a favicon for the given entry URL via DuckDuckGo's icon service.
/// Returns the raw image bytes — the caller is responsible for validating
/// the format (our existing magic-byte sniffer in
/// `keepass::repository::favicon_image_from_bytes` does this when the icon
/// is later read back from the DB).
pub fn fetch_favicon(entry_url: &str) -> Result<Vec<u8>, FaviconError> {
    let host = host_from_url(entry_url).ok_or(FaviconError::NoHost)?;
    let target = format!("https://icons.duckduckgo.com/ip3/{host}.ico");

    // Per-call agent so the timeout sticks even if a future caller wraps
    // this in a long-running task — the global ureq default is "no
    // timeout", which is wrong for an icon fetcher.
    let agent = ureq::AgentBuilder::new()
        .timeout(TIMEOUT)
        .user_agent("ferrispass/favicon-fetcher")
        .build();

    let resp = agent.get(&target).call().map_err(|e| match e {
        ureq::Error::Status(code, _) => FaviconError::Status(code),
        ureq::Error::Transport(t) => FaviconError::Network(t.to_string()),
    })?;

    let mut bytes = Vec::with_capacity(2048);
    resp.into_reader()
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| FaviconError::Network(e.to_string()))?;

    if bytes.len() > MAX_BYTES {
        return Err(FaviconError::Oversized(bytes.len()));
    }
    if bytes.len() < MIN_BYTES {
        return Err(FaviconError::Empty(bytes.len()));
    }
    Ok(bytes)
}

/// Extract a hostname from an entry URL. Accepts URLs with or without a
/// scheme — many KeePass DBs store bare `github.com` style URLs that
/// `url::Url::parse` would otherwise reject. Returns the lowercased host
/// so `Github.COM` and `github.com` hit the same DDG cache key.
fn host_from_url(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed = if trimmed.contains("://") {
        Url::parse(trimmed).ok()
    } else {
        Url::parse(&format!("https://{trimmed}")).ok()
    }?;
    parsed.host_str().map(|s| s.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_from_full_url() {
        assert_eq!(
            host_from_url("https://www.GITHUB.com/login").as_deref(),
            Some("www.github.com")
        );
    }

    #[test]
    fn host_from_bare_domain() {
        assert_eq!(host_from_url("github.com").as_deref(), Some("github.com"));
    }

    #[test]
    fn host_strips_scheme_only_input() {
        // Common pattern in old vaults: just a domain with a trailing slash.
        assert_eq!(
            host_from_url("example.org/").as_deref(),
            Some("example.org")
        );
    }

    #[test]
    fn host_rejects_empty() {
        assert_eq!(host_from_url("").as_deref(), None);
        assert_eq!(host_from_url("   ").as_deref(), None);
    }
}

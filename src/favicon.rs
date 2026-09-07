//! Favicon fetcher - pulls a small icon from DuckDuckGo's free icon
//! service for a given entry URL. The bytes are then written into the
//! KeePass database as a `Custom Icon` (see `VaultDocument::
//! set_entry_custom_icon`), so subsequent renders of the entry pick up
//! the real site icon instead of the synthesized colored letter.
//!
//! Why DuckDuckGo and not the site's own `/favicon.ico`:
//! - One CDN, one TLS handshake - much faster for a batch
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

use std::time::Duration;

use thiserror::Error;

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
/// Returns the raw image bytes - the caller is responsible for validating
/// the format (our existing magic-byte sniffer in
/// `keepass::repository::favicon_image_from_bytes` does this when the icon
/// is later read back from the DB).
pub fn fetch_favicon(entry_url: &str) -> Result<Vec<u8>, FaviconError> {
    let host = host_from_url(entry_url).ok_or(FaviconError::NoHost)?;
    let target = format!("https://icons.duckduckgo.com/ip3/{host}.ico");

    // The shared metadata client: one connection pool and one TLS setup for
    // a run over hundreds of entries, and the same proxy and trust store the
    // rest of the app uses. The old per-call agent rebuilt both every time.
    let request = crate::sync::http::metadata_client()
        .map_err(|error| FaviconError::Network(error.to_string()))?
        .get(&target)
        .timeout(TIMEOUT)
        .header(reqwest::header::USER_AGENT, "ferrispass/favicon-fetcher");
    let bytes =
        crate::sync::http::fetch_metadata_bytes(request, MAX_BYTES as u64).map_err(|error| {
            match error {
                crate::sync::http::TransferError::TooLarge { max_bytes } => {
                    FaviconError::Oversized(max_bytes as usize)
                }
                // A 404 from the icon service means this host has no icon,
                // which is neither a network problem nor worth retrying.
                // Folding it into `Network` left `Status` unreachable and
                // described an ordinary miss as a connectivity failure.
                crate::sync::http::TransferError::Status { status } => FaviconError::Status(status),
                other => FaviconError::Network(other.to_string()),
            }
        })?;
    if bytes.len() < MIN_BYTES {
        return Err(FaviconError::Empty(bytes.len()));
    }
    Ok(bytes)
}

/// Extract a hostname from an entry URL, using the same rules auto-type
/// matches on. This module used to have its own copy that accepted a hostless
/// `mailto:` and a single-label `localhost`, neither of which DuckDuckGo can
/// answer for, and disagreed with the matcher on trailing dots.
fn host_from_url(input: &str) -> Option<String> {
    crate::autotype::matcher::host_of(input)
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

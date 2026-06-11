//! Shared `ureq` agents for every sync/auth HTTP call.
//!
//! ureq 2.x has **no default timeouts** — a request against a half-dead
//! connection (sleep/wake, captive portal, Wi-Fi dropped mid-read) blocks
//! its thread forever. That is fatal here because `auto_sync_in_flight`
//! entries are only cleared when the request completes: one hung Graph call
//! would silently disable auto-sync — and with it the refresh-token
//! keep-alive — for that vault until the app restarts. Routing every call
//! through these agents puts a hard upper bound on how long any sync step
//! can stall, and gets us connection reuse across the Graph calls for free.

use std::sync::OnceLock;
use std::time::Duration;

/// TCP/TLS connect budget. Generous enough for slow corporate proxies and
/// VPN handshakes; anything slower is effectively offline for our purposes.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Overall deadline for metadata-sized requests (auth, item lookup, search).
/// These bodies are a few KB — 30 s only ever elapses on a dead connection.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Overall deadline for vault content transfers (download / upload). Vaults
/// are capped at the 4 MB small-file PUT limit, so even a slow link finishes
/// well inside this; the point is "bounded", not "tight".
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(120);

/// Agent for token-endpoint and Graph metadata calls.
pub fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
    })
}

/// Agent for vault-content download/upload — same connect budget, longer
/// overall deadline so multi-MB bodies on slow links don't get cut off.
pub fn transfer_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout(TRANSFER_TIMEOUT)
            .build()
    })
}

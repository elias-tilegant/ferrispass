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

/// Maximum time a vault transfer may make no progress on an individual socket
/// read or write. Catches dead connections quickly without imposing a
/// minimum transfer speed on large vaults.
const TRANSFER_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Hard overall deadline for one vault transfer, enforced by ureq's
/// `DeadlineStream` across the whole request — body send, response
/// headers, and response-body reads. Idle timeouts alone let a peer that
/// trickles one byte per idle window hold the request (and the per-path
/// sync slot) forever. One hour covers the full 250 MB Graph content cap
/// even on a ~0.6 Mbit/s link.
pub(crate) const TRANSFER_MAX_WALL_CLOCK: Duration = Duration::from_secs(60 * 60);

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

/// Agent for vault-content download/upload. Idle limits catch dead
/// connections fast; the overall deadline is the hard upper bound for the
/// entire request in either direction.
pub fn transfer_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(TRANSFER_IDLE_TIMEOUT)
            .timeout_write(TRANSFER_IDLE_TIMEOUT)
            .timeout(TRANSFER_MAX_WALL_CLOCK)
            .build()
    })
}

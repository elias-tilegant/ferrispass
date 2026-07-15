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

/// Overall wall-clock budget for one vault transfer. NOT set via ureq's
/// `Agent::timeout` — in ureq 2.x that deadline displaces the per-operation
/// idle timeouts and is not re-checked before each body write. Instead the
/// two body loops enforce it per chunk (`read_vault_body` on download,
/// `DeadlineReader` on upload); each individual socket operation is bounded
/// by the idle timeouts, so the effective hard ceiling is this budget plus
/// one idle window. One hour covers the full 250 MB Graph content cap even
/// on a ~0.6 Mbit/s link.
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

/// Agent for vault-content download/upload. Idle limits bound every single
/// socket operation (including the wait for response headers); the overall
/// wall clock is enforced per body chunk by the callers — see
/// [`TRANSFER_MAX_WALL_CLOCK`].
pub fn transfer_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(TRANSFER_IDLE_TIMEOUT)
            .timeout_write(TRANSFER_IDLE_TIMEOUT)
            .build()
    })
}

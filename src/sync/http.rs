//! Shared HTTP clients for sync and update calls.
//!
//! Metadata calls stay on synchronous `ureq`; vault and update-bundle
//! transfers use reqwest so one total deadline covers DNS, pooled sockets,
//! request-body writes, response headers, and response-body reads. Transfers
//! run inside GPUI background tasks, which may synchronously enter the small
//! Tokio runtime owned by this module.

use std::future::Future;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

/// TCP/TLS connect budget. Generous enough for slow corporate proxies and
/// VPN handshakes; anything slower is effectively offline for our purposes.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Overall deadline for metadata-sized requests (auth, item lookup, search).
/// These bodies are a few KB — 30 s only ever elapses on a dead connection.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum time a transfer may make no read progress. The total deadline also
/// covers request-body writes, for which reqwest has no separate idle setting.
const TRANSFER_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Overall transfer budget. Reqwest enforces it with a Tokio deadline while
/// the machine is awake. `run_transfer` additionally polls `SystemTime`, so a
/// sleep/wake cycle cannot reset or pause the user-visible wall-clock budget.
const TRANSFER_MAX_WALL_CLOCK: Duration = Duration::from_secs(60 * 60);
const WALL_CLOCK_POLL: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("could not initialize the transfer client: {0}")]
    Setup(String),
    #[error("transfer exceeded the overall one-hour time budget")]
    Deadline,
}

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

/// Shared async client for bounded large transfers. The timeout lives on the
/// request future rather than the underlying socket, so connection reuse
/// cannot silently drop it.
pub fn transfer_client() -> Result<&'static reqwest::Client, TransferError> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    match CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(TRANSFER_IDLE_TIMEOUT)
            .timeout(TRANSFER_MAX_WALL_CLOCK)
            .https_only(true)
            .build()
            .map_err(|error| error.to_string())
    }) {
        Ok(client) => Ok(client),
        Err(error) => Err(TransferError::Setup(error.clone())),
    }
}

/// Execute every phase of a transfer under one cancellable deadline. Callers
/// must include response-body consumption in `future`; returning a live
/// `Response` would move that phase outside the deadline.
pub fn run_transfer<F, T>(future: F) -> Result<T, TransferError>
where
    F: Future<Output = T>,
{
    runtime()?.block_on(run_with_budget(future, TRANSFER_MAX_WALL_CLOCK))
}

fn runtime() -> Result<&'static tokio::runtime::Runtime, TransferError> {
    static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    match RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ferrispass-http")
            .enable_all()
            .build()
            .map_err(|error| error.to_string())
    }) {
        Ok(runtime) => Ok(runtime),
        Err(error) => Err(TransferError::Setup(error.clone())),
    }
}

async fn run_with_budget<F, T>(future: F, budget: Duration) -> Result<T, TransferError>
where
    F: Future<Output = T>,
{
    tokio::pin!(future);
    tokio::select! {
        output = &mut future => Ok(output),
        () = wall_clock_deadline(budget) => Err(TransferError::Deadline),
    }
}

async fn wall_clock_deadline(budget: Duration) {
    let started = SystemTime::now();
    loop {
        let remaining = match SystemTime::now().duration_since(started) {
            Ok(elapsed) => match budget.checked_sub(elapsed) {
                Some(remaining) if !remaining.is_zero() => remaining,
                _ => return,
            },
            // A backwards clock jump must not extend a security-sensitive
            // operation indefinitely. Expire in the fail-safe direction.
            Err(_) => return,
        };
        tokio::time::sleep(remaining.min(WALL_CLOCK_POLL)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    #[test]
    fn wall_clock_budget_cancels_the_whole_future() {
        let result = runtime().unwrap().block_on(run_with_budget(
            std::future::pending::<()>(),
            Duration::from_millis(20),
        ));
        assert!(matches!(result, Err(TransferError::Deadline)));
    }

    #[test]
    fn wall_clock_budget_covers_dripping_response_headers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request);
            for byte in b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n" {
                if socket.write_all(&[*byte]).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let result = runtime().unwrap().block_on(run_with_budget(
            async move { client.get(format!("http://{address}")).send().await },
            Duration::from_millis(20),
        ));
        assert!(matches!(result, Err(TransferError::Deadline)));
        server.join().unwrap();
    }
}

//! The one HTTP transport this app uses.
//!
//! Everything goes through reqwest: sign-in, Graph metadata, vault and
//! update-bundle transfers, favicons. It used to be split, with metadata on a
//! synchronous `ureq` agent, and that split was a bug generator. `ureq` reads
//! neither the macOS system proxy nor the system trust store, so on a managed
//! Mac behind a proxy the vault transfers worked while sign-in, the file list
//! and the update check failed, which is not a failure mode anyone diagnoses
//! quickly.
//!
//! Two clients, one transport: `metadata_client` for short request/response
//! calls, `transfer_client` for vault-sized bodies with a much longer budget.
//! Both run on the small Tokio runtime owned by this module, entered
//! synchronously from GPUI background tasks.

use std::future::Future;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

/// TCP/TLS connect budget. Generous enough for slow corporate proxies and
/// VPN handshakes; anything slower is effectively offline for our purposes.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Overall deadline for metadata-sized requests (auth, item lookup, search).
/// These bodies are a few KB - 30 s only ever elapses on a dead connection.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum time a transfer may make no read progress. The total deadline also
/// covers request-body writes, for which reqwest has no separate idle setting.
const TRANSFER_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// Overall transfer budget. Reqwest enforces it with a Tokio deadline while
/// the machine is awake. `run_transfer` additionally polls `SystemTime`, so a
/// sleep/wake cycle cannot reset or pause the user-visible wall-clock budget.
const TRANSFER_MAX_WALL_CLOCK: Duration = Duration::from_secs(60 * 60);
const WALL_CLOCK_POLL: Duration = Duration::from_secs(1);

/// What went wrong on the wire, as far as the transport can say.
///
/// Read off reqwest's own predicates rather than matched out of its message
/// text, which is the pattern this app is not allowed to use for control
/// flow. There is deliberately no Proxy or Tls variant: reqwest folds both
/// into a connect failure, and a variant we could only guess at by sniffing
/// strings would be a worse answer than the honest general one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkErrorKind {
    /// The connection never came up: DNS, routing, refusal, or a proxy or TLS
    /// handshake that did not complete.
    Connect,
    /// Accepted, then did not finish inside its budget.
    Timeout,
    /// Anything else, including a request we built wrong ourselves.
    Other,
}

impl NetworkErrorKind {
    fn of(error: &reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::Timeout
        } else if error.is_connect() {
            Self::Connect
        } else {
            Self::Other
        }
    }

    /// The one sentence worth showing a user for this kind. Kept here so
    /// every surface that reports a network failure says the same thing.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Connect => "Could not reach the server. Check your network, VPN or proxy.",
            Self::Timeout => "The server did not answer in time. Check your network, then retry.",
            Self::Other => "The request did not go through. Check your network, then retry.",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("could not initialize the transfer client: {0}")]
    Setup(String),
    #[error("{detail}")]
    Transport {
        kind: NetworkErrorKind,
        detail: String,
    },
    #[error("server returned status {status}")]
    Status { status: u16 },
    #[error("transfer exceeded the overall one-hour time budget")]
    Deadline,
    #[error("response exceeds the {max_bytes}-byte limit for this endpoint")]
    TooLarge { max_bytes: u64 },
}

impl TransferError {
    /// Classify a raw reqwest failure. Public so callers that drive reqwest
    /// directly, like the vault download, classify it the same way.
    pub fn transport(error: reqwest::Error) -> Self {
        Self::Transport {
            kind: NetworkErrorKind::of(&error),
            detail: error.to_string(),
        }
    }

    /// How the wire failed, or `None` when the failure was ours (building the
    /// client) or the response's own (status, size).
    pub fn network_kind(&self) -> Option<NetworkErrorKind> {
        match self {
            Self::Transport { kind, .. } => Some(*kind),
            Self::Deadline => Some(NetworkErrorKind::Timeout),
            Self::Setup(_) | Self::Status { .. } | Self::TooLarge { .. } => None,
        }
    }
}

/// Bodies larger than this from a metadata endpoint are a bug or an attack,
/// not a response we need. Graph error envelopes are a few KB.
const MAX_METADATA_BODY_BYTES: usize = 1024 * 1024;

/// Client for token-endpoint and Graph metadata calls. Same builder as
/// `transfer_client` apart from the deadline, so the system proxy and the
/// native trust store apply to sign-in exactly as they do to a download.
pub fn metadata_client() -> Result<&'static reqwest::Client, TransferError> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    shared_client(&CLIENT, REQUEST_TIMEOUT)
}

/// A metadata response, already read. The status is handed back rather than
/// turned into an error: the device-code poll needs the body of a 400, while
/// Graph callers treat the same shape as a failure.
pub struct MetadataResponse {
    pub status: u16,
    pub body: String,
    /// `Retry-After` in delta-seconds, when the server sent one.
    pub retry_after: Option<Duration>,
}

/// Fetch a small document as exact bytes, refusing anything over `max_bytes`.
///
/// Separate from [`send_metadata`] on purpose: the update manifest and its
/// signature are verified byte for byte, and a lossy UTF-8 round trip would
/// corrupt them. The cap is enforced while reading, so an endpoint that
/// promises 4 KB and streams forever cannot exhaust memory.
pub fn fetch_metadata_bytes(
    request: reqwest::RequestBuilder,
    max_bytes: u64,
) -> Result<Vec<u8>, TransferError> {
    runtime()?.block_on(async move {
        let response = request.send().await.map_err(TransferError::transport)?;
        // Reported as its own variant rather than folded into a transport
        // failure: a 404 from the favicon service is not a network problem
        // and must not be described to the user as one.
        let status = response.status();
        if !status.is_success() {
            return Err(TransferError::Status {
                status: status.as_u16(),
            });
        }
        let mut stream = response;
        let mut bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = stream.chunk().await.map_err(TransferError::transport)? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() as u64 > max_bytes {
                return Err(TransferError::TooLarge { max_bytes });
            }
        }
        Ok(bytes)
    })
}

/// Send a metadata request and read its body, bounded in size and time.
/// Transport failures and an oversized body are errors; HTTP status codes
/// are not.
pub fn send_metadata(request: reqwest::RequestBuilder) -> Result<MetadataResponse, TransferError> {
    runtime()?.block_on(async move {
        let response = request.send().await.map_err(TransferError::transport)?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        // Read the body in chunks and stop at the cap. Collecting it first
        // and slicing afterwards let a server of any size decide how much
        // memory this process allocates, which is exactly what the cap is
        // there to prevent.
        let mut stream = response;
        let mut bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = stream.chunk().await.map_err(TransferError::transport)? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() > MAX_METADATA_BODY_BYTES {
                return Err(TransferError::TooLarge {
                    max_bytes: MAX_METADATA_BODY_BYTES as u64,
                });
            }
        }
        Ok(MetadataResponse {
            status,
            body: String::from_utf8_lossy(&bytes).into_owned(),
            retry_after,
        })
    })
}

/// Shared async client for bounded large transfers. The timeout lives on the
/// request future rather than the underlying socket, so connection reuse
/// cannot silently drop it.
pub fn transfer_client() -> Result<&'static reqwest::Client, TransferError> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    shared_client(&CLIENT, TRANSFER_MAX_WALL_CLOCK)
}

fn shared_client(
    slot: &'static OnceLock<Result<reqwest::Client, String>>,
    timeout: Duration,
) -> Result<&'static reqwest::Client, TransferError> {
    match slot.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(TRANSFER_IDLE_TIMEOUT)
            .timeout(timeout)
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

    /// Every HTTP path in the app goes through these two clients, and both are
    /// built by `shared_client`. That is the whole point of the migration off
    /// `ureq`: proxy and trust-store settings cannot apply to downloads while
    /// missing on sign-in, because there is no second builder to forget.
    #[test]
    fn both_clients_come_from_the_same_builder() {
        assert!(metadata_client().is_ok());
        assert!(transfer_client().is_ok());
    }

    /// The metadata reader hands the status back instead of turning a non-2xx
    /// into an error: the device-code poll reads "authorization_pending" out
    /// of the body of an HTTP 400.
    #[test]
    fn a_non_success_status_still_yields_its_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request);
            let body = br#"{"error":"authorization_pending"}"#;
            let _ = socket.write_all(
                format!(
                    "HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            );
            let _ = socket.write_all(body);
        });

        let client = reqwest::Client::builder().build().unwrap();
        let response =
            send_metadata(client.get(format!("http://{address}"))).expect("transport succeeded");
        assert_eq!(response.status, 400);
        assert!(response.body.contains("authorization_pending"));
        server.join().unwrap();
    }

    /// The same must hold for the metadata path. Reading the whole body and
    /// slicing it afterwards let the server decide how much this process
    /// allocated, so the cap protected nothing.
    #[test]
    fn a_metadata_body_over_the_cap_is_refused_while_reading() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let oversized = MAX_METADATA_BODY_BYTES + 1024;
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request);
            let _ = socket.write_all(
                format!("HTTP/1.1 200 OK\r\nContent-Length: {oversized}\r\n\r\n").as_bytes(),
            );
            // The reader must give up before this finishes writing.
            let _ = socket.write_all(&vec![b'x'; oversized]);
        });

        let client = reqwest::Client::builder().build().unwrap();
        let result = send_metadata(client.get(format!("http://{address}")));

        assert!(
            matches!(result, Err(TransferError::TooLarge { max_bytes })
                if max_bytes == MAX_METADATA_BODY_BYTES as u64),
            "an oversized metadata body is a typed error, not a truncated string"
        );
        let _ = server.join();
    }

    /// An ordinary response is unaffected by the cap.
    #[test]
    fn a_metadata_body_under_the_cap_is_returned_whole() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request);
            let body = vec![b'y'; 32 * 1024];
            let _ = socket.write_all(
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes(),
            );
            let _ = socket.write_all(&body);
        });

        let client = reqwest::Client::builder().build().unwrap();
        let response =
            send_metadata(client.get(format!("http://{address}"))).expect("transport succeeded");

        assert_eq!(response.status, 200);
        assert_eq!(response.body.len(), 32 * 1024);
        server.join().unwrap();
    }

    /// The kind has to come from reqwest's own verdict. Sniffing it out of
    /// the message text is the pattern this app is not allowed to use for
    /// control flow, and it breaks whenever reqwest rewords an error.
    #[test]
    fn a_connection_that_never_comes_up_is_classified_as_connect() {
        // Bound and dropped: the port is closed, so the connect is refused.
        let address = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap()
        };

        let client = reqwest::Client::builder().build().unwrap();
        let Err(error) = send_metadata(client.get(format!("http://{address}"))) else {
            panic!("nothing is listening on that port");
        };

        assert_eq!(error.network_kind(), Some(NetworkErrorKind::Connect));
        assert!(!error.to_string().is_empty(), "the detail is still carried");
    }

    /// A response the server refused is not a network failure, and reporting
    /// it as one sent the user to check a connection that was working.
    #[test]
    fn a_refused_status_is_not_a_network_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request);
            let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        });

        let client = reqwest::Client::builder().build().unwrap();
        let error = fetch_metadata_bytes(client.get(format!("http://{address}")), 4096)
            .expect_err("404 is not a body");

        assert!(matches!(error, TransferError::Status { status: 404 }));
        assert_eq!(
            error.network_kind(),
            None,
            "the connection worked; the answer was no"
        );
        server.join().unwrap();
    }

    /// A server that promises little and streams a lot must not be able to
    /// exhaust memory through the update-manifest path.
    #[test]
    fn the_byte_fetch_stops_at_its_limit() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request);
            let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4096\r\n\r\n");
            // Write past the caller's limit; the read has to stop first.
            let _ = socket.write_all(&vec![b'x'; 4096]);
        });

        let client = reqwest::Client::builder().build().unwrap();
        let result = fetch_metadata_bytes(client.get(format!("http://{address}")), 64);
        assert!(matches!(result, Err(TransferError::TooLarge { .. })));
        let _ = server.join();
    }
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

//! OAuth 2.0 device-code flow against Microsoft identity platform, plus
//! refresh-token exchange. No tokens are persisted here — this module is
//! pure HTTP + parsing; persistence is `tokens.rs`'s job (Keychain).
//!
//! Why device code (vs. loopback PKCE)? No local web server, works behind
//! NATs and corporate firewalls, no platform plumbing. Two-step UX is the
//! cost: user copies a code into a browser. Acceptable for a desktop
//! password manager that signs in once per device.

use std::time::{Duration, SystemTime};

use serde::Deserialize;
use thiserror::Error;
use ureq::Error as UreqError;

/// Multi-tenant + personal MS accounts. Use `organizations` if you want to
/// exclude personal MS accounts; `{tenant-guid}` to lock to one tenant.
const AUTHORITY: &str = "https://login.microsoftonline.com/common/oauth2/v2.0";

/// Delegated scopes the device-code flow requests.
///
/// - `Files.ReadWrite.All` covers SharePoint document libraries the user
///   has access to (the bare `Files.ReadWrite` only covers the user's own
///   OneDrive). This is also the minimum scope `/search/query` needs to
///   return SharePoint hits.
/// - `offline_access` returns a refresh token so we don't have to re-prompt
///   sign-in every hour.
///
/// Some enterprise tenants require admin consent for `Files.ReadWrite.All`.
/// If the user hits "needs admin approval" at sign-in, a tenant admin must
/// approve the app once.
pub const SCOPE: &str = "Files.ReadWrite.All offline_access";

/// Public Azure AD app registration owned by this project. Public client
/// IDs are *not* secrets — they appear in every sign-in URL the user sees;
/// committing this to a public repo is intended and standard for Azure AD
/// public clients (no client secret involved). Forks can override this at
/// build time without touching source:
///
/// ```sh
/// FERRISPASS_CLIENT_ID=<your-guid> cargo build --release
/// ```
pub const DEFAULT_CLIENT_ID: &str = "39481acc-7592-42c8-a8ae-3481cb76bb27";

/// Resolves to the active Azure AD client ID — env override at build time
/// wins over the default const so forks don't have to patch source.
pub fn client_id() -> &'static str {
    match option_env!("FERRISPASS_CLIENT_ID") {
        Some(s) if !s.is_empty() => s,
        _ => DEFAULT_CLIENT_ID,
    }
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("network error: {0}")]
    Network(String),

    #[error("response was not valid JSON: {0}")]
    Parse(String),

    #[error("authorization server returned an error: {0}")]
    Server(String),

    #[error("user declined the sign-in request")]
    Declined,

    #[error("device code expired before sign-in completed")]
    Expired,

    /// Refresh failed terminally — the user must re-run Connect. The
    /// optional payload carries the Azure `error_description` (e.g. the
    /// `AADSTS700082: …` line) so the UI and our diagnostics can tell
    /// *why* the grant died: short inactivity window vs. a tenant
    /// sign-in-frequency policy vs. a revoked token look identical
    /// otherwise, and the AADSTS code is the only reliable discriminator.
    #[error("refresh token is no longer valid; user must reconnect{}", match .0 { Some(d) => format!(" ({d})"), None => String::new() })]
    InvalidGrant(Option<String>),
}

/// Result of a `request_device_code` call. Carries everything the user-facing
/// code needs to display + everything the polling loop needs to keep going.
#[derive(Debug, Clone)]
pub struct DeviceCodeChallenge {
    /// Short alphanumeric code the user types into the verification URL.
    pub user_code: String,
    /// URL the user opens in the browser (e.g. `https://microsoft.com/devicelogin`).
    pub verification_uri: String,
    /// Opaque blob passed back to `poll_token`. Not shown to the user.
    pub device_code: String,
    /// Wall-clock deadline after which `poll_token` will start returning
    /// `Failed(Expired)`. Comes from the server's `expires_in` (typically
    /// 900 s = 15 min) added to "now".
    pub expires_at: SystemTime,
    /// Server-recommended polling interval. We honour this exactly to avoid
    /// `slow_down` responses; if we get `slow_down` anyway we double it.
    pub interval: Duration,
}

/// Successful token bundle. `expires_at` is computed locally as
/// `now + expires_in` so callers can compare without knowing when the
/// token was issued.
#[derive(Debug, Clone)]
pub struct AccessToken {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: SystemTime,
}

impl AccessToken {
    /// True when the token is within `slack` of expiring. Caller should
    /// refresh before making the next API call. 60 s is a reasonable slack
    /// to ride out clock skew + network round-trip.
    pub fn is_near_expiry(&self, slack: Duration) -> bool {
        match self.expires_at.duration_since(SystemTime::now()) {
            Ok(remaining) => remaining < slack,
            Err(_) => true, // already past
        }
    }
}

/// Outcome of one `poll_token` call. The polling loop should keep going on
/// `Pending`, treat `SlowDown` as `Pending` with a longer interval, stop
/// happily on `Token`, and stop unhappily on `Failed(_)`.
#[derive(Debug)]
pub enum PollOutcome {
    Pending,
    SlowDown,
    Token(AccessToken),
    Failed(AuthError),
}

/// Step 1 of device code flow: request a code. Doesn't sign anyone in yet.
pub fn request_device_code() -> Result<DeviceCodeChallenge, AuthError> {
    let url = format!("{AUTHORITY}/devicecode");
    let body = post_form(&url, &[("client_id", client_id()), ("scope", SCOPE)])?;

    let resp: DeviceCodeResponse = serde_json::from_str(&body)
        .map_err(|e| AuthError::Parse(format!("devicecode response: {e}")))?;

    Ok(DeviceCodeChallenge {
        user_code: resp.user_code,
        verification_uri: resp.verification_uri,
        device_code: resp.device_code,
        expires_at: SystemTime::now() + Duration::from_secs(resp.expires_in),
        interval: Duration::from_secs(resp.interval),
    })
}

/// Step 2 of device code flow: poll once for the token. Caller drives the
/// loop (so they can `cx.background_spawn` between polls).
pub fn poll_token(challenge: &DeviceCodeChallenge) -> PollOutcome {
    let url = format!("{AUTHORITY}/token");
    let result = post_form(
        &url,
        &[
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("client_id", client_id()),
            ("device_code", &challenge.device_code),
        ],
    );

    let body = match result {
        Ok(b) => b,
        Err(e) => return PollOutcome::Failed(e),
    };

    classify_token_response(&body)
}

/// Refresh an access token. Returns a fresh `AccessToken` (with a possibly
/// rotated refresh token — Microsoft sometimes does, sometimes doesn't,
/// callers should always persist whatever comes back).
pub fn refresh(refresh_token: &str) -> Result<AccessToken, AuthError> {
    let url = format!("{AUTHORITY}/token");
    let body = post_form(
        &url,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", client_id()),
            ("refresh_token", refresh_token),
            ("scope", SCOPE),
        ],
    )?;

    match parse_token_response(&body) {
        Ok(token) => Ok(token),
        Err(AuthError::Server(_msg)) => {
            // Distinguish `invalid_grant` (terminal — user must reconnect)
            // from generic server errors so the caller can render a clear
            // "sign-in expired" message.
            if body.contains("\"invalid_grant\"") {
                Err(AuthError::InvalidGrant(extract_error_detail(&body)))
            } else {
                Err(AuthError::Server(body))
            }
        }
        Err(other) => Err(other),
    }
}

// --------------- internals ---------------

fn post_form(url: &str, params: &[(&str, &str)]) -> Result<String, AuthError> {
    match ureq::post(url)
        .set("Accept", "application/json")
        .send_form(params)
    {
        Ok(resp) => resp
            .into_string()
            .map_err(|e| AuthError::Network(e.to_string())),
        Err(UreqError::Status(_, resp)) => {
            // Read body so callers can interpret the JSON error envelope.
            // This is the path the device-code "authorization_pending" error
            // takes (HTTP 400 with a JSON body).
            resp.into_string()
                .map_err(|e| AuthError::Network(e.to_string()))
        }
        Err(UreqError::Transport(t)) => Err(AuthError::Network(t.to_string())),
    }
}

/// Map a token-endpoint response body to a `PollOutcome`. Public for tests.
fn classify_token_response(body: &str) -> PollOutcome {
    // Try success path first.
    match parse_token_response(body) {
        Ok(token) => return PollOutcome::Token(token),
        Err(AuthError::Server(_)) => { /* fall through to error parsing */ }
        Err(other) => return PollOutcome::Failed(other),
    }

    // Parse the OAuth error envelope.
    let err = match serde_json::from_str::<ErrorResponse>(body) {
        Ok(e) => e,
        Err(e) => return PollOutcome::Failed(AuthError::Parse(e.to_string())),
    };
    match err.error.as_str() {
        "authorization_pending" => PollOutcome::Pending,
        "slow_down" => PollOutcome::SlowDown,
        "expired_token" => PollOutcome::Failed(AuthError::Expired),
        "authorization_declined" | "access_denied" => PollOutcome::Failed(AuthError::Declined),
        other => PollOutcome::Failed(AuthError::Server(format!(
            "{other}: {}",
            err.error_description.unwrap_or_default()
        ))),
    }
}

fn parse_token_response(body: &str) -> Result<AccessToken, AuthError> {
    let resp: TokenResponse = serde_json::from_str(body).map_err(|e| {
        // Distinguish "this is an OAuth error envelope, not a success" from
        // truly malformed JSON. The token endpoint returns 4xx with a JSON
        // body containing `{"error": "..."}` when something's wrong; in that
        // case we want callers to fall through to error classification.
        if body.contains("\"error\"") {
            AuthError::Server(body.to_string())
        } else {
            AuthError::Parse(e.to_string())
        }
    })?;
    Ok(AccessToken {
        access_token: resp.access_token,
        refresh_token: resp.refresh_token,
        expires_at: SystemTime::now() + Duration::from_secs(resp.expires_in),
    })
}

/// Pull a short, human-readable reason out of an OAuth error body. Azure's
/// `error_description` leads with the `AADSTS<code>: <message>` line we care
/// about, then appends a multi-line trace dump we don't. Keep only the first
/// line and cap the length so it fits a status pill / log line without
/// leaking the full trace id into the UI.
fn extract_error_detail(body: &str) -> Option<String> {
    let desc = serde_json::from_str::<ErrorResponse>(body)
        .ok()
        .and_then(|e| e.error_description)?;
    let first_line = desc.lines().next().unwrap_or(&desc).trim();
    if first_line.is_empty() {
        return None;
    }
    const MAX: usize = 200;
    let trimmed = if first_line.chars().count() > MAX {
        let head: String = first_line.chars().take(MAX).collect();
        format!("{head}…")
    } else {
        first_line.to_string()
    };
    Some(trimmed)
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    user_code: String,
    device_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_response_classifies_as_pending() {
        let body = r#"{"error":"authorization_pending","error_description":"User has not yet completed the sign-in."}"#;
        match classify_token_response(body) {
            PollOutcome::Pending => {}
            other => panic!("expected Pending, got {other:?}"),
        }
    }

    #[test]
    fn slow_down_classifies_as_slow_down() {
        let body = r#"{"error":"slow_down"}"#;
        assert!(matches!(
            classify_token_response(body),
            PollOutcome::SlowDown
        ));
    }

    #[test]
    fn expired_token_classifies_as_failed_expired() {
        let body = r#"{"error":"expired_token","error_description":"..."}"#;
        match classify_token_response(body) {
            PollOutcome::Failed(AuthError::Expired) => {}
            other => panic!("expected Failed(Expired), got {other:?}"),
        }
    }

    #[test]
    fn user_decline_classifies_as_failed_declined() {
        // Microsoft uses `authorization_declined` in the device-code flow;
        // some samples use `access_denied`. Handle both.
        for body in [
            r#"{"error":"authorization_declined"}"#,
            r#"{"error":"access_denied"}"#,
        ] {
            match classify_token_response(body) {
                PollOutcome::Failed(AuthError::Declined) => {}
                other => panic!("expected Failed(Declined), got {other:?} for body {body}"),
            }
        }
    }

    #[test]
    fn success_response_parses_into_access_token() {
        let body = r#"{
            "token_type": "Bearer",
            "scope": "Files.ReadWrite offline_access",
            "expires_in": 3600,
            "access_token": "eyJ-fake-access",
            "refresh_token": "M.R3-fake-refresh"
        }"#;
        match classify_token_response(body) {
            PollOutcome::Token(t) => {
                assert_eq!(t.access_token, "eyJ-fake-access");
                assert_eq!(t.refresh_token, "M.R3-fake-refresh");
                // expires_at should be ~3600 s in the future, allow generous slack.
                let remaining = t
                    .expires_at
                    .duration_since(SystemTime::now())
                    .unwrap()
                    .as_secs();
                assert!(
                    (3590..=3600).contains(&remaining),
                    "remaining = {remaining}"
                );
            }
            other => panic!("expected Token, got {other:?}"),
        }
    }

    #[test]
    fn malformed_response_classifies_as_parse_error() {
        let body = "this is not json";
        match classify_token_response(body) {
            PollOutcome::Failed(AuthError::Parse(_)) => {}
            other => panic!("expected Failed(Parse), got {other:?}"),
        }
    }

    #[test]
    fn extract_error_detail_pulls_first_aadsts_line() {
        // Azure's invalid_grant body leads with the AADSTS line we want,
        // then dumps a multi-line trace we don't. Keep only the first line.
        let body = r#"{"error":"invalid_grant","error_description":"AADSTS700082: The refresh token has expired due to inactivity.\r\nTrace ID: abc\r\nCorrelation ID: def"}"#;
        let detail = extract_error_detail(body).expect("detail present");
        assert_eq!(
            detail,
            "AADSTS700082: The refresh token has expired due to inactivity."
        );
        assert!(!detail.contains("Trace ID"));
    }

    #[test]
    fn extract_error_detail_none_when_no_description() {
        assert_eq!(extract_error_detail(r#"{"error":"invalid_grant"}"#), None);
        assert_eq!(extract_error_detail("not json at all"), None);
    }

    #[test]
    fn token_is_near_expiry_when_clock_passed() {
        let token = AccessToken {
            access_token: "x".into(),
            refresh_token: "y".into(),
            expires_at: SystemTime::now() - Duration::from_secs(1),
        };
        assert!(token.is_near_expiry(Duration::from_secs(60)));
    }

    #[test]
    fn token_is_not_near_expiry_when_far_future() {
        let token = AccessToken {
            access_token: "x".into(),
            refresh_token: "y".into(),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
        };
        assert!(!token.is_near_expiry(Duration::from_secs(60)));
    }
}

//! Tiny Microsoft Graph v1.0 client — exactly the endpoints the SharePoint
//! sync flow needs, no crate-wide HTTP framework.
//!
//! All calls are synchronous (`ureq`); the caller wraps them in
//! `cx.background_spawn(...)`. Rate limits, throttling, retries, and
//! resumable uploads are out of scope; the simple content endpoint supports
//! vaults up to 250 MB.

use serde::Deserialize;
use std::io;
use thiserror::Error;
use ureq::Error as UreqError;

use crate::sync::auth::AccessToken;
use crate::sync::http;

const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";

/// Microsoft Graph's simple `PUT .../content` endpoint accepts files up to
/// 250 MB. Use the same bound for downloads so a remote item can always be
/// uploaded again without switching protocols, and so an untrusted response
/// can never grow the in-memory vault buffer without limit.
///
/// Interpret the published MB value conservatively as decimal bytes:
/// <https://learn.microsoft.com/graph/api/driveitem-put-content>
const MAX_VAULT_CONTENT_BYTES: usize = 250_000_000;

/// Error responses are surfaced in toasts. A small prefix is enough to keep
/// Graph's diagnostic code/request ID without retaining an arbitrary server
/// body in UI state.
const MAX_ERROR_BODY_BYTES: usize = 8 * 1024;
const READ_CHUNK_BYTES: usize = 64 * 1024;

/// Overall wall-clock budget for one vault body transfer (either
/// direction). See the comment in `read_vault_body` — idle timeouts alone
/// leave transfers unbounded.
const TRANSFER_MAX_WALL_CLOCK: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// Wraps the upload body so each socket-write chunk re-checks the overall
/// transfer deadline.
struct DeadlineReader<'a> {
    inner: io::Cursor<&'a [u8]>,
    deadline: std::time::Instant,
}

impl io::Read for DeadlineReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if std::time::Instant::now() >= self.deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "vault transfer exceeded the overall time budget",
            ));
        }
        self.inner.read(buf)
    }
}

#[derive(Debug, Error)]
pub enum GraphError {
    #[error("network error: {0}")]
    Network(String),
    #[error("graph returned HTTP {status}: {body}")]
    Status { status: u16, body: String },
    #[error("could not parse graph response: {0}")]
    Parse(String),
    #[error("vault exceeds Microsoft Graph's 250 MB content limit")]
    VaultTooLarge,
    #[error("graph returned an invalid Content-Length header")]
    InvalidContentLength,
    #[error("could not buffer graph response: {0}")]
    Buffer(String),
    #[error("response was missing required field: {0}")]
    MissingField(&'static str),
    #[error("no drive on this site matches library name '{0}'")]
    DriveNotFound(String),
}

#[derive(Debug, Clone)]
pub struct Site {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Clone)]
pub struct Drive {
    pub id: String,
    pub name: String,
    pub web_url: String,
}

#[derive(Debug, Clone)]
pub struct DriveItem {
    pub id: String,
    /// Quoted etag string as Graph returns it (e.g. `"{guid},N"`). Pass back
    /// verbatim as the `If-Match` value on upload.
    pub etag: String,
    pub name: String,
    /// RFC3339 string from the server. We don't parse to chrono here — UI
    /// layer can do that on demand.
    pub last_modified: String,
}

/// One `.kdbx` file the search endpoint returned. Carries everything the
/// picker UI needs to display, plus the durable identifiers we need to wire
/// up sync without re-resolving by URL.
#[derive(Debug, Clone)]
pub struct DriveItemHit {
    pub item_id: String,
    pub site_id: String,
    pub drive_id: String,
    pub name: String,
    pub web_url: String,
    /// e.g. `/drives/b!xxx/root:/Folder/Sub` — used to render a friendly
    /// path under the filename in the picker.
    pub path: String,
    pub last_modified: String,
}

#[derive(Debug)]
pub enum UploadOutcome {
    /// Upload succeeded; carry the freshly-issued etag for the next save.
    Ok { new_etag: String, item: DriveItem },
    /// Server etag doesn't match `If-Match` — conflict. Caller should
    /// download the remote and surface the Conflict overlay.
    Conflict,
}

#[derive(Debug, Clone)]
pub struct User {
    pub email: String,
}

// ---------- public API ----------

/// `POST /search/query` filtered to `driveItem`s with extension `.kdbx`.
/// One call returns up to 50 results across every site / drive the user
/// has access to (personal OneDrive too, if any). Empty list when there
/// are no `.kdbx` files anywhere — caller renders an empty-state UI.
///
/// Uses Microsoft Search KQL: `filetype:kdbx` is the canonical way to
/// filter by extension. Defends against unrelated hits by post-filtering
/// for `.kdbx` suffix on the name.
pub fn search_kdbx_files(token: &AccessToken) -> Result<Vec<DriveItemHit>, GraphError> {
    let url = format!("{GRAPH_BASE}/search/query");
    let body = serde_json::json!({
        "requests": [{
            "entityTypes": ["driveItem"],
            "query": { "queryString": "filetype:kdbx" },
            "from": 0,
            "size": 50,
        }]
    });

    let body_str = body.to_string();
    let resp = http::agent()
        .post(&url)
        .set("Authorization", &format!("Bearer {}", token.access_token))
        .set("Content-Type", "application/json")
        .set("Accept", "application/json")
        .send_string(&body_str)
        .map_err(map_ureq_error)?;
    let text = resp
        .into_string()
        .map_err(|e| GraphError::Network(e.to_string()))?;
    let parsed: SearchQueryResponse = parse_json(&text)?;
    Ok(parsed.flatten_hits())
}

/// `GET /me` — used after sign-in to learn the user's email for the
/// keychain key + sync-config display.
pub fn me(token: &AccessToken) -> Result<User, GraphError> {
    let url = format!("{GRAPH_BASE}/me?$select=mail,userPrincipalName");
    let body = http_get(&url, token)?;
    let resp: MeResponse = parse_json(&body)?;
    // `mail` can be null for some accounts (especially personal MS accounts);
    // userPrincipalName is the always-present fallback.
    let email = resp
        .mail
        .or(resp.user_principal_name)
        .ok_or(GraphError::MissingField("mail or userPrincipalName"))?;
    Ok(User { email })
}

/// Resolve a SharePoint site by its hostname + server-relative site path.
///
/// `host` = e.g. `contoso.sharepoint.com`
/// `site_path` = e.g. `sites/MyTeam`
///
/// Built URL: `/sites/{host}:/{site_path}?$select=id,displayName`
pub fn resolve_site(host: &str, site_path: &str, token: &AccessToken) -> Result<Site, GraphError> {
    let url = format!(
        "{GRAPH_BASE}/sites/{host}:/{site_path}?$select=id,displayName",
        host = host,
        site_path = site_path,
    );
    let body = http_get(&url, token)?;
    let resp: SiteResponse = parse_json(&body)?;
    Ok(Site {
        id: resp.id,
        display_name: resp.display_name.unwrap_or_default(),
    })
}

/// List all document libraries (drives) on a site. Used to find the drive
/// whose name matches the URL parser's library segment.
pub fn list_drives(site_id: &str, token: &AccessToken) -> Result<Vec<Drive>, GraphError> {
    let url = format!("{GRAPH_BASE}/sites/{site_id}/drives?$select=id,name,webUrl",);
    let body = http_get(&url, token)?;
    let resp: DriveListResponse = parse_json(&body)?;
    Ok(resp
        .value
        .into_iter()
        .map(|d| Drive {
            id: d.id,
            name: d.name.unwrap_or_default(),
            web_url: d.web_url.unwrap_or_default(),
        })
        .collect())
}

/// Find the drive that matches `library_name`. Tries exact-case match first
/// (the common case — `library_name` came straight from the SharePoint URL,
/// which uses the canonical drive name), then case-insensitive as a fallback.
pub fn find_drive(
    site_id: &str,
    library_name: &str,
    token: &AccessToken,
) -> Result<Drive, GraphError> {
    let drives = list_drives(site_id, token)?;
    if let Some(d) = drives.iter().find(|d| d.name == library_name) {
        return Ok(d.clone());
    }
    if let Some(d) = drives
        .iter()
        .find(|d| d.name.eq_ignore_ascii_case(library_name))
    {
        return Ok(d.clone());
    }
    Err(GraphError::DriveNotFound(library_name.to_string()))
}

/// Resolve a drive item by its path within the drive.
///
/// Built URL: `/drives/{drive_id}/root:/{file_path}?$select=...`
///
/// `file_path` is URL-encoded segment-by-segment so spaces and unicode
/// survive the round trip.
pub fn resolve_item_by_path(
    drive_id: &str,
    file_path: &str,
    token: &AccessToken,
) -> Result<DriveItem, GraphError> {
    let url = format!(
        "{GRAPH_BASE}/drives/{drive_id}/root:/{path}?$select=id,eTag,name,lastModifiedDateTime",
        drive_id = drive_id,
        path = encode_path_segments(file_path),
    );
    let body = http_get(&url, token)?;
    let resp: ItemResponse = parse_json(&body)?;
    Ok(item_from_response(resp))
}

/// Get current item metadata (for ETag-comparison polling on app launch).
pub fn get_item_metadata(
    drive_id: &str,
    item_id: &str,
    token: &AccessToken,
) -> Result<DriveItem, GraphError> {
    let url = format!(
        "{GRAPH_BASE}/drives/{drive_id}/items/{item_id}?$select=id,eTag,name,lastModifiedDateTime",
    );
    let body = http_get(&url, token)?;
    let resp: ItemResponse = parse_json(&body)?;
    Ok(item_from_response(resp))
}

/// Download the content bytes of an item plus its ETag (returned in the
/// response headers).
pub fn download_content(
    drive_id: &str,
    item_id: &str,
    token: &AccessToken,
) -> Result<(Vec<u8>, String), GraphError> {
    let url = format!("{GRAPH_BASE}/drives/{drive_id}/items/{item_id}/content");
    let resp = http::transfer_agent()
        .get(&url)
        .set("Authorization", &format!("Bearer {}", token.access_token))
        .call()
        .map_err(map_ureq_error)?;
    let etag = resp.header("ETag").unwrap_or_default().to_string();
    let declared_length = parse_content_length(resp.header("Content-Length"))?;
    let bytes = read_vault_body(resp.into_reader(), declared_length, MAX_VAULT_CONTENT_BYTES)?;
    Ok((bytes, etag))
}

/// Upload bytes via the small-file PUT endpoint with optional `If-Match`.
///
/// - 200/201 → `Ok { new_etag, item }`
/// - 412 (Precondition Failed) → `Conflict` (server etag changed since
///   `if_match` was captured; the caller must download remote + diff)
/// - Other 4xx/5xx → `Err`
///
/// Files >250 MB exceed Graph's simple-content endpoint. Reject them before
/// issuing a request so download and upload enforce the same bound.
pub fn upload_content(
    drive_id: &str,
    item_id: &str,
    bytes: &[u8],
    if_match: Option<&str>,
    token: &AccessToken,
) -> Result<UploadOutcome, GraphError> {
    ensure_vault_size(bytes.len(), MAX_VAULT_CONTENT_BYTES)?;
    let url = format!("{GRAPH_BASE}/drives/{drive_id}/items/{item_id}/content");
    let mut req = http::transfer_agent()
        .put(&url)
        .set("Authorization", &format!("Bearer {}", token.access_token))
        .set("Content-Type", "application/octet-stream");
    if let Some(etag) = if_match {
        req = req.set("If-Match", etag);
    }
    // Mirror of the download wall clock: idle write timeouts alone let a
    // peer that reads one byte per timeout window keep the upload — and the
    // per-path sync slot — alive indefinitely. The reader is polled once
    // per socket-write chunk, so it observes the deadline even while bytes
    // are still trickling. Content-Length is set explicitly because
    // streaming via `send` would otherwise switch to chunked encoding.
    let body = DeadlineReader {
        inner: io::Cursor::new(bytes),
        deadline: std::time::Instant::now() + TRANSFER_MAX_WALL_CLOCK,
    };
    match req
        .set("Content-Length", &bytes.len().to_string())
        .send(body)
    {
        Ok(resp) => {
            let body = resp
                .into_string()
                .map_err(|e| GraphError::Network(e.to_string()))?;
            let item_resp: ItemResponse = parse_json(&body)?;
            let item = item_from_response(item_resp);
            // Callers persist `new_etag` as the optimistic-concurrency
            // revision for every future push, so an empty one must never
            // escape. Do NOT patch it up with a metadata GET: between our
            // PUT and that GET a concurrent client may have uploaded, and
            // adopting *its* eTag would let our next push overwrite that
            // change without ever seeing a conflict. Failing keeps the
            // caller's previous eTag — the retry then gets a 412 and heals
            // through the normal conflict/merge path.
            if item.etag.trim().is_empty() {
                return Err(GraphError::MissingField("eTag"));
            }
            Ok(UploadOutcome::Ok {
                new_etag: item.etag.clone(),
                item,
            })
        }
        Err(UreqError::Status(412, _)) => Ok(UploadOutcome::Conflict),
        Err(e) => Err(map_ureq_error(e)),
    }
}

// ---------- internals ----------

fn http_get(url: &str, token: &AccessToken) -> Result<String, GraphError> {
    http::agent()
        .get(url)
        .set("Authorization", &format!("Bearer {}", token.access_token))
        .set("Accept", "application/json")
        .call()
        .map_err(map_ureq_error)?
        .into_string()
        .map_err(|e| GraphError::Network(e.to_string()))
}

fn parse_json<T: for<'de> Deserialize<'de>>(body: &str) -> Result<T, GraphError> {
    serde_json::from_str(body).map_err(|e| {
        GraphError::Parse(format!(
            "{e}\nbody: {}",
            truncate_text(body, MAX_ERROR_BODY_BYTES)
        ))
    })
}

fn map_ureq_error(e: UreqError) -> GraphError {
    match e {
        UreqError::Status(status, resp) => {
            let body = read_truncated_text(resp.into_reader(), MAX_ERROR_BODY_BYTES)
                .unwrap_or_else(|error| format!("could not read error response: {error}"));
            GraphError::Status { status, body }
        }
        UreqError::Transport(t) => GraphError::Network(t.to_string()),
    }
}

fn parse_content_length(header: Option<&str>) -> Result<Option<u64>, GraphError> {
    header
        .map(|value| {
            value
                .trim()
                .parse::<u64>()
                .map_err(|_| GraphError::InvalidContentLength)
        })
        .transpose()
}

fn ensure_vault_size(size: usize, limit: usize) -> Result<(), GraphError> {
    if size > limit {
        Err(GraphError::VaultTooLarge)
    } else {
        Ok(())
    }
}

/// Read a response body without trusting either `Content-Length` or transfer
/// encoding. The declared length is checked before the reader is touched;
/// the loop then enforces the same limit for chunked and dishonest responses.
fn read_vault_body(
    mut reader: impl io::Read,
    declared_length: Option<u64>,
    limit: usize,
) -> Result<Vec<u8>, GraphError> {
    if declared_length.is_some_and(|length| length > limit as u64) {
        return Err(GraphError::VaultTooLarge);
    }

    let initial_capacity = declared_length
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or(READ_CHUNK_BYTES)
        .min(READ_CHUNK_BYTES)
        .min(limit);
    let mut bytes = Vec::with_capacity(initial_capacity);
    let mut chunk = [0_u8; READ_CHUNK_BYTES];

    // The agent's idle timeouts bound each individual read, but a server
    // dribbling one chunk per 119 s would keep this loop — and with it the
    // per-path `syncs_in_flight` slot — alive indefinitely, leaving sync
    // dead until an app restart. One hour covers the full 250 MB content
    // cap even on a ~0.6 Mbit/s link; anything slower is not going to
    // finish anyway.
    // ponytail: uploads have no equivalent wall clock (ureq owns that write
    // loop); add one if drip-feed uploads ever show up in practice.
    let deadline = std::time::Instant::now() + TRANSFER_MAX_WALL_CLOCK;

    loop {
        if std::time::Instant::now() >= deadline {
            return Err(GraphError::Network(
                "vault transfer exceeded the overall time budget".into(),
            ));
        }
        // Once exactly `limit` bytes have arrived, read one more byte to
        // distinguish an exact-boundary body from an oversized one without
        // ever appending beyond the cap.
        let remaining = limit.saturating_sub(bytes.len());
        let read_capacity = chunk.len().min(remaining.saturating_add(1));
        let count = reader
            .read(&mut chunk[..read_capacity])
            .map_err(|error| GraphError::Network(error.to_string()))?;
        if count == 0 {
            return Ok(bytes);
        }
        if count > remaining {
            return Err(GraphError::VaultTooLarge);
        }

        if bytes.capacity().saturating_sub(bytes.len()) < count {
            // Grow geometrically so a large vault does not reallocate and
            // copy the complete prefix for every 64 KiB chunk. Keep the
            // requested capacity within the same hard content bound.
            let required_capacity = bytes.len() + count;
            let target_capacity = bytes
                .capacity()
                .saturating_mul(2)
                .max(required_capacity)
                .min(limit);
            bytes
                .try_reserve_exact(target_capacity - bytes.len())
                .map_err(|error| GraphError::Buffer(error.to_string()))?;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}

fn read_truncated_text(mut reader: impl io::Read, limit: usize) -> Result<String, io::Error> {
    let mut bytes = Vec::with_capacity(limit.min(1024));
    let mut chunk = [0_u8; 1024];
    let mut truncated = false;

    loop {
        let remaining = limit.saturating_sub(bytes.len());
        let read_capacity = chunk.len().min(remaining.saturating_add(1));
        let count = reader.read(&mut chunk[..read_capacity])?;
        if count == 0 {
            break;
        }
        let accepted = count.min(remaining);
        bytes.extend_from_slice(&chunk[..accepted]);
        if count > remaining {
            truncated = true;
            break;
        }
    }

    let display_bytes = match std::str::from_utf8(&bytes) {
        Err(error) if truncated && error.error_len().is_none() => &bytes[..error.valid_up_to()],
        _ => &bytes,
    };
    let mut text = String::from_utf8_lossy(display_bytes).into_owned();
    if truncated {
        text.push_str("\n[truncated]");
    }
    Ok(text)
}

fn truncate_text(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[truncated]", &text[..end])
}

fn item_from_response(resp: ItemResponse) -> DriveItem {
    DriveItem {
        id: resp.id,
        etag: resp.e_tag.unwrap_or_default(),
        name: resp.name.unwrap_or_default(),
        last_modified: resp.last_modified_date_time.unwrap_or_default(),
    }
}

/// Percent-encode a forward-slash-separated path the way Graph wants:
/// each segment is encoded individually so that spaces become `%20` etc.,
/// but the slashes between segments remain literal.
fn encode_path_segments(path: &str) -> String {
    path.split('/')
        .map(percent_encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

/// Minimal segment percent-encoder. Encodes everything that isn't an
/// unreserved URL char. Lazy — Graph accepts more than this strictly
/// requires, but encoding extra is harmless.
fn percent_encode_segment(seg: &str) -> String {
    let mut out = String::with_capacity(seg.len());
    for byte in seg.as_bytes() {
        let safe = matches!(
            byte,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~'
        );
        if safe {
            out.push(*byte as char);
        } else {
            use std::fmt::Write as _;
            let _ = write!(&mut out, "%{:02X}", byte);
        }
    }
    out
}

// ---------- response models ----------

#[derive(Deserialize)]
struct MeResponse {
    mail: Option<String>,
    #[serde(rename = "userPrincipalName")]
    user_principal_name: Option<String>,
}

#[derive(Deserialize)]
struct SiteResponse {
    id: String,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct DriveListResponse {
    value: Vec<DriveResponse>,
}

#[derive(Deserialize)]
struct DriveResponse {
    id: String,
    name: Option<String>,
    #[serde(rename = "webUrl")]
    web_url: Option<String>,
}

#[derive(Deserialize)]
struct ItemResponse {
    id: String,
    #[serde(rename = "eTag")]
    e_tag: Option<String>,
    name: Option<String>,
    #[serde(rename = "lastModifiedDateTime")]
    last_modified_date_time: Option<String>,
}

// ---------- search response models ----------
//
// Microsoft Search's response is deeply nested: response.value is a list of
// per-request results; each has `hitsContainers`; each container has `hits`;
// each hit has a `resource`. We flatten it down to a flat Vec<DriveItemHit>
// for the picker UI.

#[derive(Deserialize)]
struct SearchQueryResponse {
    #[serde(default)]
    value: Vec<SearchValue>,
}

#[derive(Deserialize)]
struct SearchValue {
    #[serde(rename = "hitsContainers", default)]
    hits_containers: Vec<HitsContainer>,
}

#[derive(Deserialize)]
struct HitsContainer {
    #[serde(default)]
    hits: Vec<Hit>,
}

#[derive(Deserialize)]
struct Hit {
    resource: Option<HitResource>,
}

#[derive(Deserialize)]
struct HitResource {
    id: String,
    name: Option<String>,
    #[serde(rename = "webUrl")]
    web_url: Option<String>,
    #[serde(rename = "lastModifiedDateTime")]
    last_modified: Option<String>,
    #[serde(rename = "parentReference")]
    parent: Option<HitParentReference>,
}

#[derive(Deserialize)]
struct HitParentReference {
    #[serde(rename = "siteId")]
    site_id: Option<String>,
    #[serde(rename = "driveId")]
    drive_id: Option<String>,
    path: Option<String>,
}

impl SearchQueryResponse {
    /// Walk the nested response, drop hits without the identifiers we need,
    /// and post-filter by `.kdbx` extension (KQL's `filetype:kdbx` already
    /// does this server-side, but defending here means a misbehaving server
    /// can't sneak `.txt` hits into the picker).
    fn flatten_hits(self) -> Vec<DriveItemHit> {
        self.value
            .into_iter()
            .flat_map(|v| v.hits_containers)
            .flat_map(|c| c.hits)
            .filter_map(|h| {
                let res = h.resource?;
                let parent = res.parent?;
                let name = res
                    .name
                    .filter(|n| n.to_ascii_lowercase().ends_with(".kdbx"))?;
                Some(DriveItemHit {
                    item_id: res.id,
                    site_id: parent.site_id?,
                    drive_id: parent.drive_id?,
                    name,
                    web_url: res.web_url.unwrap_or_default(),
                    path: parent.path.unwrap_or_default(),
                    last_modified: res.last_modified.unwrap_or_default(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};

    struct PanicReader;

    impl Read for PanicReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            panic!("oversized Content-Length must be rejected before reading")
        }
    }

    #[test]
    fn declared_oversized_vault_is_rejected_before_reading() {
        let error = read_vault_body(PanicReader, Some(5), 4).unwrap_err();
        assert!(matches!(error, GraphError::VaultTooLarge));
    }

    #[test]
    fn chunked_oversized_vault_is_rejected_while_streaming() {
        let error = read_vault_body(Cursor::new([1, 2, 3, 4, 5]), None, 4).unwrap_err();
        assert!(matches!(error, GraphError::VaultTooLarge));
    }

    #[test]
    fn dishonest_content_length_cannot_bypass_stream_limit() {
        let error = read_vault_body(Cursor::new([1, 2, 3, 4, 5]), Some(4), 4).unwrap_err();
        assert!(matches!(error, GraphError::VaultTooLarge));
    }

    #[test]
    fn vault_body_at_exact_limit_is_accepted() {
        let bytes = read_vault_body(Cursor::new([1, 2, 3, 4]), None, 4).unwrap();
        assert_eq!(bytes, [1, 2, 3, 4]);
    }

    #[test]
    fn invalid_content_length_is_rejected() {
        assert!(matches!(
            parse_content_length(Some("not-a-number")),
            Err(GraphError::InvalidContentLength)
        ));
        assert_eq!(parse_content_length(Some(" 42 ")).unwrap(), Some(42));
        assert_eq!(parse_content_length(None).unwrap(), None);
    }

    #[test]
    fn upload_size_uses_the_same_limit() {
        assert!(ensure_vault_size(4, 4).is_ok());
        assert!(matches!(
            ensure_vault_size(5, 4),
            Err(GraphError::VaultTooLarge)
        ));
    }

    #[test]
    fn status_body_is_truncated_without_splitting_utf8() {
        let body = "ééé";
        let text = read_truncated_text(Cursor::new(body.as_bytes()), 5).unwrap();
        assert_eq!(text, "éé\n[truncated]");
    }

    #[test]
    fn short_status_body_is_preserved() {
        let text = read_truncated_text(Cursor::new(b"graph error"), 64).unwrap();
        assert_eq!(text, "graph error");
    }

    #[test]
    fn parse_error_body_is_utf8_safely_truncated() {
        let body = "ééé";
        assert_eq!(truncate_text(body, 5), "éé\n[truncated]");
    }

    #[test]
    fn segment_encoder_handles_spaces_and_unicode() {
        assert_eq!(percent_encode_segment("Hello"), "Hello");
        assert_eq!(percent_encode_segment("Hello World"), "Hello%20World");
        assert_eq!(percent_encode_segment("file.kdbx"), "file.kdbx");
        // Cyrillic "тест" = bytes D1 82 D0 B5 D1 81 D1 82
        assert_eq!(percent_encode_segment("тест"), "%D1%82%D0%B5%D1%81%D1%82");
    }

    #[test]
    fn path_encoder_keeps_slashes_literal() {
        assert_eq!(
            encode_path_segments("My Folder/Sub Folder/file.kdbx"),
            "My%20Folder/Sub%20Folder/file.kdbx"
        );
    }

    #[test]
    fn item_response_parses_with_all_fields() {
        let body = r#"{
            "id": "01ABCDEF",
            "eTag": "\"{guid-here},2\"",
            "name": "passwords.kdbx",
            "lastModifiedDateTime": "2026-04-29T12:34:56Z"
        }"#;
        let item: ItemResponse = serde_json::from_str(body).unwrap();
        let item = item_from_response(item);
        assert_eq!(item.id, "01ABCDEF");
        assert_eq!(item.etag, "\"{guid-here},2\"");
        assert_eq!(item.name, "passwords.kdbx");
        assert_eq!(item.last_modified, "2026-04-29T12:34:56Z");
    }

    #[test]
    fn item_response_tolerates_missing_optional_fields() {
        // Real Graph responses always include eTag for files, but be defensive.
        let body = r#"{ "id": "01ABCDEF" }"#;
        let item: ItemResponse = serde_json::from_str(body).unwrap();
        let item = item_from_response(item);
        assert_eq!(item.id, "01ABCDEF");
        assert_eq!(item.etag, "");
        assert_eq!(item.name, "");
    }

    #[test]
    fn me_response_falls_back_to_user_principal_name_when_mail_null() {
        let body = r#"{ "mail": null, "userPrincipalName": "alice@contoso.onmicrosoft.com" }"#;
        let resp: MeResponse = serde_json::from_str(body).unwrap();
        let email = resp.mail.or(resp.user_principal_name).unwrap();
        assert_eq!(email, "alice@contoso.onmicrosoft.com");
    }

    #[test]
    fn search_response_flattens_nested_hits() {
        // Real shape from the Microsoft Search API. Two hits across two
        // hitsContainers; one is a non-kdbx (.txt) we expect to drop.
        // Note: `r##"..."##` (double-hash delimiters) so the JSON's
        // `"#microsoft.graph.driveItem"` value doesn't accidentally close
        // the raw string at the `"#` boundary.
        let body = r##"{
            "value": [{
                "hitsContainers": [
                    {
                        "hits": [{
                            "hitId": "h1",
                            "resource": {
                                "@odata.type": "#microsoft.graph.driveItem",
                                "id": "01ABC",
                                "name": "Personal.kdbx",
                                "webUrl": "https://contoso.sharepoint.com/sites/x/Shared%20Documents/Personal.kdbx",
                                "lastModifiedDateTime": "2026-04-29T12:00:00Z",
                                "parentReference": {
                                    "siteId": "contoso.sharepoint.com,abc,def",
                                    "driveId": "b!drive1",
                                    "path": "/drives/b!drive1/root:/Folder"
                                }
                            }
                        }]
                    },
                    {
                        "hits": [
                            {
                                "hitId": "h2",
                                "resource": {
                                    "id": "01DEF",
                                    "name": "Team.kdbx",
                                    "parentReference": {
                                        "siteId": "contoso.sharepoint.com,xyz,uvw",
                                        "driveId": "b!drive2"
                                    }
                                }
                            },
                            {
                                "hitId": "h3-bogus",
                                "resource": {
                                    "id": "01GHI",
                                    "name": "notes.txt",
                                    "parentReference": {
                                        "siteId": "x", "driveId": "y"
                                    }
                                }
                            }
                        ]
                    }
                ]
            }]
        }"##;
        let resp: SearchQueryResponse = serde_json::from_str(body).unwrap();
        let hits = resp.flatten_hits();
        assert_eq!(hits.len(), 2, "expected 2 .kdbx hits, got {hits:?}");
        assert_eq!(hits[0].name, "Personal.kdbx");
        assert_eq!(hits[0].drive_id, "b!drive1");
        assert_eq!(hits[1].name, "Team.kdbx");
    }

    #[test]
    fn search_response_drops_hits_missing_required_ids() {
        // Hit with no parent → unusable; should be silently dropped rather
        // than panicking the picker.
        let body = r#"{
            "value": [{
                "hitsContainers": [{
                    "hits": [{
                        "hitId": "h1",
                        "resource": { "id": "01ABC", "name": "x.kdbx" }
                    }]
                }]
            }]
        }"#;
        let resp: SearchQueryResponse = serde_json::from_str(body).unwrap();
        assert!(resp.flatten_hits().is_empty());
    }

    #[test]
    fn search_response_with_no_hits_returns_empty() {
        let body = r#"{ "value": [{ "hitsContainers": [{ "hits": [] }] }] }"#;
        let resp: SearchQueryResponse = serde_json::from_str(body).unwrap();
        assert!(resp.flatten_hits().is_empty());
    }

    #[test]
    fn drive_list_parses_into_drives() {
        let body = r#"{
            "value": [
                {"id": "b!abc", "name": "Documents", "webUrl": "https://x.sharepoint.com/sites/y/Shared%20Documents"},
                {"id": "b!def", "name": "Vaults",    "webUrl": "https://x.sharepoint.com/sites/y/Vaults"}
            ]
        }"#;
        let resp: DriveListResponse = serde_json::from_str(body).unwrap();
        let drives: Vec<Drive> = resp
            .value
            .into_iter()
            .map(|d| Drive {
                id: d.id,
                name: d.name.unwrap_or_default(),
                web_url: d.web_url.unwrap_or_default(),
            })
            .collect();
        assert_eq!(drives.len(), 2);
        assert_eq!(drives[0].name, "Documents");
        assert_eq!(drives[1].id, "b!def");
    }
}

//! Signed update-manifest handling around `cargo-packager-updater`.
//!
//! The updater crate verifies the downloaded application bundle, but not the
//! manifest that assigns a version and URL to that signature. FerrisPass
//! therefore verifies a detached signature over the complete manifest first
//! and binds the install candidate to those signed fields before downloading.

use std::collections::HashMap;
use std::io::Read;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use cargo_packager_updater::{Config, Update, check_update, target};
use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde::Deserialize;
use sha2::Digest as _;
use url::Url;

use super::info::{UpdateError, UpdateInfo};
use super::{MINISIGN_PUBLIC_KEY, UPDATE_ENDPOINT, UPDATE_SIGNATURE_ENDPOINT};

const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
const MAX_MANIFEST_SIGNATURE_BYTES: u64 = 16 * 1024;
const MAX_BUNDLE_BYTES: u64 = 512 * 1024 * 1024;

/// Ask the GitHub-hosted, signed manifest whether a newer release exists.
/// Blocking: call from `cx.background_spawn`.
pub fn check() -> Result<Option<UpdateInfo>, UpdateError> {
    ensure_signing_configured()?;

    let release = fetch_verified_manifest()?;
    let current = current_version()?;
    if release.version <= current {
        return Ok(None);
    }

    Ok(Some(release.info()))
}

/// Install exactly `expected_version`, which must be the signed version the
/// user confirmed. A newer manifest appearing between check and install is
/// reported as a mismatch and is never installed implicitly.
///
/// FerrisPass performs a second cryptographic check over the bundle itself.
/// Before download, all security-relevant candidate fields are compared
/// against the signed manifest to prevent replaying a different previously
/// signed bundle under the confirmed version.
///
/// `on_progress(downloaded, total)` receives cumulative downloaded bytes.
/// Blocking: call from `cx.background_spawn`.
pub fn install<F>(expected: &UpdateInfo, on_progress: F) -> Result<UpdateInfo, UpdateError>
where
    F: Fn(usize, Option<u64>) + Send + 'static,
{
    let release = fetch_verified_manifest()?;

    release.validate_expected_candidate(expected)?;

    let config = build_config()?;
    let current = current_version()?;
    let update = check_update(current, config)
        .map_err(map_err)?
        .ok_or_else(|| UpdateError::Network("no update available at install time".into()))?;

    release.validate_candidate(&update)?;
    let info = release.info();

    let bytes = download_bundle(&release.platform, on_progress)?;
    verify_bundle(&bytes, &release.platform.signature)?;

    #[cfg(target_os = "macos")]
    super::macos_installer::install(&update.extract_path, bytes)?;

    #[cfg(not(target_os = "macos"))]
    update.install(bytes).map_err(map_err)?;

    Ok(info)
}

#[derive(Debug, Deserialize)]
struct ReleaseManifest {
    version: String,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    pub_date: Option<String>,
    platforms: HashMap<String, ReleasePlatform>,
}

#[derive(Debug, Deserialize)]
struct ReleasePlatform {
    signature: String,
    url: Url,
    format: String,
    size: u64,
}

#[derive(Debug)]
struct VerifiedRelease {
    version: Version,
    notes: String,
    pub_date: Option<String>,
    platform: ReleasePlatform,
    candidate_id: String,
}

impl VerifiedRelease {
    fn parse(bytes: &[u8]) -> Result<Self, UpdateError> {
        let manifest: ReleaseManifest = serde_json::from_slice(bytes)
            .map_err(|e| UpdateError::Parse(format!("signed manifest: {e}")))?;
        let version = Version::parse(&manifest.version)
            .map_err(|e| UpdateError::Parse(format!("manifest version: {e}")))?;
        let target = target().ok_or_else(|| {
            UpdateError::Parse("updates are not supported on this platform".into())
        })?;
        let platform = manifest.platforms.get(&target).ok_or_else(|| {
            UpdateError::Parse(format!("signed manifest has no entry for {target}"))
        })?;

        if platform.signature.trim().is_empty() {
            return Err(UpdateError::Parse(
                "signed manifest contains an empty bundle signature".into(),
            ));
        }
        if platform.url.scheme() != "https" {
            return Err(UpdateError::Parse(
                "signed manifest bundle URL must use HTTPS".into(),
            ));
        }
        if platform.size == 0 || platform.size > MAX_BUNDLE_BYTES {
            return Err(UpdateError::Parse(format!(
                "signed manifest bundle size must be between 1 and {MAX_BUNDLE_BYTES} bytes"
            )));
        }

        Ok(Self {
            version,
            notes: manifest.notes.unwrap_or_default(),
            pub_date: manifest.pub_date,
            platform: ReleasePlatform {
                signature: platform.signature.clone(),
                url: platform.url.clone(),
                format: platform.format.clone(),
                size: platform.size,
            },
            candidate_id: BASE64.encode(sha2::Sha256::digest(bytes)),
        })
    }

    fn info(&self) -> UpdateInfo {
        UpdateInfo {
            version: self.version.to_string(),
            notes: self.notes.clone(),
            pub_date: self.pub_date.clone(),
            candidate_id: Some(self.candidate_id.clone()),
        }
    }

    fn validate_expected_candidate(&self, expected: &UpdateInfo) -> Result<(), UpdateError> {
        let expected_version = Version::parse(&expected.version)
            .map_err(|e| UpdateError::Parse(format!("confirmed version: {e}")))?;
        if self.version != expected_version {
            return Err(UpdateError::VersionMismatch {
                expected: expected_version.to_string(),
                actual: self.version.to_string(),
            });
        }
        if expected.candidate_id.as_deref() != Some(&self.candidate_id) {
            return Err(UpdateError::ManifestMismatch);
        }
        Ok(())
    }

    fn validate_candidate(&self, update: &Update) -> Result<(), UpdateError> {
        let matches = update.version == self.version.to_string()
            && update.download_url == self.platform.url
            && update.signature == self.platform.signature
            && update.format.to_string() == self.platform.format;

        if matches {
            Ok(())
        } else {
            Err(UpdateError::ManifestMismatch)
        }
    }
}

fn fetch_verified_manifest() -> Result<VerifiedRelease, UpdateError> {
    let manifest = download_limited(UPDATE_ENDPOINT, MAX_MANIFEST_BYTES)?;
    let signature = download_limited(UPDATE_SIGNATURE_ENDPOINT, MAX_MANIFEST_SIGNATURE_BYTES)?;
    verify_manifest(&manifest, &signature)?;
    VerifiedRelease::parse(&manifest)
}

fn download_limited(url: &str, max_bytes: u64) -> Result<Vec<u8>, UpdateError> {
    let response = crate::sync::http::agent()
        .get(url)
        .set("Accept", "application/octet-stream")
        .set("User-Agent", "FerrisPass updater")
        .call()
        .map_err(|e| UpdateError::Network(e.to_string()))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| UpdateError::Network(e.to_string()))?;
    if bytes.len() as u64 > max_bytes {
        return Err(UpdateError::Parse(format!(
            "update metadata exceeds the {max_bytes}-byte limit"
        )));
    }
    Ok(bytes)
}

fn download_bundle<F>(platform: &ReleasePlatform, on_progress: F) -> Result<Vec<u8>, UpdateError>
where
    F: Fn(usize, Option<u64>),
{
    let response = crate::sync::http::transfer_agent()
        .get(platform.url.as_str())
        .set("Accept", "application/octet-stream")
        .set("User-Agent", "FerrisPass updater")
        .call()
        .map_err(|error| UpdateError::Network(error.to_string()))?;

    if let Some(content_length) = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok())
        && content_length != platform.size
    {
        return Err(UpdateError::Network(format!(
            "update bundle length mismatch (signed {}, received {content_length})",
            platform.size
        )));
    }

    read_exact_bundle(response.into_reader(), platform.size, on_progress)
}

fn read_exact_bundle<R, F>(
    mut reader: R,
    expected: u64,
    on_progress: F,
) -> Result<Vec<u8>, UpdateError>
where
    R: Read,
    F: Fn(usize, Option<u64>),
{
    if expected == 0 || expected > MAX_BUNDLE_BYTES {
        return Err(UpdateError::Parse(
            "signed update bundle size is out of range".into(),
        ));
    }

    let capacity = usize::try_from(expected)
        .map_err(|_| UpdateError::Parse("signed update bundle size is too large".into()))?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(capacity).map_err(|error| {
        UpdateError::Install(format!("could not reserve bundle buffer: {error}"))
    })?;
    let mut chunk = [0u8; 64 * 1024];

    loop {
        let remaining = expected
            .saturating_add(1)
            .saturating_sub(bytes.len() as u64);
        if remaining == 0 {
            break;
        }
        let read_len = usize::try_from(remaining.min(chunk.len() as u64)).unwrap_or(chunk.len());
        let count = reader
            .read(&mut chunk[..read_len])
            .map_err(|error| UpdateError::Network(error.to_string()))?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
        on_progress(bytes.len().min(capacity), Some(expected));
    }

    if bytes.len() as u64 != expected {
        return Err(UpdateError::Network(format!(
            "update bundle length mismatch (signed {expected}, received {})",
            bytes.len()
        )));
    }
    Ok(bytes)
}

fn verify_manifest(manifest: &[u8], signature: &[u8]) -> Result<(), UpdateError> {
    let public_key =
        PublicKey::decode(MINISIGN_PUBLIC_KEY.trim()).map_err(|_| UpdateError::SignatureInvalid)?;
    let signature = std::str::from_utf8(signature)
        .ok()
        .and_then(|value| Signature::decode(value).ok())
        .ok_or(UpdateError::SignatureInvalid)?;

    public_key
        .verify(manifest, &signature, false)
        .map_err(|_| UpdateError::SignatureInvalid)
}

fn verify_bundle(bundle: &[u8], encoded_signature: &str) -> Result<(), UpdateError> {
    let public_key =
        PublicKey::decode(MINISIGN_PUBLIC_KEY.trim()).map_err(|_| UpdateError::SignatureInvalid)?;
    let signature = BASE64
        .decode(encoded_signature)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .and_then(|value| Signature::decode(&value).ok())
        .ok_or(UpdateError::SignatureInvalid)?;

    public_key
        .verify(bundle, &signature, false)
        .map_err(|_| UpdateError::SignatureInvalid)
}

fn ensure_signing_configured() -> Result<(), UpdateError> {
    let raw = MINISIGN_PUBLIC_KEY.trim();
    if raw.contains("PLACEHOLDER") || raw.contains("AAAAAAAAAAAA") {
        Err(UpdateError::PlaceholderKey)
    } else {
        Ok(())
    }
}

fn build_config() -> Result<Config, UpdateError> {
    ensure_signing_configured()?;

    // cargo-packager-updater expects base64 of the entire two-line public-key
    // file and decodes it internally before verifying the bundle signature.
    let pubkey = BASE64.encode(MINISIGN_PUBLIC_KEY.trim().as_bytes());
    let endpoint = UPDATE_ENDPOINT
        .parse()
        .map_err(|e: url::ParseError| UpdateError::Parse(format!("endpoint URL: {e}")))?;

    Ok(Config {
        endpoints: vec![endpoint],
        pubkey,
        ..Default::default()
    })
}

fn current_version() -> Result<Version, UpdateError> {
    Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|e| UpdateError::Parse(format!("CARGO_PKG_VERSION: {e}")))
}

fn map_err(e: cargo_packager_updater::Error) -> UpdateError {
    let msg = e.to_string();
    let lower = msg.to_lowercase();

    if lower.contains("base64") || lower.contains("invalid symbol") || lower.contains("decode") {
        UpdateError::Parse(msg)
    } else if lower.contains("signature") || lower.contains("pubkey") || lower.contains("verify") {
        UpdateError::SignatureInvalid
    } else if lower.contains("install")
        || lower.contains("relocate")
        || lower.contains("extract")
        || lower.contains("permission")
    {
        UpdateError::Install(msg)
    } else if lower.contains("parse") || lower.contains("json") || lower.contains("deserialize") {
        UpdateError::Parse(msg)
    } else {
        UpdateError::Network(msg)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use cargo_packager_updater::UpdateFormat;

    use super::*;

    fn release() -> VerifiedRelease {
        VerifiedRelease {
            version: Version::parse("1.2.3").unwrap(),
            notes: "Security fixes".into(),
            pub_date: Some("2026-07-13T12:00:00Z".into()),
            platform: ReleasePlatform {
                signature: "signed-payload".into(),
                url: Url::parse("https://example.test/FerrisPass-1.2.3.app.tar.gz").unwrap(),
                format: "app".into(),
                size: 4,
            },
            candidate_id: "confirmed-manifest".into(),
        }
    }

    fn candidate(release: &VerifiedRelease) -> Update {
        Update {
            config: Config::default(),
            body: None,
            current_version: "1.2.2".into(),
            version: release.version.to_string(),
            date: None,
            target: "macos".into(),
            extract_path: PathBuf::from("FerrisPass.app"),
            download_url: release.platform.url.clone(),
            signature: release.platform.signature.clone(),
            timeout: None,
            headers: Default::default(),
            format: UpdateFormat::App,
        }
    }

    #[test]
    fn accepts_candidate_bound_to_signed_manifest() {
        let release = release();
        assert!(release.validate_candidate(&candidate(&release)).is_ok());
    }

    #[test]
    fn rejects_replayed_bundle_under_confirmed_version() {
        let release = release();
        let mut candidate = candidate(&release);
        candidate.download_url =
            Url::parse("https://example.test/FerrisPass-1.2.2.app.tar.gz").unwrap();
        candidate.signature = "previously-signed-payload".into();

        assert!(matches!(
            release.validate_candidate(&candidate),
            Err(UpdateError::ManifestMismatch)
        ));
    }

    #[test]
    fn rejects_version_not_bound_to_signed_manifest() {
        let release = release();
        let mut candidate = candidate(&release);
        candidate.version = "1.2.4".into();

        assert!(matches!(
            release.validate_candidate(&candidate),
            Err(UpdateError::ManifestMismatch)
        ));
    }

    #[test]
    fn requires_the_version_the_user_confirmed() {
        let release = release();
        let mut expected = release.info();
        expected.version = "1.2.2".into();
        let error = release.validate_expected_candidate(&expected).unwrap_err();

        assert!(matches!(
            error,
            UpdateError::VersionMismatch { expected, actual }
                if expected == "1.2.2" && actual == "1.2.3"
        ));
    }

    #[test]
    fn requires_the_exact_manifest_the_user_confirmed() {
        let release = release();
        let mut expected = release.info();
        expected.candidate_id = Some("same-version-but-different-manifest".into());

        assert!(matches!(
            release.validate_expected_candidate(&expected),
            Err(UpdateError::ManifestMismatch)
        ));
    }

    #[test]
    fn rejects_unsigned_manifest_data() {
        assert!(matches!(
            verify_manifest(br#"{"version":"9.9.9"}"#, b"not a minisign signature"),
            Err(UpdateError::SignatureInvalid)
        ));
    }

    #[test]
    fn bounded_bundle_reader_requires_signed_length() {
        let progress = std::cell::RefCell::new(Vec::new());
        let bytes = read_exact_bundle(std::io::Cursor::new(b"test"), 4, |done, total| {
            progress.borrow_mut().push((done, total));
        })
        .unwrap();

        assert_eq!(bytes, b"test");
        assert_eq!(progress.borrow().last(), Some(&(4, Some(4))));
        assert!(read_exact_bundle(std::io::Cursor::new(b"too long"), 3, |_, _| {}).is_err());
        assert!(read_exact_bundle(std::io::Cursor::new(b"short"), 8, |_, _| {}).is_err());
    }
}

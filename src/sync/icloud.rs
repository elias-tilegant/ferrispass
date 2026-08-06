//! iCloud Drive provider primitives.
//!
//! FerrisPass deliberately uses user-selected iCloud Drive files rather than
//! an app-owned ubiquity container. The selected file is addressed by an
//! ordinary NSURL bookmark so moves and renames survive app restarts. All
//! reads and replacements are coordinated with Foundation on macOS.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ICloudError {
    #[error("iCloud Drive is only available on macOS")]
    Unsupported,
    #[error("the selected item is not a .kdbx file")]
    NotKdbx,
    #[error("the selected item is not stored in iCloud Drive")]
    NotICloud,
    #[error("the iCloud vault no longer exists; select it again")]
    Missing,
    #[error("the iCloud vault changed before it could be published")]
    Conflict,
    #[error("invalid iCloud bookmark: {0}")]
    Bookmark(String),
    #[error("iCloud file coordination failed: {0}")]
    Coordination(String),
    #[error("i/o error for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[derive(Debug)]
pub struct ICloudRead {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub revision: String,
    pub refreshed_bookmark: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ICloudEventKind {
    Changed,
    Removed,
    Other,
}

#[derive(Clone, Debug)]
pub struct ICloudEvent {
    pub path: PathBuf,
    pub kind: ICloudEventKind,
}

/// RAII filesystem presenter. macOS's FSEvents backend reports content,
/// rename and deletion changes without polling. Every event is still
/// revision-checked by the sync engine, so self-generated and duplicate
/// notifications are harmless.
pub struct ICloudWatcher {
    _watcher: RecommendedWatcher,
}

impl std::fmt::Debug for ICloudWatcher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ICloudWatcher")
            .finish_non_exhaustive()
    }
}

impl ICloudWatcher {
    pub fn watch(path: &Path, sender: mpsc::Sender<ICloudEvent>) -> Result<Self, ICloudError> {
        let watched_path = path.to_path_buf();
        let mut watcher =
            notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
                let Ok(event) = result else { return };
                let kind = match event.kind {
                    notify::EventKind::Modify(_) | notify::EventKind::Create(_) => {
                        ICloudEventKind::Changed
                    }
                    notify::EventKind::Remove(_) => ICloudEventKind::Removed,
                    _ => ICloudEventKind::Other,
                };
                let _ = sender.send(ICloudEvent {
                    path: watched_path.clone(),
                    kind,
                });
            })
            .map_err(|error| ICloudError::Coordination(error.to_string()))?;
        watcher
            .watch(path, RecursiveMode::NonRecursive)
            .map_err(|error| ICloudError::Coordination(error.to_string()))?;
        Ok(Self { _watcher: watcher })
    }
}

pub fn revision(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub fn validate_kdbx_path(path: &Path) -> Result<(), ICloudError> {
    if path.extension().and_then(|value| value.to_str()) != Some("kdbx") {
        return Err(ICloudError::NotKdbx);
    }
    Ok(())
}

pub fn create_bookmark(path: &Path) -> Result<String, ICloudError> {
    validate_kdbx_path(path)?;
    platform::create_bookmark(path)
}

pub fn resolve_bookmark(bookmark: &str) -> Result<(PathBuf, bool), ICloudError> {
    platform::resolve_bookmark(bookmark)
}

pub fn is_icloud_item(path: &Path) -> bool {
    platform::is_icloud_item(path)
}

pub fn read(bookmark: &str) -> Result<ICloudRead, ICloudError> {
    let (path, stale) = resolve_bookmark(bookmark)?;
    validate_existing_remote(&path)?;
    platform::request_download(&path)?;
    let bytes = platform::coordinated_read(&path)?;
    crate::keepass::limits::validate_kdbx_size(bytes.len() as u64)
        .map_err(|source| io_error(&path, source))?;
    let refreshed_bookmark = if stale {
        create_bookmark(&path)?
    } else {
        bookmark.to_string()
    };
    Ok(ICloudRead {
        path,
        revision: revision(&bytes),
        bytes,
        refreshed_bookmark,
    })
}

pub fn probe(bookmark: &str, expected_revision: &str) -> Result<bool, ICloudError> {
    Ok(read(bookmark)?.revision != expected_revision)
}

pub fn publish(
    bookmark: &str,
    expected_revision: &str,
    bytes: &[u8],
) -> Result<ICloudRead, ICloudError> {
    let before = read(bookmark)?;
    if before.revision != expected_revision {
        return Err(ICloudError::Conflict);
    }
    platform::coordinated_replace(&before.path, expected_revision, bytes)?;
    read(&before.refreshed_bookmark)
}

/// Publish a new remote without ever replacing a file that appeared after
/// the save panel closed.
pub fn create_remote(path: &Path, bytes: &[u8]) -> Result<ICloudRead, ICloudError> {
    validate_kdbx_path(path)?;
    if path.exists() {
        return Err(io_error(
            path,
            io::Error::new(io::ErrorKind::AlreadyExists, "remote vault already exists"),
        ));
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if !is_icloud_item(parent) {
        return Err(ICloudError::NotICloud);
    }
    crate::keepass::limits::validate_kdbx_size(bytes.len() as u64)
        .map_err(|source| io_error(path, source))?;

    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|source| io_error(path, source))?;
    if let Err(source) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(io_error(path, source));
    }
    drop(file);
    if !is_icloud_item(path) {
        let _ = fs::remove_file(path);
        return Err(ICloudError::NotICloud);
    }
    let bookmark = create_bookmark(path)?;
    read(&bookmark)
}

fn validate_existing_remote(path: &Path) -> Result<(), ICloudError> {
    validate_kdbx_path(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            ICloudError::Missing
        } else {
            io_error(path, source)
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(io_error(
            path,
            io::Error::new(io::ErrorKind::InvalidInput, "remote must be a regular file"),
        ));
    }
    if !is_icloud_item(path) {
        return Err(ICloudError::NotICloud);
    }
    Ok(())
}

fn io_error(path: &Path, source: io::Error) -> ICloudError {
    ICloudError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(target_os = "macos")]
mod platform {
    #![allow(unsafe_code)]

    use std::path::{Path, PathBuf};
    use std::ptr::NonNull;
    use std::sync::{Arc, Mutex};

    use base64::Engine as _;
    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{
        NSData, NSError, NSFileCoordinator, NSFileCoordinatorReadingOptions,
        NSFileCoordinatorWritingOptions, NSFileManager, NSString, NSURLBookmarkCreationOptions,
        NSURLBookmarkResolutionOptions, NSURL,
    };

    use super::{revision, ICloudError};

    fn url(path: &Path) -> objc2::rc::Retained<NSURL> {
        NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
    }

    fn path(url: &NSURL) -> Result<PathBuf, ICloudError> {
        url.path()
            .map(|value| PathBuf::from(value.to_string()))
            .ok_or_else(|| ICloudError::Bookmark("bookmark did not resolve to a file path".into()))
    }

    pub(super) fn create_bookmark(path: &Path) -> Result<String, ICloudError> {
        let data = url(path)
            .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
                NSURLBookmarkCreationOptions::WithoutImplicitSecurityScope,
                None,
                None,
            )
            .map_err(|error| ICloudError::Bookmark(error.localizedDescription().to_string()))?;
        Ok(base64::engine::general_purpose::STANDARD.encode(data.to_vec()))
    }

    pub(super) fn resolve_bookmark(bookmark: &str) -> Result<(PathBuf, bool), ICloudError> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(bookmark)
            .map_err(|error| ICloudError::Bookmark(error.to_string()))?;
        let data = NSData::with_bytes(&bytes);
        let mut stale = Bool::NO;
        let resolved = unsafe {
            NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
                &data,
                NSURLBookmarkResolutionOptions::WithoutUI
                    | NSURLBookmarkResolutionOptions::WithoutImplicitStartAccessing,
                None,
                &mut stale,
            )
        }
        .map_err(|error| ICloudError::Bookmark(error.localizedDescription().to_string()))?;
        Ok((path(&resolved)?, stale.as_bool()))
    }

    pub(super) fn is_icloud_item(path: &Path) -> bool {
        NSFileManager::defaultManager().isUbiquitousItemAtURL(&url(path))
    }

    pub(super) fn request_download(path: &Path) -> Result<(), ICloudError> {
        NSFileManager::defaultManager()
            .startDownloadingUbiquitousItemAtURL_error(&url(path))
            .map_err(|error| ICloudError::Coordination(error.localizedDescription().to_string()))
    }

    pub(super) fn coordinated_read(path_value: &Path) -> Result<Vec<u8>, ICloudError> {
        let target = url(path_value);
        let result = Arc::new(Mutex::new(None));
        let result_for_block = Arc::clone(&result);
        let reader = RcBlock::new(move |coordinated_url: NonNull<NSURL>| {
            let coordinated_url = unsafe { coordinated_url.as_ref() };
            let value = path(coordinated_url).and_then(|path| {
                std::fs::read(&path).map_err(|source| super::io_error(&path, source))
            });
            *result_for_block
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(value);
        });
        let coordinator = NSFileCoordinator::new();
        let mut error: Option<objc2::rc::Retained<NSError>> = None;
        coordinator.coordinateReadingItemAtURL_options_error_byAccessor(
            &target,
            NSFileCoordinatorReadingOptions::WithoutChanges,
            Some(&mut error),
            &reader,
        );
        if let Some(error) = error {
            return Err(ICloudError::Coordination(
                error.localizedDescription().to_string(),
            ));
        }
        result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or_else(|| ICloudError::Coordination("read accessor was not invoked".into()))?
    }

    pub(super) fn coordinated_replace(
        path_value: &Path,
        expected_revision: &str,
        bytes: &[u8],
    ) -> Result<(), ICloudError> {
        let target = url(path_value);
        let expected_revision = expected_revision.to_string();
        let bytes = bytes.to_vec();
        let result = Arc::new(Mutex::new(None));
        let result_for_block = Arc::clone(&result);
        let writer = RcBlock::new(move |coordinated_url: NonNull<NSURL>| {
            let coordinated_url = unsafe { coordinated_url.as_ref() };
            let value = path(coordinated_url).and_then(|target_path| {
                let current = std::fs::read(&target_path)
                    .map_err(|source| super::io_error(&target_path, source))?;
                if revision(&current) != expected_revision {
                    return Err(ICloudError::Conflict);
                }
                let mut temp_name = target_path.as_os_str().to_owned();
                temp_name.push(format!(".ferrispass-{}.tmp", std::process::id()));
                let temp = PathBuf::from(temp_name);
                let write = (|| {
                    use std::io::Write as _;
                    let mut options = std::fs::OpenOptions::new();
                    options.write(true).create_new(true);
                    use std::os::unix::fs::OpenOptionsExt as _;
                    options.mode(0o600);
                    let mut file = options
                        .open(&temp)
                        .map_err(|source| super::io_error(&temp, source))?;
                    file.write_all(&bytes)
                        .and_then(|()| file.sync_all())
                        .map_err(|source| super::io_error(&temp, source))?;
                    std::fs::rename(&temp, &target_path)
                        .map_err(|source| super::io_error(&target_path, source))
                })();
                if write.is_err() {
                    let _ = std::fs::remove_file(&temp);
                }
                write
            });
            *result_for_block
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(value);
        });
        let coordinator = NSFileCoordinator::new();
        let mut error: Option<objc2::rc::Retained<NSError>> = None;
        coordinator.coordinateWritingItemAtURL_options_error_byAccessor(
            &target,
            NSFileCoordinatorWritingOptions::ForReplacing,
            Some(&mut error),
            &writer,
        );
        if let Some(error) = error {
            return Err(ICloudError::Coordination(
                error.localizedDescription().to_string(),
            ));
        }
        result
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or_else(|| ICloudError::Coordination("write accessor was not invoked".into()))?
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use std::path::{Path, PathBuf};

    use super::ICloudError;

    pub(super) fn create_bookmark(_path: &Path) -> Result<String, ICloudError> {
        Err(ICloudError::Unsupported)
    }

    pub(super) fn resolve_bookmark(_bookmark: &str) -> Result<(PathBuf, bool), ICloudError> {
        Err(ICloudError::Unsupported)
    }

    pub(super) fn is_icloud_item(_path: &Path) -> bool {
        false
    }

    pub(super) fn request_download(_path: &Path) -> Result<(), ICloudError> {
        Err(ICloudError::Unsupported)
    }

    pub(super) fn coordinated_read(_path: &Path) -> Result<Vec<u8>, ICloudError> {
        Err(ICloudError::Unsupported)
    }

    pub(super) fn coordinated_replace(
        _path: &Path,
        _expected_revision: &str,
        _bytes: &[u8],
    ) -> Result<(), ICloudError> {
        Err(ICloudError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revisions_are_content_addressed() {
        assert_eq!(revision(b"same"), revision(b"same"));
        assert_ne!(revision(b"same"), revision(b"different"));
    }

    #[test]
    fn validates_kdbx_extension() {
        assert!(validate_kdbx_path(Path::new("vault.kdbx")).is_ok());
        assert!(matches!(
            validate_kdbx_path(Path::new("vault.txt")),
            Err(ICloudError::NotKdbx)
        ));
    }
}

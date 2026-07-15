//! Transactional replacement of the running macOS application bundle.
//!
//! The only unsafe operation is macOS' `renamex_np(RENAME_SWAP)`: it swaps two
//! sibling directory entries atomically, so a crash cannot leave the user with
//! neither the old nor the new application. No privileged shell is involved.

#![allow(unsafe_code)]

use std::ffi::CString;
use std::fs::{self, File};
use std::io::{self, Cursor};
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Component, Path};

use flate2::read::GzDecoder;

use super::UpdateError;

const MAX_ARCHIVE_ENTRIES: usize = 100_000;
const MAX_EXTRACTED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub(super) fn install(target: &Path, bytes: Vec<u8>) -> Result<(), UpdateError> {
    let target = fs::canonicalize(target)
        .map_err(|error| install_io("resolve installed application bundle", error))?;
    let parent = target
        .parent()
        .ok_or_else(|| install_error("application bundle has no parent directory"))?;
    let bundle_name = target
        .file_name()
        .filter(|name| Path::new(name).extension().is_some_and(|ext| ext == "app"))
        .ok_or_else(|| install_error("installer target is not an .app bundle"))?;

    if !target.is_dir() {
        return Err(install_error("installed application bundle was not found"));
    }

    // Gatekeeper-translocated apps run from a read-only mount; staging next
    // to the bundle would fail every time with an unactionable "Read-only
    // file system". Name the actual remedy instead.
    if target
        .components()
        .any(|component| component.as_os_str() == "AppTranslocation")
    {
        return Err(install_error(
            "FerrisPass is running from a macOS security quarantine location. \
             Move FerrisPass.app to /Applications and start it from there, then update again.",
        ));
    }

    // A kill/crash mid-install orphans the hidden staging directory (up to
    // the full bundle size) next to the app forever. Sweep leftovers ONLY
    // when the installed bundle validates as a working app: after a failed
    // rollback the staging directory holds the only known-good old bundle,
    // and `target.is_dir()` alone cannot tell that state apart. The age
    // gate additionally protects a concurrent instance's live staging.
    if validate_bundle(&target).is_ok() {
        sweep_stale_staging(parent);
    }

    // Keeping this directory until the transaction is resolved is deliberate:
    // after a failed rollback it contains the only known-good old bundle.
    let staging_root = tempfile::Builder::new()
        .prefix(".ferrispass-update-")
        .tempdir_in(parent)
        .map_err(|error| install_io("create sibling staging directory", error))?
        .keep();
    let staged_bundle = staging_root.join(bundle_name);

    let prepared = extract_bundle(&bytes, &staging_root, bundle_name)
        .and_then(|()| validate_bundle(&staged_bundle))
        .and_then(|()| sync_tree(&staged_bundle))
        // Durable keep-marker BEFORE the first swap, like a transaction
        // journal: from the moment the swap can have moved the old bundle
        // into staging, a crash at any point must leave the directory
        // marked, or a later sweep could delete the only good app copy.
        // Failure to write it aborts the install while nothing has been
        // touched yet.
        .and_then(|()| write_keep_marker(&staging_root));
    if let Err(error) = prepared {
        let _ = fs::remove_dir_all(&staging_root);
        return Err(error);
    }

    match replace_bundle(&target, &staged_bundle) {
        Ok(()) => {
            // The old application now lives in the staging directory. Once the
            // new directory entry is durable, removing it cannot endanger the
            // installed update.
            let _ = fs::remove_dir_all(&staging_root);
            let _ = sync_directory(parent);
            Ok(())
        }
        Err(failure) => {
            if !failure.preserve_staging {
                let _ = fs::remove_dir_all(&staging_root);
            }
            Err(install_error(failure.message))
        }
    }
}

/// Write and fsync the keep-marker, then fsync the staging directory so the
/// marker's directory entry is durable before any swap runs.
fn write_keep_marker(staging_root: &Path) -> Result<(), UpdateError> {
    let path = staging_root.join(STAGING_KEEP_MARKER);
    let file = File::create(&path).map_err(|error| install_io("write staging marker", error))?;
    file.sync_all()
        .map_err(|error| install_io("sync staging marker", error))?;
    sync_directory(staging_root).map_err(|error| install_io("sync staging directory", error))?;
    Ok(())
}

/// How old a leftover staging directory must be before the sweep may touch
/// it. Far above any real install duration, so a second FerrisPass instance
/// mid-install never loses its live staging to this cleanup.
const STAGING_SWEEP_MIN_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Marker file that makes a staging directory unsweepable. Written durably
/// BEFORE the first swap, because from that point on the directory may hold
/// the only known-good old bundle — and `validate_bundle` on the target is
/// structural only, it cannot prove the new bundle actually starts. Only a
/// fully successful install removes the directory (marker included).
const STAGING_KEEP_MARKER: &str = ".ferrispass-keep";

/// Remove `.ferrispass-update-*` directories left behind by a crashed or
/// killed earlier install. Only called after the installed bundle passed
/// `validate_bundle`, so nothing in these leftovers is still needed. Best
/// effort — a failure here must never block the actual update.
fn sweep_stale_staging(parent: &Path) {
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let is_staging = name
            .to_str()
            .is_some_and(|name| name.starts_with(".ferrispass-update-"))
            && entry.file_type().is_ok_and(|kind| kind.is_dir());
        if !is_staging {
            continue;
        }
        // Fail closed: an unreadable marker state must protect the
        // directory exactly like a present marker — this may be the only
        // good copy of the old app.
        if !matches!(
            entry.path().join(STAGING_KEEP_MARKER).try_exists(),
            Ok(false)
        ) {
            continue;
        }
        let is_old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .and_then(|modified| modified.elapsed().map_err(io::Error::other))
            .is_ok_and(|age| age >= STAGING_SWEEP_MIN_AGE);
        if is_old {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

fn extract_bundle(
    bytes: &[u8],
    root: &Path,
    bundle_name: &std::ffi::OsStr,
) -> Result<(), UpdateError> {
    let decoder = GzDecoder::new(Cursor::new(bytes));
    let mut archive = tar::Archive::new(decoder);
    let mut entries = 0usize;
    let mut extracted_bytes = 0u64;

    for entry in archive
        .entries()
        .map_err(|error| install_io("read update archive", error))?
    {
        let mut entry = entry.map_err(|error| install_io("read update archive entry", error))?;
        entries = entries.saturating_add(1);
        if entries > MAX_ARCHIVE_ENTRIES {
            return Err(install_error("update archive contains too many entries"));
        }

        let kind = entry.header().entry_type();
        if kind.is_pax_global_extensions()
            || kind.is_pax_local_extensions()
            || kind.is_gnu_longname()
            || kind.is_gnu_longlink()
        {
            // Metadata records are consumed by `tar` and applied to the next
            // real entry. The resulting path is validated below when that
            // entry is visited.
            continue;
        }

        let path = entry
            .path()
            .map_err(|error| install_io("read update archive path", error))?;
        validate_archive_path(&path, bundle_name)?;

        extracted_bytes = extracted_bytes
            .checked_add(entry.size())
            .ok_or_else(|| install_error("update archive size overflow"))?;
        if extracted_bytes > MAX_EXTRACTED_BYTES {
            return Err(install_error("expanded update exceeds the 2 GiB limit"));
        }

        if !(kind.is_file() || kind.is_dir() || kind.is_symlink() || kind.is_hard_link()) {
            return Err(install_error(
                "update archive contains an unsupported entry type",
            ));
        }
        if (kind.is_symlink() || kind.is_hard_link())
            && !link_stays_in_bundle(
                &path,
                entry.link_name().ok().flatten().as_deref(),
                bundle_name,
            )
        {
            return Err(install_error("update archive contains an unsafe link"));
        }

        let unpacked = entry
            .unpack_in(root)
            .map_err(|error| install_io("extract update archive", error))?;
        if !unpacked {
            return Err(install_error(
                "update archive path escapes the staging directory",
            ));
        }
    }

    if entries == 0 {
        return Err(install_error("update archive is empty"));
    }
    Ok(())
}

fn validate_archive_path(path: &Path, bundle_name: &std::ffi::OsStr) -> Result<(), UpdateError> {
    let mut components = path.components();
    match components.next() {
        Some(Component::Normal(first)) if first == bundle_name => {}
        _ => {
            return Err(install_error(
                "update archive has an unexpected top-level directory",
            ));
        }
    }
    if components.any(|part| !matches!(part, Component::Normal(_) | Component::CurDir)) {
        return Err(install_error("update archive contains an unsafe path"));
    }
    Ok(())
}

fn link_stays_in_bundle(path: &Path, link: Option<&Path>, bundle_name: &std::ffi::OsStr) -> bool {
    let Some(link) = link else {
        return false;
    };
    if link.is_absolute() {
        return false;
    }

    // Hard-link names are archive-root relative. Symlink names are relative to
    // their containing directory. Accept either representation, but never let
    // `..` climb above the bundle's top-level component.
    let mut depth = if link
        .components()
        .next()
        .is_some_and(|part| part == Component::Normal(bundle_name))
    {
        0usize
    } else {
        path.parent()
            .map(|parent| parent.components().count())
            .unwrap_or(1)
    };

    for component in link.components() {
        match component {
            Component::Normal(_) => depth = depth.saturating_add(1),
            Component::CurDir => {}
            Component::ParentDir if depth > 1 => depth -= 1,
            _ => return false,
        }
    }
    depth >= 1
}

fn validate_bundle(bundle: &Path) -> Result<(), UpdateError> {
    if !bundle.is_dir()
        || !bundle.join("Contents/Info.plist").is_file()
        || !bundle.join("Contents/MacOS").is_dir()
    {
        return Err(install_error(
            "update archive does not contain a valid macOS app bundle",
        ));
    }

    let has_executable = fs::read_dir(bundle.join("Contents/MacOS"))
        .map_err(|error| install_io("inspect staged application", error))?
        .filter_map(Result::ok)
        .any(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()));
    if !has_executable {
        return Err(install_error("staged application contains no executable"));
    }
    Ok(())
}

fn sync_tree(path: &Path) -> Result<(), UpdateError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| install_io("inspect staged application", error))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        for entry in
            fs::read_dir(path).map_err(|error| install_io("read staged application", error))?
        {
            let entry = entry.map_err(|error| install_io("read staged application", error))?;
            sync_tree(&entry.path())?;
        }
        sync_directory(path).map_err(|error| install_io("flush staged directory", error))
    } else if metadata.is_file() {
        File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(|error| install_io("flush staged file", error))
    } else {
        Err(install_error("staged application contains a special file"))
    }
}

struct ReplaceFailure {
    message: String,
    preserve_staging: bool,
}

fn replace_bundle(target: &Path, staged: &Path) -> Result<(), ReplaceFailure> {
    match atomic_swap(target, staged) {
        Ok(()) => {
            if let Err(error) = sync_swap_parents(target, staged) {
                return match atomic_swap(target, staged) {
                    Ok(()) => {
                        let _ = sync_swap_parents(target, staged);
                        Err(ReplaceFailure {
                            message: format!(
                                "could not persist update; restored previous app: {error}"
                            ),
                            preserve_staging: false,
                        })
                    }
                    Err(rollback) => Err(ReplaceFailure {
                        message: format!(
                            "could not persist update and rollback failed ({rollback}); previous app retained at {}",
                            staged.display()
                        ),
                        preserve_staging: true,
                    }),
                };
            }
            Ok(())
        }
        // A two-rename fallback has an unavoidable crash window in which the
        // installed path does not exist. Fail closed and leave the old app in
        // place when the filesystem cannot exchange both entries atomically.
        Err(error) if swap_is_unsupported(&error) => Err(ReplaceFailure {
            message: format!("application filesystem does not support atomic updates: {error}"),
            preserve_staging: false,
        }),
        Err(error) => Err(ReplaceFailure {
            message: format!("could not atomically replace application: {error}"),
            preserve_staging: false,
        }),
    }
}

fn sync_swap_parents(first: &Path, second: &Path) -> io::Result<()> {
    let first_parent = first.parent().expect("validated first parent");
    let second_parent = second.parent().expect("validated second parent");
    sync_directory(first_parent)?;
    if second_parent != first_parent {
        sync_directory(second_parent)?;
    }
    Ok(())
}

fn atomic_swap(first: &Path, second: &Path) -> io::Result<()> {
    let first = CString::new(first.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let second = CString::new(second.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;

    // SAFETY: both pointers are valid NUL-terminated path strings for the
    // duration of the call; RENAME_SWAP does not retain them.
    let result = unsafe { libc::renamex_np(first.as_ptr(), second.as_ptr(), libc::RENAME_SWAP) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn swap_is_unsupported(error: &io::Error) -> bool {
    error
        .raw_os_error()
        .is_some_and(|code| code == libc::ENOTSUP || code == libc::EINVAL || code == libc::ENOSYS)
}

fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

fn install_error(message: impl Into<String>) -> UpdateError {
    UpdateError::Install(message.into())
}

fn install_io(action: &str, error: io::Error) -> UpdateError {
    install_error(format!("{action}: {error}"))
}

#[cfg(test)]
mod tests {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use tar::{Builder, Header};

    use super::*;

    fn append(builder: &mut Builder<GzEncoder<Vec<u8>>>, path: &str, contents: &[u8], mode: u32) {
        let mut header = Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(mode);
        header.set_cksum();
        builder.append_data(&mut header, path, contents).unwrap();
    }

    fn app_archive(bundle_name: &str, executable: &[u8]) -> Vec<u8> {
        let encoder = GzEncoder::new(Vec::new(), Compression::fast());
        let mut builder = Builder::new(encoder);
        append(
            &mut builder,
            &format!("{bundle_name}/Contents/Info.plist"),
            b"plist",
            0o644,
        );
        append(
            &mut builder,
            &format!("{bundle_name}/Contents/MacOS/ferrispass"),
            executable,
            0o755,
        );
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn old_app(path: &Path) {
        fs::create_dir_all(path.join("Contents/MacOS")).unwrap();
        fs::write(path.join("Contents/Info.plist"), "old-plist").unwrap();
        fs::write(path.join("Contents/MacOS/ferrispass"), "old").unwrap();
    }

    #[test]
    fn installs_bundle_without_deleting_the_only_good_copy() {
        let parent = tempfile::tempdir().unwrap();
        let target = parent.path().join("FerrisPass.app");
        old_app(&target);

        install(&target, app_archive("FerrisPass.app", b"new")).unwrap();

        assert_eq!(
            fs::read(target.join("Contents/MacOS/ferrispass")).unwrap(),
            b"new"
        );
        assert!(fs::read_dir(parent.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".ferrispass-update-")
        }));
    }

    #[test]
    fn invalid_archive_leaves_installed_app_untouched() {
        let parent = tempfile::tempdir().unwrap();
        let target = parent.path().join("FerrisPass.app");
        old_app(&target);

        let error = install(&target, app_archive("Other.app", b"new")).unwrap_err();

        assert!(error.to_string().contains("unexpected top-level"));
        assert_eq!(
            fs::read(target.join("Contents/MacOS/ferrispass")).unwrap(),
            b"old"
        );
    }

    #[test]
    fn unsafe_archive_paths_and_links_are_rejected() {
        let bundle = std::ffi::OsStr::new("FerrisPass.app");

        assert!(validate_archive_path(Path::new("Other.app/file"), bundle).is_err());
        assert!(validate_archive_path(Path::new("FerrisPass.app/../escape"), bundle).is_err());
        assert!(!link_stays_in_bundle(
            Path::new("FerrisPass.app/Contents/link"),
            Some(Path::new("../../outside")),
            bundle,
        ));
        assert!(link_stays_in_bundle(
            Path::new("FerrisPass.app/Contents/link"),
            Some(Path::new("MacOS/ferrispass")),
            bundle,
        ));
    }
}

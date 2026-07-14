//! Owned temp-file abstraction for launch payloads.
//!
//! The file lives in `$TMPDIR/ferrispass-launch-<uid>/` (one shared
//! subdir for the whole app). On Unix the subdir is `0700` and each
//! file is `0600` — strictly per-user, no symlink races (we use
//! `O_CREAT | O_EXCL` via `OpenOptions::create_new`).
//!
//! Cleanup has three layers, by intent:
//! 1. AppShell holds the `TempLaunchFile` in `pending_launches` and
//!    drops it after the cleanup TTL → file unlinked.
//! 2. On lock or quit, the AppShell calls `sweeper::purge_all()` →
//!    whole subdir removed.
//! 3. `sweeper::sweep_stale()` removes anything older than 60 s from
//!    a run that crashed. It runs at startup, again 120 s later (a
//!    crash + quick relaunch leaves the orphan younger than 60 s, so
//!    the startup pass alone would spare it for the whole session),
//!    and on every vault unlock.
//!
//! We deliberately do NOT use the `tempfile` crate: its `NamedTempFile`
//! is dev-only in our Cargo.toml, and its eager `Drop` semantics
//! would race with `open` (which returns immediately while the target
//! app is still parsing the file).

use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use uuid::Uuid;

/// A file we wrote into the launch tempdir. Drop = best-effort unlink.
pub struct TempLaunchFile {
    path: PathBuf,
}

impl TempLaunchFile {
    /// Create a fresh launch file under our managed launch tempdir.
    /// `extension` is appended (no dot), e.g. `"sapc"`. Returns `Err`
    /// on any I/O failure — the caller surfaces this as a toast and
    /// aborts the launch.
    pub fn create(extension: &str, contents: &[u8]) -> io::Result<Self> {
        let dir = launch_dir()?;
        Self::create_in(&dir, extension, contents)
    }

    /// Same as `create`, but writes into a caller-provided directory.
    /// Used by the test suite to keep parallel tests from racing on a
    /// shared real tempdir; production always goes through `create`.
    pub(crate) fn create_in(dir: &Path, extension: &str, contents: &[u8]) -> io::Result<Self> {
        let name = format!("launch-{}.{}", Uuid::new_v4(), extension);
        let path = dir.join(name);

        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600);
        }
        let mut f = opts.open(&path)?;
        f.write_all(contents)?;
        f.sync_all()?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempLaunchFile {
    fn drop(&mut self) {
        // Best-effort. The scheduled-cleanup task is the primary path;
        // this is the safety net for early-drop (lock / quit / error
        // recovery). Errors are intentionally swallowed — there's
        // nothing the user can do about a failing unlink at this
        // point, and logging the path or content here would defeat
        // the whole "no body in logs" rule.
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Resolve (and lazily create) the per-user launch tempdir. All launch
/// payloads land here, and `sweeper::purge_all()` wipes the whole
/// thing on lock/quit. Idempotent — safe to call from `AppShell::new`
/// before we know if we'll ever launch anything.
pub fn launch_dir() -> io::Result<PathBuf> {
    let mut path = launch_base_dir();
    path.push(format!("ferrispass-launch-{}", instance_tag()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        // Create atomically with 0700 — no chmod-after-create window in
        // which the directory is briefly listable by other users.
        match std::fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        verify_launch_dir(&path)?;
    }
    #[cfg(not(unix))]
    if !path.exists() {
        std::fs::create_dir_all(&path)?;
    }
    Ok(path)
}

/// A pre-existing launch dir cannot be trusted: it lives under a sometimes
/// world-writable parent (the `/tmp` fallback) with a predictable name, so
/// another local user may have planted a symlink or pre-created it — as the
/// directory's owner they could swap or unlink payloads between our write
/// and the target app's open, regardless of the payload files' 0600 mode.
/// Require a real directory owned by this uid; tighten loose permissions.
#[cfg(unix)]
fn verify_launch_dir(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "launch path exists but is not a real directory",
        ));
    }
    if meta.uid() != process_uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "launch directory is owned by another user",
        ));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        // Ours but loosened (or created by an older build): tighten
        // instead of failing.
        let mut perms = meta.permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(path, perms)?;
    }
    Ok(())
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn process_uid() -> u32 {
    // SAFETY: getuid() is async-signal-safe, takes no arguments, and
    // cannot fail on POSIX.
    unsafe { libc::getuid() }
}

/// Base directory for cleartext launch payloads. `std::env::temp_dir()`
/// is normally the right macOS per-user temp root (`/var/folders/.../T`),
/// but it is environment-driven. If a user or launcher accidentally points
/// `TMPDIR` at a synced folder, keep secrets out of cloud storage and fall
/// back to the local system temp root.
fn launch_base_dir() -> PathBuf {
    let candidate = std::env::temp_dir();
    if is_cloud_storage_path(&candidate) {
        fallback_temp_dir()
    } else {
        candidate
    }
}

#[cfg(unix)]
fn fallback_temp_dir() -> PathBuf {
    PathBuf::from("/tmp")
}

#[cfg(not(unix))]
fn fallback_temp_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Temp"))
}

fn is_cloud_storage_path(path: &Path) -> bool {
    path.components().any(|component| {
        let name = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        name == "cloudstorage"
            || name.starts_with("onedrive")
            || name.contains("onedrive -")
            || name == "icloud drive"
            || name == "dropbox"
            || name == "google drive"
            || name == "box sync"
    })
}

/// Stable per-user tag mixed into the tempdir name. Username + uid
/// gives each macOS account on a shared box its own subdir; without
/// this, two users running FerrisPass would race for the same path
/// (and the `0700` would lock the second one out).
fn instance_tag() -> String {
    #[cfg(unix)]
    {
        // Safe: getuid() is signal-safe and infallible on POSIX.
        // We can't avoid `unsafe` for getuid itself, so we wrap it
        // in a helper module further below to keep the unsafe block
        // contained to a single line. forbid(unsafe_code) at the
        // crate root means we use `users`-style fallback instead:
        // read $USER from env, hash with the process's start time.
        let user = std::env::var("USER").unwrap_or_else(|_| "anon".to_string());
        // On macOS $TMPDIR is already per-user (`/var/folders/.../T/`),
        // so $USER alone is sufficient as a uniqueness tag.
        sanitize(&user)
    }
    #[cfg(not(unix))]
    {
        let user = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "anon".to_string());
        sanitize(&user)
    }
}

/// Strip anything that would be questionable in a directory name.
/// Conservative — alphanumeric only, lowercased.
fn sanitize(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    if cleaned.is_empty() {
        "anon".into()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `create_in` writes the bytes verbatim, then `Drop` unlinks the
    /// file. Together those two halves are the entire contract the
    /// launchers depend on. Uses an isolated tempdir per test to keep
    /// parallel runs from racing on the production launch_dir.
    #[test]
    fn create_then_drop_unlinks() {
        let dir = ::tempfile::TempDir::new().expect("tempdir");
        let body = b"conn=test&pass=hunter2";
        let path = {
            let f = TempLaunchFile::create_in(dir.path(), "sapc", body).expect("create");
            let path = f.path().to_path_buf();
            assert!(path.exists(), "file must exist while handle alive");
            assert_eq!(std::fs::read(&path).expect("read"), body);
            path
        };
        assert!(
            !path.exists(),
            "file must be unlinked once the handle is dropped"
        );
    }

    /// Two creates in the same directory don't collide — UUID-suffixed
    /// names are unique. This is also what defends against multiple
    /// rapid launches stomping each other's files.
    #[test]
    fn two_creates_do_not_collide() {
        let dir = ::tempfile::TempDir::new().expect("tempdir");
        let a = TempLaunchFile::create_in(dir.path(), "sapc", b"a").expect("create a");
        let b = TempLaunchFile::create_in(dir.path(), "sapc", b"b").expect("create b");
        assert_ne!(a.path(), b.path());
        assert!(a.path().exists() && b.path().exists());
    }

    #[cfg(unix)]
    #[test]
    fn file_permissions_are_user_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = ::tempfile::TempDir::new().expect("tempdir");
        let f = TempLaunchFile::create_in(dir.path(), "sapc", b"x").expect("create");
        let mode = std::fs::metadata(f.path())
            .expect("stat")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "launch payload must be user-only");
    }

    #[test]
    fn sanitize_drops_specials() {
        assert_eq!(sanitize("user.name"), "username");
        assert_eq!(sanitize("Alice/Bob"), "alicebob");
        assert_eq!(sanitize(""), "anon");
        assert_eq!(sanitize("---"), "anon");
    }

    #[cfg(unix)]
    #[test]
    fn launch_dir_verification_rejects_symlinks_and_files() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = tempfile::TempDir::new().expect("tempdir");

        let file_path = tmp.path().join("a-file");
        std::fs::write(&file_path, b"x").expect("write file");
        assert!(verify_launch_dir(&file_path).is_err());

        let link_path = tmp.path().join("a-link");
        std::os::unix::fs::symlink(tmp.path(), &link_path).expect("symlink");
        assert!(verify_launch_dir(&link_path).is_err());

        // A directory we own with loose permissions is tightened, not rejected.
        let dir_path = tmp.path().join("loose-dir");
        std::fs::create_dir(&dir_path).expect("mkdir");
        std::fs::set_permissions(&dir_path, std::fs::Permissions::from_mode(0o755))
            .expect("loosen");
        verify_launch_dir(&dir_path).expect("owned dir passes");
        let mode = std::fs::metadata(&dir_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
    }

    #[test]
    fn cloud_storage_paths_are_not_valid_launch_roots() {
        assert!(is_cloud_storage_path(Path::new(
            "/Users/alice/Library/CloudStorage/OneDrive-Contoso/T"
        )));
        assert!(is_cloud_storage_path(Path::new(
            "/Users/alice/OneDrive - Contoso/Documents"
        )));
    }

    #[test]
    fn ordinary_temp_paths_are_valid_launch_roots() {
        assert!(!is_cloud_storage_path(Path::new("/var/folders/ab/cdef/T")));
        assert!(!is_cloud_storage_path(Path::new("/tmp")));
    }
}

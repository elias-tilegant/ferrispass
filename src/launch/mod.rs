//! Pluggable "open this entry in the right native app" layer.
//!
//! The flow is uniform across launchers:
//! 1. UI checks `primary_launcher_for(entry)` to decide whether to
//!    show the Launch button at all.
//! 2. On click, the AppShell hands the launcher an entry + password +
//!    custom-field slice via `LaunchContext`.
//! 3. The launcher writes a small file with the connection details
//!    into our managed temp directory, then asks the OS to open it
//!    with the registered handler. The returned `LaunchHandle` owns
//!    the temp file's path and unlinks it on drop.
//! 4. The AppShell parks the handle in `pending_launches` and
//!    schedules a delayed cleanup task (TTL from `AppSettings`). On
//!    Lock or Quit, the handle is dropped immediately and the whole
//!    launch tempdir is purged.
//!
//! v0.3.0 ships with one backend: `SapGuiMacLauncher`. The trait is
//! intentionally platform-neutral so the same `LaunchContext` works
//! for future backends (Windows `sapshcut.exe` / `.sap` shortcut,
//! Linux `sapgui` binary, future RDP / SSH launchers, …).

use crate::domain::{CustomField, VaultEntry};

pub mod sap;
pub mod sweeper;
pub mod tempfile;

pub use tempfile::{TempLaunchFile, launch_dir};

/// Backend that knows how to open one entry in one external app.
pub trait Launcher: Send + Sync {
    /// Stable id, e.g. `"sap-gui"`. Routing key — never user-visible.
    fn id(&self) -> &'static str;
    /// User-facing button label, e.g. `"Open in SAP GUI"`.
    fn label(&self) -> &'static str;
    /// Cheap detection from the snapshot. Must not touch the password
    /// or do I/O — that's reserved for `launch`.
    fn supports(&self, entry: &VaultEntry) -> bool;
    /// Compose the launch payload, hand it to the OS. Caller is
    /// responsible for keeping the returned handle alive long enough
    /// for the target app to read the temp file (see `pending_launches`
    /// in AppShell). On error, no temp file is left behind.
    fn launch(&self, ctx: LaunchContext<'_>) -> Result<LaunchHandle, LaunchError>;
}

/// Everything a launcher needs to compose its payload, borrowed from
/// AppShell-owned state. Lifetime tied to the launch call — the
/// launcher must not stash references past `launch`'s return.
pub struct LaunchContext<'a> {
    pub entry: &'a VaultEntry,
    /// `None` only when the entry has no password set. Cleartext —
    /// already exposed in the same trust zone as the snapshot.
    pub password: Option<&'a str>,
    /// Convenience pointer to `entry.custom_fields`. Same allocation,
    /// just saves the launcher a `&entry.custom_fields[..]`.
    pub custom_fields: &'a [CustomField],
}

/// Handle returned from a successful `launch`. Owning the handle
/// owns the temp file's lifetime: drop = unlink. AppShell parks
/// these in a `Vec` and pops the oldest after the cleanup TTL.
pub struct LaunchHandle {
    pub temp_file: Option<TempLaunchFile>,
    pub launcher_id: &'static str,
}

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    /// Entry is missing a field the backend requires (e.g. no
    /// `SAP_CONN` for SAP). Caller renders this as a toast.
    #[error("missing required field: {0}")]
    MissingField(&'static str),

    /// Entry has no password set, but the backend needs one.
    #[error("entry has no password")]
    NoPassword,

    /// I/O during temp-file write or process spawn. Display value is
    /// safe (no body) — only the kind + path; never log the file
    /// contents themselves.
    #[error("launch i/o failed: {0}")]
    Io(#[from] std::io::Error),
}

/// All launchers that match this entry, in registry order. Used by
/// the future "open with…" submenu when more than one applies. v0.3
/// always returns 0 or 1.
pub fn launchers_for(entry: &VaultEntry) -> Vec<&'static dyn Launcher> {
    REGISTRY
        .iter()
        .copied()
        .filter(|l| l.supports(entry))
        .collect()
}

/// First applicable launcher, or `None` when no backend supports the
/// entry. Drives the conditional Launch button in the detail panel.
pub fn primary_launcher_for(entry: &VaultEntry) -> Option<&'static dyn Launcher> {
    launchers_for(entry).into_iter().next()
}

/// Static registry — order = priority. Backends are gated by
/// `cfg(target_os)` so a Linux build doesn't carry the macOS-only
/// `open` path or vice versa.
static REGISTRY: &[&dyn Launcher] = &[
    #[cfg(target_os = "macos")]
    &sap::SAP_GUI_MAC,
];

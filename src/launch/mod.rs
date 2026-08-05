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

/// Closed set of launch protocols supported by FerrisPass. Keeping routing
/// typed avoids making CLI/API callers depend on backend registry strings and
/// gives future SSH/RDP launchers an explicit, exhaustively matched home.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchTarget {
    Sap,
}

impl LaunchTarget {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Sap => "sap",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Sap => "Open in SAP GUI",
        }
    }
}

/// Backend that knows how to open one entry in one external app.
pub(crate) trait Launcher: Send + Sync {
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

    /// The requested protocol has no backend on this operating system.
    #[error("{0} launch is unsupported on this platform")]
    UnsupportedTarget(&'static str),

    /// I/O during temp-file write or process spawn. Display value is
    /// safe (no body) — only the kind + path; never log the file
    /// contents themselves.
    #[error("launch i/o failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Launch an entry through an explicitly selected protocol. Both GUI and CLI
/// ultimately use the same backend implementation; only target selection and
/// handle lifetime differ between those frontends.
pub fn launch(target: LaunchTarget, ctx: LaunchContext<'_>) -> Result<LaunchHandle, LaunchError> {
    match target {
        LaunchTarget::Sap => {
            #[cfg(target_os = "macos")]
            {
                sap::SAP_GUI_MAC.launch(ctx)
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = ctx;
                Err(LaunchError::UnsupportedTarget("SAP"))
            }
        }
    }
}

/// Infer the typed protocol from entry metadata. This is deliberately based
/// on backend validation rather than titles or other fuzzy user content.
pub fn target_for_entry(entry: &VaultEntry) -> Option<LaunchTarget> {
    #[cfg(target_os = "macos")]
    if sap::SAP_GUI_MAC.supports(entry) {
        return Some(LaunchTarget::Sap);
    }
    let _ = entry;
    None
}

/// Primary typed target for UI auto-detection. Kept as a named helper because
/// a future multi-target entry may add an "open with…" chooser while retaining
/// one deterministic default.
pub fn primary_launcher_for(entry: &VaultEntry) -> Option<LaunchTarget> {
    target_for_entry(entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{domain::CustomField, launch::sap};

    #[test]
    fn target_ids_are_stable_cli_routing_values() {
        assert_eq!(LaunchTarget::Sap.id(), "sap");
        assert_eq!(LaunchTarget::Sap.label(), "Open in SAP GUI");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn sap_target_is_inferred_only_from_complete_metadata() {
        let mut entry = VaultEntry::default();
        assert_eq!(target_for_entry(&entry), None);
        entry.custom_fields.push(CustomField {
            key: sap::KEY_HOST.into(),
            value: "sap.example.com".into(),
            protected: false,
        });
        assert_eq!(target_for_entry(&entry), None);
        entry.custom_fields.push(CustomField {
            key: sap::KEY_INSTANCE.into(),
            value: "00".into(),
            protected: false,
        });
        assert_eq!(target_for_entry(&entry), Some(LaunchTarget::Sap));
    }
}

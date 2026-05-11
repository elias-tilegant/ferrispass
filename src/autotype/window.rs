//! Read the OS's foreground window.
//!
//! Wraps `active_win_pos_rs::get_active_window` to a smaller shape
//! that the rest of the auto-type pipeline cares about (title +
//! app name + process path). Returns `None` rather than `Result`
//! because there's nothing the caller can do about the error case —
//! "we couldn't tell what window is focused" is operationally the
//! same as "no foreground", and the only sensible response is to
//! abort the auto-type with a notification.

use std::path::PathBuf;

/// Distilled foreground-window descriptor. `app_name` is the
/// human-readable application name on macOS (e.g. `Safari`,
/// `Firefox`, `Chromium`) — derived by `active-win-pos-rs` from the
/// `LocalizedName` of the frontmost app. `window_title` is the
/// frontmost window's title for that app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForegroundInfo {
    pub app_name: String,
    pub window_title: String,
    pub process_path: PathBuf,
}

impl ForegroundInfo {
    /// `true` when the foreground belongs to the FerrisPass app itself.
    /// We use this as a guard so the global hotkey doesn't try to
    /// auto-type into our own vault list (which would also be a
    /// security footgun — typing the user's password back into the
    /// password input).
    ///
    /// Matched on `app_name` rather than process path, because the
    /// dev build (`cargo run`) and the bundled `.app` produce
    /// different process paths but the same display name.
    pub fn is_self(&self) -> bool {
        self.app_name.eq_ignore_ascii_case("ferrispass")
    }
}

/// Read the current foreground window, or `None` if the OS query
/// failed. The crate panics under unusual conditions on some
/// platforms — we don't reach for `catch_unwind` here because
/// `active-win-pos-rs`'s macOS path uses CFRetained references that
/// won't unwind safely. Instead we rely on the crate's `Result` for
/// the documented failure modes.
pub fn foreground() -> Option<ForegroundInfo> {
    let win = active_win_pos_rs::get_active_window().ok()?;
    Some(ForegroundInfo {
        app_name: win.app_name,
        window_title: win.title,
        process_path: win.process_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_self_is_case_insensitive() {
        // The bundled .app reports "FerrisPass", but the dev build
        // running via `cargo run` reports "ferrispass" — both must
        // resolve to the same self-detection branch or we'd accept a
        // self-target on dev builds.
        let info = ForegroundInfo {
            app_name: "ferrispass".into(),
            window_title: String::new(),
            process_path: PathBuf::new(),
        };
        assert!(info.is_self());
        let info = ForegroundInfo {
            app_name: "FerrisPass".into(),
            window_title: String::new(),
            process_path: PathBuf::new(),
        };
        assert!(info.is_self());
    }

    #[test]
    fn is_self_rejects_other_apps() {
        let info = ForegroundInfo {
            app_name: "Safari".into(),
            window_title: "Sign in".into(),
            process_path: PathBuf::new(),
        };
        assert!(!info.is_self());
    }
}

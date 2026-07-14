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

    /// Whether the foreground process is a known web browser. A page title is
    /// attacker-controlled, so the automatic matcher must not use it as a
    /// credential-selection signal. This list is intentionally conservative:
    /// a false positive only disables automatic selection, while a false
    /// negative still cannot match by title or hostname substring.
    pub fn is_browser(&self) -> bool {
        if is_known_browser_name(&self.app_name) {
            return true;
        }

        self.process_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(is_known_browser_name)
    }

    /// `true` when `other` plausibly refers to the same application as
    /// `self`. The typer's focus guard uses this to confirm focus hasn't
    /// moved to a *different app* between the hotkey press and keystroke
    /// dispatch (or across a `{DELAY}` pause). Window titles legitimately
    /// change mid-sequence — multi-step logins navigate from a username
    /// page to a password page — so the title is deliberately excluded;
    /// the process path is the strongest stable signal we have. Falls
    /// back to the app name when either side lacks a path (some AX
    /// queries yield an empty one).
    pub fn same_app(&self, other: &ForegroundInfo) -> bool {
        if !self.process_path.as_os_str().is_empty() && !other.process_path.as_os_str().is_empty() {
            return self.process_path == other.process_path;
        }
        self.app_name.eq_ignore_ascii_case(&other.app_name)
    }
}

fn is_known_browser_name(raw: &str) -> bool {
    let name = raw.trim().to_ascii_lowercase();
    matches!(
        name.as_str(),
        "safari"
            | "safari technology preview"
            | "google chrome"
            | "google chrome canary"
            | "google chrome beta"
            | "google chrome dev"
            | "chromium"
            | "firefox"
            | "firefox developer edition"
            | "firefox nightly"
            | "arc"
            | "arc browser"
            | "brave browser"
            | "microsoft edge"
            | "microsoft edge beta"
            | "microsoft edge dev"
            | "microsoft edge canary"
            | "opera"
            | "opera gx"
            | "vivaldi"
            | "orion"
            | "duckduckgo"
            | "duckduckgo browser"
            | "zen"
            | "zen browser"
            | "dia"
    )
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

    fn fg(app: &str, title: &str, path: &str) -> ForegroundInfo {
        ForegroundInfo {
            app_name: app.into(),
            window_title: title.into(),
            process_path: PathBuf::from(path),
        }
    }

    #[test]
    fn identifies_common_browsers_by_app_or_executable_name() {
        for info in [
            fg("Safari", "", ""),
            fg("Google Chrome", "", ""),
            fg("Firefox Developer Edition", "", ""),
            fg("Arc", "", ""),
            fg(
                "Unknown localized name",
                "",
                "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            ),
        ] {
            assert!(info.is_browser(), "expected browser: {info:?}");
        }
    }

    #[test]
    fn domain_named_native_app_is_not_assumed_to_be_a_browser() {
        let info = fg("github.com", "Sign in to GitHub", "/Applications/GitHub");
        assert!(!info.is_browser());
    }

    #[test]
    fn same_app_compares_process_path_when_both_present() {
        let a = fg(
            "Safari",
            "Sign in",
            "/Applications/Safari.app/Contents/MacOS/Safari",
        );
        let b = fg(
            "Safari",
            "Password",
            "/Applications/Safari.app/Contents/MacOS/Safari",
        );
        // Title changed (multi-step login) but same process → still same app.
        assert!(a.same_app(&b));

        let c = fg(
            "Safari",
            "Sign in",
            "/Applications/Slack.app/Contents/MacOS/Slack",
        );
        // Spoofed/equal app name but different process → different app.
        assert!(!a.same_app(&c));
    }

    #[test]
    fn same_app_falls_back_to_app_name_when_path_missing() {
        let a = fg("Safari", "Sign in", "");
        let b = fg(
            "safari",
            "Password",
            "/Applications/Safari.app/Contents/MacOS/Safari",
        );
        assert!(a.same_app(&b));
        let c = fg("Slack", "general", "");
        assert!(!a.same_app(&c));
    }
}

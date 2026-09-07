//! App-wide preferences (auto-lock / clipboard-clear timeouts) persisted
//! at `~/Library/Application Support/ferrispass/settings.json`.
//!
//! Only stores plain numbers - no secrets - so JSON is fine. Same atomic
//! write pattern as `sync/config.rs` and `app/recents.rs` (temp file +
//! fsync + rename).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const FILE_NAME: &str = "settings.json";

/// `None` on a timeout field means "disabled" - i.e. never auto-lock /
/// never auto-clear. We keep the type explicit (rather than a magic 0)
/// so the UI can distinguish "user picked Never" from "the file is
/// missing this field".
// Not `Eq`: the window geometry is floating point. Equality is only used to
// skip redundant writes, and `PartialEq` is enough for that.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppSettings {
    pub auto_lock_secs: Option<u64>,
    pub clipboard_clear_secs: Option<u64>,
    /// When `true`, FerrisPass quietly checks GitHub Releases on app start
    /// (rate-limited to ~1×/24h) and surfaces a banner if a newer build
    /// is available. Off-by-default would be more privacy-conservative,
    /// but the security upside of fast patch propagation in a password
    /// manager is significant - net better default is on.
    ///
    /// `#[serde(default = "default_true")]` so settings.json files written
    /// by older builds (which lack this field) deserialize cleanly with
    /// the right default rather than silently flipping to off.
    #[serde(default = "default_true")]
    pub auto_update_check_enabled: bool,
    /// How long the launch tempfile (e.g. `.sapc` for SAP GUI) lives
    /// before the cleanup task unlinks it. SAP GUI parses the file in
    /// well under 2 s in practice, but slow VPNs or first-launch
    /// keychain prompts can stretch it; 30 s is a comfortable default.
    /// Clamped on read to 10..=60 so a corrupt or hand-edited
    /// settings file can't disable cleanup or set an absurd window.
    #[serde(default = "default_launch_cleanup_secs")]
    pub launch_cleanup_secs: u32,
    /// Master switch for KeePass-style auto-type. Off by default
    /// because the feature pops a system permission prompt on first
    /// use - surfacing that to users who didn't ask for it would be
    /// surprising. `#[serde(default)]` so pre-feature settings.json
    /// files deserialize cleanly (= `false`, matching the cold-start
    /// behaviour).
    #[serde(default)]
    pub auto_type_enabled: bool,
    /// User-tunable global hotkey combo, in `global-hotkey` parse
    /// format (e.g. `ctrl+alt+super+KeyV`). The default matches
    /// KeePassXC's macOS default. Validated at registration time -
    /// a bad combo leaves the feature off with a Settings-tab error.
    #[serde(default = "default_auto_type_hotkey")]
    pub auto_type_hotkey: String,
    /// Auto-type sequence template (KeePass placeholder grammar). The
    /// default mirrors `{USERNAME}{TAB}{PASSWORD}{ENTER}` - the
    /// canonical login-form sequence used by ~every browser-form on
    /// the web. Per-entry overrides are not in v1; this is the global
    /// template.
    #[serde(default = "default_auto_type_sequence")]
    pub auto_type_sequence: String,
    /// When `true`, the Touch ID unlock prompt also accepts the
    /// user's macOS account password as a fallback (LAPolicy
    /// `DeviceOwnerAuthentication`). Lets the user unlock the vault
    /// in clamshell mode - the built-in Touch ID sensor is
    /// unreachable when the MacBook lid is closed, and many users
    /// have no Apple Watch fallback configured.
    ///
    /// **Security tradeoff:** with this on, anyone who knows the
    /// user's macOS login password can unlock the vault even
    /// without biometry. That's the same trust boundary 1Password
    /// and Bitwarden offer as an opt-in for Mac users; the
    /// alternative (strict biometry-only) blocks every clamshell
    /// unlock attempt and forces the master vault password.
    ///
    /// Default `true` - the product call here is "convenience over
    /// strict isolation": the user has already proven themselves
    /// to macOS, and the threat of "someone with my Mac password
    /// but not my fingerprint" is small versus the daily friction
    /// in clamshell mode. Users who want the stricter posture
    /// can turn it off in Settings → General.
    ///
    /// `#[serde(default = "default_true")]` so settings.json
    /// written by pre-Touch-ID builds deserialise cleanly with
    /// the documented default rather than silently flipping off.
    #[serde(default = "default_true")]
    pub biometric_allow_passcode_fallback: bool,
    /// How often FerrisPass checks the remote in the background and pulls
    /// in changes from other devices, in seconds. `None` means "Never"
    /// (auto-sync off).
    ///
    /// This doubles as the OAuth *keep-alive*: every tick refreshes the
    /// access token when it's near expiry, and that refresh resets the
    /// refresh token's sliding-inactivity window. Without it, a synced
    /// vault left open but untouched for longer than the tenant's
    /// inactivity window silently loses its refresh token and forces a
    /// full reconnect - the exact pain this setting exists to prevent.
    ///
    /// `#[serde(default)]` so settings.json written by pre-auto-sync
    /// builds deserialises cleanly with the documented default (on,
    /// 15 min) rather than silently disabling the feature. Clamped to a
    /// 60 s floor on read (`auto_sync_secs_clamped`) so a corrupt or
    /// hand-edited file can't hammer Graph every second.
    #[serde(default = "default_auto_sync_secs")]
    pub auto_sync_secs: Option<u64>,
    /// Light, dark, or whatever macOS is set to. Cmd+Shift+D rotated the
    /// live theme and forgot it on quit, and "follow the system" only ever
    /// meant "read it once at startup".
    #[serde(default)]
    pub theme: ThemeChoice,
    /// Last window geometry, restored on the next launch. Every start used to
    /// centre a fixed 1120x760 window, which is a poor fit for anyone with a
    /// large display or a tiling habit.
    #[serde(default)]
    pub window: Option<WindowBoundsSetting>,
}

/// Which appearance the app uses.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThemeChoice {
    /// Follow the macOS appearance, including a change while running.
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    /// Cycle order for the Cmd+Shift+D shortcut.
    pub fn next(self) -> Self {
        match self {
            ThemeChoice::System => ThemeChoice::Light,
            ThemeChoice::Light => ThemeChoice::Dark,
            ThemeChoice::Dark => ThemeChoice::System,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "System",
            ThemeChoice::Light => "Light",
            ThemeChoice::Dark => "Dark",
        }
    }
}

/// Window geometry in logical pixels. Stored flat so a hand-edited or
/// truncated settings file degrades to "no saved geometry" rather than
/// failing the whole load.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct WindowBoundsSetting {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl WindowBoundsSetting {
    /// Reject a size below the app's own minimum, and the non-finite values a
    /// hand-edited settings file can carry. Says nothing about position: see
    /// [`Self::is_reachable_on`].
    pub fn is_usable(&self) -> bool {
        self.width >= MIN_WINDOW_WIDTH
            && self.height >= MIN_WINDOW_HEIGHT
            && self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
    }

    /// True when enough of the window would land on one of `displays` for the
    /// user to grab its title bar. The saved position outlives the display it
    /// was saved on: undock a laptop, and the window reopens at coordinates
    /// that are now off every screen, with no way to drag it back.
    ///
    /// `displays` are screen rectangles in the same logical pixel space, in
    /// the same field order. An empty list means the platform could not tell
    /// us, and is accepted rather than silently discarding the user's layout.
    pub fn is_reachable_on(&self, displays: &[Self]) -> bool {
        displays.is_empty()
            || displays.iter().any(|display| {
                let horizontal =
                    (self.x + self.width).min(display.x + display.width) - self.x.max(display.x);
                let vertical =
                    (self.y + self.height).min(display.y + display.height) - self.y.max(display.y);
                horizontal >= MIN_VISIBLE_EDGE && vertical >= MIN_VISIBLE_EDGE
            })
    }
}

pub const MIN_WINDOW_WIDTH: f32 = 860.0;
pub const MIN_WINDOW_HEIGHT: f32 = 560.0;
/// How much of a restored window must be on screen, in logical pixels. Sized
/// to leave a grabbable piece of title bar, not a whole window.
const MIN_VISIBLE_EDGE: f32 = 80.0;

/// Store the window geometry, unless it is already what is on disk.
///
/// Written once, on the way out, rather than on every resize event: a drag
/// fires continuously and this is a preference nobody reads until the next
/// launch.
pub fn persist_window_bounds(bounds: WindowBoundsSetting) {
    if !bounds.is_usable() {
        return;
    }
    let mut settings = load();
    if settings.window == Some(bounds) {
        return;
    }
    settings.window = Some(bounds);
    let _ = save(&settings);
}

fn default_true() -> bool {
    true
}

fn default_launch_cleanup_secs() -> u32 {
    DEFAULT_LAUNCH_CLEANUP_SECS
}

fn default_auto_sync_secs() -> Option<u64> {
    Some(DEFAULT_AUTO_SYNC_SECS)
}

fn default_auto_type_hotkey() -> String {
    crate::autotype::hotkey::DEFAULT_HOTKEY.to_string()
}

fn default_auto_type_sequence() -> String {
    crate::autotype::sequence::DEFAULT_SEQUENCE.to_string()
}

pub const DEFAULT_LAUNCH_CLEANUP_SECS: u32 = 30;
pub const LAUNCH_CLEANUP_SECS_RANGE: std::ops::RangeInclusive<u32> = 10..=60;

/// Default background auto-sync cadence: 15 minutes. Frequent enough to
/// keep the OAuth refresh token's inactivity window alive and devices
/// reasonably in step, infrequent enough not to spam Graph or churn the
/// battery on an idle laptop.
pub const DEFAULT_AUTO_SYNC_SECS: u64 = 900;
/// Lower bound applied on read. A timer faster than this would be
/// pointless (a sync round-trip alone is often >1 s) and abusive toward
/// Graph throttling.
pub const AUTO_SYNC_SECS_FLOOR: u64 = 60;

impl AppSettings {
    /// Read the launch-cleanup TTL with the documented clamp applied.
    /// Centralised so every consumer gets the same safety net rather
    /// than each one re-implementing the bounds check.
    pub fn launch_cleanup_secs_clamped(&self) -> u32 {
        self.launch_cleanup_secs.clamp(
            *LAUNCH_CLEANUP_SECS_RANGE.start(),
            *LAUNCH_CLEANUP_SECS_RANGE.end(),
        )
    }

    /// Auto-sync interval with the 60 s floor applied. `None` is passed
    /// through unchanged - it means the feature is off, not "every 0 s".
    /// Single choke-point so the timer task and any UI both agree on the
    /// effective cadence even if the on-disk value was hand-edited below
    /// the floor.
    pub fn auto_sync_secs_clamped(&self) -> Option<u64> {
        self.auto_sync_secs.map(|s| s.max(AUTO_SYNC_SECS_FLOOR))
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        // Mirrors the previous hardcoded constants so users upgrading
        // from a build without a settings file see no behavior change.
        Self {
            auto_lock_secs: Some(240),
            clipboard_clear_secs: Some(10),
            auto_update_check_enabled: true,
            launch_cleanup_secs: DEFAULT_LAUNCH_CLEANUP_SECS,
            auto_type_enabled: false,
            auto_type_hotkey: default_auto_type_hotkey(),
            auto_type_sequence: default_auto_type_sequence(),
            biometric_allow_passcode_fallback: true,
            auto_sync_secs: default_auto_sync_secs(),
            theme: ThemeChoice::System,
            window: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("could not locate app-support directory: {0}")]
    NoSupportDir(String),

    #[error("io error on {0}: {1}")]
    Io(PathBuf, #[source] io::Error),

    #[error("could not serialise settings: {0}")]
    Serialize(#[source] serde_json::Error),
}

/// Read settings from disk. Falls back to `AppSettings::default()` on:
/// missing file (cold first run), parse failure (corrupt file - better
/// to recover than to brick the app on start), or path resolution
/// failure. Real I/O errors still propagate so genuinely broken disks
/// surface.
pub fn load() -> AppSettings {
    let dir = match crate::sync::config::app_support_dir() {
        Ok(d) => d,
        Err(_) => return AppSettings::default(),
    };
    load_in(&dir).unwrap_or_default()
}

pub fn save(settings: &AppSettings) -> Result<(), SettingsError> {
    let dir = match crate::sync::config::app_support_dir() {
        Ok(d) => d,
        Err(e) => return Err(SettingsError::NoSupportDir(e.to_string())),
    };
    save_in(&dir, settings)
}

pub(crate) fn load_in(dir: &Path) -> Result<AppSettings, SettingsError> {
    let path = dir.join(FILE_NAME);
    match fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<AppSettings>(&text) {
            Ok(s) => Ok(s),
            // Corrupt file: don't block startup; treat as defaults.
            Err(_) => Ok(AppSettings::default()),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(AppSettings::default()),
        Err(e) => Err(SettingsError::Io(path, e)),
    }
}

pub(crate) fn save_in(dir: &Path, settings: &AppSettings) -> Result<(), SettingsError> {
    fs::create_dir_all(dir).map_err(|e| SettingsError::Io(dir.to_path_buf(), e))?;
    let target = dir.join(FILE_NAME);
    let tmp = {
        let mut buf = target.as_os_str().to_owned();
        buf.push(".tmp");
        PathBuf::from(buf)
    };

    let text = serde_json::to_string_pretty(settings).map_err(SettingsError::Serialize)?;

    {
        let mut file = fs::File::create(&tmp).map_err(|e| SettingsError::Io(tmp.clone(), e))?;
        use std::io::Write as _;
        file.write_all(text.as_bytes())
            .map_err(|e| SettingsError::Io(tmp.clone(), e))?;
        file.sync_all()
            .map_err(|e| SettingsError::Io(tmp.clone(), e))?;
    }
    fs::rename(&tmp, &target).map_err(|e| SettingsError::Io(target, e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn save_then_load_roundtrips() {
        let dir = TempDir::new().unwrap();
        let s = AppSettings {
            auto_lock_secs: Some(60),
            clipboard_clear_secs: None,
            launch_cleanup_secs: 45,
            ..AppSettings::default()
        };
        save_in(dir.path(), &s).unwrap();
        let loaded = load_in(dir.path()).unwrap();
        assert_eq!(loaded, s);
    }

    /// Old settings files (written before launch_cleanup_secs existed)
    /// must deserialize cleanly with the documented default applied -
    /// otherwise upgrading the app would brick the settings file.
    #[test]
    fn missing_launch_cleanup_uses_default() {
        let dir = TempDir::new().unwrap();
        // Write a v0.2.x-shaped settings file (no launch_cleanup_secs).
        fs::write(
            dir.path().join(FILE_NAME),
            r#"{"auto_lock_secs":60,"clipboard_clear_secs":null,"auto_update_check_enabled":true}"#,
        )
        .unwrap();
        let loaded = load_in(dir.path()).unwrap();
        assert_eq!(loaded.launch_cleanup_secs, DEFAULT_LAUNCH_CLEANUP_SECS);
    }

    /// Hand-edited or corrupt TTL values can't be allowed to disable
    /// cleanup (0) or invent a 24-hour window (huge value). The
    /// clamp() is the single defensive choke-point everyone reads
    /// through.
    fn with_launch_cleanup(secs: u32) -> AppSettings {
        AppSettings {
            launch_cleanup_secs: secs,
            ..AppSettings::default()
        }
    }

    /// Settings files written before these fields existed have to keep
    /// loading, with the documented defaults rather than a failed parse.
    #[test]
    fn a_settings_file_without_theme_or_window_still_loads() {
        let older = r#"{"auto_lock_secs":240,"clipboard_clear_secs":10}"#;
        let parsed: AppSettings = serde_json::from_str(older).expect("older file parses");
        assert_eq!(parsed.theme, ThemeChoice::System);
        assert_eq!(parsed.window, None);
    }

    #[test]
    fn the_theme_choice_cycles_back_to_system() {
        assert_eq!(ThemeChoice::System.next(), ThemeChoice::Light);
        assert_eq!(ThemeChoice::Light.next(), ThemeChoice::Dark);
        assert_eq!(ThemeChoice::Dark.next(), ThemeChoice::System);
    }

    /// Geometry from a hand-edited file must not open a window the user
    /// cannot use.
    #[test]
    fn unusable_window_geometry_is_rejected() {
        let usable = WindowBoundsSetting {
            x: 100.0,
            y: 80.0,
            width: 1200.0,
            height: 800.0,
        };
        assert!(usable.is_usable());
        assert!(
            !WindowBoundsSetting {
                width: MIN_WINDOW_WIDTH - 1.0,
                ..usable
            }
            .is_usable(),
            "below the window minimum"
        );
        assert!(
            !WindowBoundsSetting {
                height: MIN_WINDOW_HEIGHT - 1.0,
                ..usable
            }
            .is_usable(),
            "below the window minimum"
        );
        assert!(
            !WindowBoundsSetting {
                x: f32::NAN,
                ..usable
            }
            .is_usable(),
            "a non-finite coordinate is not a position"
        );
    }

    /// Undocking a laptop leaves the saved position on a screen that is no
    /// longer there. Restoring it put the window where nobody could drag it
    /// back, and the size-only check accepted every one of those positions.
    #[test]
    fn geometry_on_a_display_that_is_gone_is_rejected() {
        let built_in = WindowBoundsSetting {
            x: 0.0,
            y: 0.0,
            width: 1512.0,
            height: 982.0,
        };
        let on_built_in = WindowBoundsSetting {
            x: 100.0,
            y: 80.0,
            width: 1200.0,
            height: 800.0,
        };
        // Where an external display to the right of the built-in one was.
        let on_unplugged_display = WindowBoundsSetting {
            x: 2000.0,
            ..on_built_in
        };

        assert!(on_unplugged_display.is_usable(), "the size is fine");
        assert!(on_built_in.is_reachable_on(&[built_in]));
        assert!(!on_unplugged_display.is_reachable_on(&[built_in]));
        assert!(
            on_unplugged_display.is_reachable_on(&[
                built_in,
                WindowBoundsSetting {
                    x: 1512.0,
                    ..built_in
                }
            ]),
            "plugging the display back in restores the saved position"
        );
        assert!(
            on_unplugged_display.is_reachable_on(&[]),
            "an unknown display layout keeps the user's geometry"
        );
    }

    /// A window dragged mostly off the edge still has to come back where the
    /// user left it. Only the unreachable case is rejected.
    #[test]
    fn geometry_hanging_off_a_screen_edge_is_kept() {
        let screen = WindowBoundsSetting {
            x: 0.0,
            y: 0.0,
            width: 1512.0,
            height: 982.0,
        };
        let mostly_off_the_right = WindowBoundsSetting {
            x: 1400.0,
            y: 40.0,
            width: 1200.0,
            height: 800.0,
        };
        assert!(mostly_off_the_right.is_reachable_on(&[screen]));
    }

    #[test]
    fn launch_cleanup_secs_clamps_to_range() {
        assert_eq!(with_launch_cleanup(0).launch_cleanup_secs_clamped(), 10);
        assert_eq!(with_launch_cleanup(9999).launch_cleanup_secs_clamped(), 60);
        assert_eq!(with_launch_cleanup(30).launch_cleanup_secs_clamped(), 30);
    }

    #[test]
    fn load_missing_returns_defaults() {
        let dir = TempDir::new().unwrap();
        let loaded = load_in(dir.path()).unwrap();
        assert_eq!(loaded, AppSettings::default());
    }

    #[test]
    fn load_corrupt_returns_defaults_not_error() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join(FILE_NAME), "{ bogus json").unwrap();
        let loaded = load_in(dir.path()).unwrap();
        // Must recover gracefully - don't brick the app on a stray file.
        assert_eq!(loaded, AppSettings::default());
    }

    #[test]
    fn never_options_serialize_as_null() {
        // Belt-and-braces: the UI's "Never" option must round-trip
        // through JSON as `null`, not be silently coerced to 0.
        let s = AppSettings {
            auto_lock_secs: None,
            clipboard_clear_secs: None,
            ..AppSettings::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"auto_lock_secs\":null"));
        assert!(json.contains("\"clipboard_clear_secs\":null"));
    }

    /// Old settings files (written before auto_type_* existed) must
    /// deserialize cleanly with documented defaults applied. Same
    /// shape as the launch_cleanup_secs back-compat check, but a
    /// regression here would be louder: the feature would either
    /// fail to load or default to the wrong combo on upgrade.
    /// settings.json written before the Touch ID feature shipped
    /// must deserialise cleanly with the documented default applied.
    /// A regression here would silently flip every upgrading user
    /// to "biometry-only" - breaking the clamshell-mode unlock
    /// flow they may rely on without ever opening Settings.
    #[test]
    fn missing_biometric_fallback_uses_default_true() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(FILE_NAME),
            r#"{"auto_lock_secs":60,"clipboard_clear_secs":null,"auto_update_check_enabled":true,"launch_cleanup_secs":30,"auto_type_enabled":false,"auto_type_hotkey":"ctrl+alt+v","auto_type_sequence":"{USERNAME}"}"#,
        )
        .unwrap();
        let loaded = load_in(dir.path()).unwrap();
        assert!(loaded.biometric_allow_passcode_fallback);
    }

    /// settings.json written before auto-sync shipped must deserialise
    /// with the feature ON at the 15-min default - a regression here
    /// would silently leave upgrading users with no keep-alive, which
    /// is the precise failure mode the feature was added to fix.
    #[test]
    fn missing_auto_sync_secs_uses_default_on() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(FILE_NAME),
            r#"{"auto_lock_secs":60,"clipboard_clear_secs":null,"auto_update_check_enabled":true,"launch_cleanup_secs":30}"#,
        )
        .unwrap();
        let loaded = load_in(dir.path()).unwrap();
        assert_eq!(loaded.auto_sync_secs, Some(DEFAULT_AUTO_SYNC_SECS));
    }

    /// A hand-edited or corrupt sub-floor interval must be clamped up to
    /// the 60 s floor, while `None` (= "Never") passes through untouched.
    fn with_auto_sync(secs: Option<u64>) -> AppSettings {
        AppSettings {
            auto_sync_secs: secs,
            ..AppSettings::default()
        }
    }

    #[test]
    fn auto_sync_secs_clamps_to_floor_but_keeps_none() {
        assert_eq!(
            with_auto_sync(Some(1)).auto_sync_secs_clamped(),
            Some(AUTO_SYNC_SECS_FLOOR)
        );
        assert_eq!(
            with_auto_sync(Some(1800)).auto_sync_secs_clamped(),
            Some(1800)
        );
        assert_eq!(with_auto_sync(None).auto_sync_secs_clamped(), None);
    }

    #[test]
    fn missing_auto_type_fields_use_defaults() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(FILE_NAME),
            r#"{"auto_lock_secs":60,"clipboard_clear_secs":null,"auto_update_check_enabled":true,"launch_cleanup_secs":30}"#,
        )
        .unwrap();
        let loaded = load_in(dir.path()).unwrap();
        assert!(!loaded.auto_type_enabled, "opt-in default off");
        assert_eq!(loaded.auto_type_hotkey, default_auto_type_hotkey());
        assert_eq!(loaded.auto_type_sequence, default_auto_type_sequence());
    }
}

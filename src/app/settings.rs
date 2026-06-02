//! App-wide preferences (auto-lock / clipboard-clear timeouts) persisted
//! at `~/Library/Application Support/ferrispass/settings.json`.
//!
//! Only stores plain numbers — no secrets — so JSON is fine. Same atomic
//! write pattern as `sync/config.rs` and `app/recents.rs` (temp file +
//! fsync + rename).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const FILE_NAME: &str = "settings.json";

/// `None` on a timeout field means "disabled" — i.e. never auto-lock /
/// never auto-clear. We keep the type explicit (rather than a magic 0)
/// so the UI can distinguish "user picked Never" from "the file is
/// missing this field".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppSettings {
    pub auto_lock_secs: Option<u64>,
    pub clipboard_clear_secs: Option<u64>,
    /// When `true`, FerrisPass quietly checks GitHub Releases on app start
    /// (rate-limited to ~1×/24h) and surfaces a banner if a newer build
    /// is available. Off-by-default would be more privacy-conservative,
    /// but the security upside of fast patch propagation in a password
    /// manager is significant — net better default is on.
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
    /// use — surfacing that to users who didn't ask for it would be
    /// surprising. `#[serde(default)]` so pre-feature settings.json
    /// files deserialize cleanly (= `false`, matching the cold-start
    /// behaviour).
    #[serde(default)]
    pub auto_type_enabled: bool,
    /// User-tunable global hotkey combo, in `global-hotkey` parse
    /// format (e.g. `ctrl+alt+super+KeyV`). The default matches
    /// KeePassXC's macOS default. Validated at registration time —
    /// a bad combo leaves the feature off with a Settings-tab error.
    #[serde(default = "default_auto_type_hotkey")]
    pub auto_type_hotkey: String,
    /// Auto-type sequence template (KeePass placeholder grammar). The
    /// default mirrors `{USERNAME}{TAB}{PASSWORD}{ENTER}` — the
    /// canonical login-form sequence used by ~every browser-form on
    /// the web. Per-entry overrides are not in v1; this is the global
    /// template.
    #[serde(default = "default_auto_type_sequence")]
    pub auto_type_sequence: String,
    /// When `true`, the Touch ID unlock prompt also accepts the
    /// user's macOS account password as a fallback (LAPolicy
    /// `DeviceOwnerAuthentication`). Lets the user unlock the vault
    /// in clamshell mode — the built-in Touch ID sensor is
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
    /// Default `true` — the product call here is "convenience over
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
}

fn default_true() -> bool {
    true
}

fn default_launch_cleanup_secs() -> u32 {
    DEFAULT_LAUNCH_CLEANUP_SECS
}

fn default_auto_type_hotkey() -> String {
    crate::autotype::hotkey::DEFAULT_HOTKEY.to_string()
}

fn default_auto_type_sequence() -> String {
    crate::autotype::sequence::DEFAULT_SEQUENCE.to_string()
}

pub const DEFAULT_LAUNCH_CLEANUP_SECS: u32 = 30;
pub const LAUNCH_CLEANUP_SECS_RANGE: std::ops::RangeInclusive<u32> = 10..=60;

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
/// missing file (cold first run), parse failure (corrupt file — better
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
    /// must deserialize cleanly with the documented default applied —
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
    #[test]
    fn launch_cleanup_secs_clamps_to_range() {
        let mut s = AppSettings::default();
        s.launch_cleanup_secs = 0;
        assert_eq!(s.launch_cleanup_secs_clamped(), 10);
        s.launch_cleanup_secs = 9999;
        assert_eq!(s.launch_cleanup_secs_clamped(), 60);
        s.launch_cleanup_secs = 30;
        assert_eq!(s.launch_cleanup_secs_clamped(), 30);
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
        // Must recover gracefully — don't brick the app on a stray file.
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
    /// to "biometry-only" — breaking the clamshell-mode unlock
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

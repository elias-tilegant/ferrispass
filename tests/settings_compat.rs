//! Every `settings.json` this build writes has to load into 0.9.2.
//!
//! Someone who tries a newer build and goes back to 0.9.2 keeps the same
//! settings file. 0.9.2 parses it with `serde_json` and, on any error, falls
//! back to the defaults without saying so. A renamed field, a changed type or a
//! renamed value would quietly reset their auto-lock, clipboard and Touch ID
//! choices. New fields are fine: 0.9.2 ignores what it does not know, which is
//! why the copy below has no `deny_unknown_fields`.

use ferrispass::app::settings::{AppSettings, ThemeChoice, WindowBoundsSetting};
use ferrispass::keepass::limits::{Argon2Variant, KdfCost};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// Frozen copy of the 0.9.2 struct, never edited. Field names, types and serde
// attributes are those of `AppSettings`, `ThemeChoice`, `WindowBoundsSetting`
// and `KdfCost` as tagged in v0.9.2. If a test here fails, rethink the change
// to `AppSettings`, not this copy.

#[derive(Debug, Serialize, Deserialize)]
struct Settings092 {
    auto_lock_secs: Option<u64>,
    clipboard_clear_secs: Option<u64>,
    #[serde(default = "default_true")]
    auto_update_check_enabled: bool,
    #[serde(default = "default_launch_cleanup_secs")]
    launch_cleanup_secs: u32,
    #[serde(default)]
    auto_type_enabled: bool,
    #[serde(default = "default_auto_type_hotkey")]
    auto_type_hotkey: String,
    #[serde(default = "default_auto_type_sequence")]
    auto_type_sequence: String,
    #[serde(default = "default_true")]
    biometric_allow_passcode_fallback: bool,
    #[serde(default = "default_auto_sync_secs")]
    auto_sync_secs: Option<u64>,
    #[serde(default)]
    theme: ThemeChoice092,
    #[serde(default)]
    window: Option<WindowBoundsSetting092>,
    #[serde(default)]
    confirmed_kdf_costs: Vec<KdfCost092>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ThemeChoice092 {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Serialize, Deserialize)]
struct WindowBoundsSetting092 {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Debug, Serialize, Deserialize)]
struct KdfCost092 {
    variant: Argon2Variant092,
    memory: u64,
    iterations: u64,
    parallelism: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Argon2Variant092 {
    Argon2d,
    Argon2id,
}

fn default_true() -> bool {
    true
}

fn default_launch_cleanup_secs() -> u32 {
    30
}

fn default_auto_type_hotkey() -> String {
    "ctrl+alt+super+KeyV".to_string()
}

fn default_auto_type_sequence() -> String {
    "{USERNAME}{TAB}{PASSWORD}{ENTER}".to_string()
}

fn default_auto_sync_secs() -> Option<u64> {
    Some(900)
}

/// Settings with nothing left at its default: every option set, every flag
/// flipped, a window and two Argon2 costs. A field added to `AppSettings` has
/// to be set here too, so the round trip writes it out.
fn every_field_set(theme: ThemeChoice) -> AppSettings {
    AppSettings {
        auto_lock_secs: Some(60),
        clipboard_clear_secs: Some(25),
        auto_update_check_enabled: false,
        launch_cleanup_secs: 45,
        auto_type_enabled: true,
        auto_type_hotkey: "ctrl+shift+KeyA".to_string(),
        auto_type_sequence: "{PASSWORD}{ENTER}".to_string(),
        biometric_allow_passcode_fallback: false,
        auto_sync_secs: Some(300),
        theme,
        window: Some(WindowBoundsSetting {
            x: 40.0,
            y: 64.5,
            width: 1280.0,
            height: 812.25,
        }),
        confirmed_kdf_costs: vec![
            KdfCost {
                variant: Argon2Variant::Argon2d,
                memory: 256 * 1024 * 1024,
                iterations: 12,
                parallelism: 4,
            },
            KdfCost {
                variant: Argon2Variant::Argon2id,
                memory: 64 * 1024 * 1024,
                iterations: 3,
                parallelism: 2,
            },
        ],
    }
}

/// Write `settings` the way the app does and read the file back the way 0.9.2
/// does. Every field 0.9.2 knows has to come through with the value written.
fn assert_loads_into_092(settings: &AppSettings, case: &str) {
    let written = serde_json::to_string_pretty(settings).expect("settings serialize");
    let loaded: Settings092 = serde_json::from_str(&written).unwrap_or_else(|error| {
        panic!("{case}: 0.9.2 cannot read this settings.json and would reset it: {error}")
    });

    let written: Value = serde_json::from_str(&written).expect("written JSON parses");
    let loaded = serde_json::to_value(&loaded).expect("0.9.2 settings serialize");
    let loaded = loaded.as_object().expect("0.9.2 settings are an object");
    for (field, value) in loaded {
        assert_eq!(
            written.get(field),
            Some(value),
            "{case}: 0.9.2 reads `{field}` differently from what was written"
        );
    }
}

#[test]
fn new_settings_load_into_the_092_struct() {
    assert_loads_into_092(&AppSettings::default(), "defaults");
    for theme in [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark] {
        assert_loads_into_092(
            &every_field_set(theme),
            &format!("every field set, {theme:?}"),
        );
    }
}

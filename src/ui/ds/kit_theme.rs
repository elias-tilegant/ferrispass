//! The GPUI Kit theme set built from the tokens, and the switch between its
//! four themes.
//!
//! `assets/theme/ferrispass.theme.json` holds light, dark, and both with
//! increased contrast. The golden test in `tokens/codegen.rs` writes it from
//! `design/tokens.json`; the app embeds it.

use std::rc::Rc;

use gpui::{App, Global, Window, px};
use gpui_component::{Theme, ThemeConfig, ThemeMode, ThemeSet};

use super::tokens::Mode;
use crate::app::settings::ThemeChoice;

/// The theme set as the golden test wrote it.
const THEME_SET: &str = include_str!("../../../assets/theme/ferrispass.theme.json");

/// The four themes, parsed. The file is generated and tested, so a parse
/// error is a build that should never have passed its tests.
pub fn theme_set() -> ThemeSet {
    serde_json::from_str(THEME_SET).expect("the embedded theme set parses")
}

/// The four configs, one per [`Mode`], in the order of [`Mode::ALL`]. They
/// stay out of the Kit's `ThemeRegistry` on purpose: its folder watcher
/// reloads the active themes by name whenever a theme file changes.
struct KitThemes([Rc<ThemeConfig>; 4]);

impl Global for KitThemes {}

impl KitThemes {
    fn load() -> Self {
        let set = theme_set();
        Self(Mode::ALL.map(|mode| {
            let config = set
                .themes
                .get(mode as usize)
                .expect("the theme set has one theme per mode");
            assert_eq!(config.mode.is_dark(), mode.is_dark(), "{}", config.name);
            Rc::new(config.clone())
        }))
    }

    fn config(&self, mode: Mode) -> Rc<ThemeConfig> {
        self.0[mode as usize].clone()
    }
}

/// Parses the theme set once and keeps it for the life of the app.
pub(super) fn install(cx: &mut App) {
    if !cx.has_global::<KitThemes>() {
        cx.set_global(KitThemes::load());
    }
}

/// Puts the Kit theme for `choice` into effect, with or without increased
/// contrast, at `text_size` points. Returns the mode it ended in.
pub(super) fn switch(
    choice: ThemeChoice,
    increased_contrast: bool,
    text_size: f32,
    window: Option<&mut Window>,
    cx: &mut App,
) -> Mode {
    install(cx);
    let (light, dark) = {
        let themes = cx.global::<KitThemes>();
        (
            themes.config(Mode::new(false, increased_contrast)),
            themes.config(Mode::new(true, increased_contrast)),
        )
    };
    // The configs first, without touching the mode. A new config and a new
    // mode in the same update count as already installed: the mode would
    // flip and the colors would stay.
    Theme::update(cx, |theme| {
        theme.light_theme = light;
        theme.dark_theme = dark;
    });
    // `change` loads the config of the mode even when the mode stays the same.
    match choice {
        ThemeChoice::System => Theme::sync_system_appearance(window, cx),
        ThemeChoice::Light => Theme::change(ThemeMode::Light, window, cx),
        ThemeChoice::Dark => Theme::change(ThemeMode::Dark, window, cx),
    }
    // A font size in the config would win on every change, so the theme set
    // has none and the size goes in after the change.
    Theme::update(cx, |theme| {
        theme.font_size = px(text_size);
        theme.focus_ring = true;
    });
    Mode::new(Theme::global(cx).mode.is_dark(), increased_contrast)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gpui::TestAppContext;
    use gpui_component::{ThemeConfigColors, try_parse_color};
    use serde_json::Value;

    use super::*;

    fn color_keys(theme: &Value) -> BTreeSet<String> {
        theme["colors"]
            .as_object()
            .expect("colors")
            .iter()
            .filter(|(_, value)| !value.is_null())
            .map(|(key, _)| key.clone())
            .collect()
    }

    /// serde drops a key the Kit schema does not know without a word, and the
    /// Kit swaps a color its parser rejects for its own default, again
    /// without a word. So: every one of the schema's color keys is set, each
    /// survives a round trip through `ThemeSet`, and each value parses.
    #[test]
    fn kit_keys_survive_roundtrip() {
        let schema: BTreeSet<String> = serde_json::to_value(ThemeConfigColors::default())
            .expect("serialize the schema")
            .as_object()
            .expect("object")
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            schema.len(),
            140,
            "the Kit schema changed, check the mapping"
        );

        let raw: Value = serde_json::from_str(THEME_SET).expect("theme set json");
        let round_trip = serde_json::to_value(theme_set()).expect("serialize the theme set");
        let before = raw["themes"].as_array().expect("themes");
        let after = round_trip["themes"].as_array().expect("themes");
        assert_eq!(before.len(), 4);
        assert_eq!(after.len(), 4);
        for (before, after) in before.iter().zip(after) {
            let name = &before["name"];
            assert_eq!(color_keys(before), schema, "{name} does not set every key");
            assert_eq!(
                color_keys(after),
                schema,
                "{name} lost keys in the round trip"
            );
            for (key, value) in before["colors"].as_object().expect("colors") {
                let value = value.as_str().expect("a color is a string");
                assert!(try_parse_color(value).is_ok(), "{name}: {key} = {value}");
                assert_eq!(after["colors"][key], value, "{name}: {key}");
            }
            for key in ["name", "mode", "font.family", "mono_font.family", "shadow"] {
                assert_eq!(after[key], before[key], "{name}: {key}");
            }
            for key in ["mono_font.size", "radius", "radius.lg"] {
                assert!(before[key].is_number(), "{name}: {key}");
                assert_eq!(after[key].as_f64(), before[key].as_f64(), "{name}: {key}");
            }
            // Set by `ds::apply` after each theme change instead.
            assert!(before.get("font.size").is_none(), "{name} sets font.size");
        }
    }

    /// What the Kit shows: background, separator, copper, a hover state, and
    /// the theme's name.
    fn assert_kit_shows(mode: Mode, cx: &App) {
        let theme = Theme::global(cx);
        let tokens = mode.colors();
        let config = cx.global::<KitThemes>().config(mode);
        assert_eq!(theme.mode.is_dark(), mode.is_dark(), "{mode:?}");
        assert_eq!(theme.theme_name(), &config.name, "{mode:?}");
        assert_eq!(theme.background, tokens.bg_window.hsla, "{mode:?}");
        assert_eq!(theme.border, tokens.border_separator.hsla, "{mode:?}");
        assert_eq!(theme.input, tokens.border_control.hsla, "{mode:?}");
        assert_eq!(theme.primary, tokens.accent_fill.hsla, "{mode:?}");
        assert_eq!(
            theme.primary_hover, tokens.accent_fill_hover.hsla,
            "{mode:?}"
        );
        assert_eq!(
            theme.tokens.primary.color, tokens.accent_fill.hsla,
            "{mode:?}"
        );
        assert_eq!(theme.radius, px(8.));
        assert_eq!(theme.radius_lg, px(16.));
        assert!(!theme.shadow);
        // The text size survives the change, and so does the focus ring.
        assert_eq!(theme.font_size, px(13.));
        assert!(theme.focus_ring);
    }

    /// The switch reaches every one of the four configs from every other,
    /// including the case the Kit gets wrong when configs and mode change in
    /// one update: dark to light with increased contrast at once.
    #[gpui::test]
    fn hc_configs_switch_with_theme_change(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            let steps = [
                (ThemeChoice::Light, false, Mode::Light),
                (ThemeChoice::Light, true, Mode::LightHc),
                (ThemeChoice::Dark, true, Mode::DarkHc),
                (ThemeChoice::Dark, false, Mode::Dark),
                (ThemeChoice::Light, true, Mode::LightHc),
                (ThemeChoice::Dark, false, Mode::Dark),
                (ThemeChoice::Light, false, Mode::Light),
                (ThemeChoice::Dark, true, Mode::DarkHc),
                (ThemeChoice::Light, false, Mode::Light),
            ];
            for (choice, increased_contrast, expected) in steps {
                // Something else set a size in between; the switch puts 13 back.
                Theme::update(cx, |theme| theme.font_size = px(20.));
                let mode = switch(choice, increased_contrast, 13., None, cx);
                assert_eq!(
                    mode, expected,
                    "{choice:?}, increased contrast {increased_contrast}"
                );
                assert_kit_shows(mode, cx);
            }
            // System follows what the platform reports, light on the test platform.
            let mode = switch(ThemeChoice::System, true, 13., None, cx);
            assert_eq!(
                mode.is_dark(),
                ThemeMode::from(cx.window_appearance()).is_dark()
            );
            assert!(mode.increased_contrast());
            assert_kit_shows(mode, cx);
        });
    }
}

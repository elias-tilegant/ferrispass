//! The GPUI Kit theme set built from the tokens.
//!
//! `assets/theme/ferrispass.theme.json` holds four themes: light, dark, and
//! both with increased contrast. The golden test in `tokens/codegen.rs`
//! writes it from `design/tokens.json`; the app embeds it.

use gpui_component::ThemeSet;

/// The theme set as the golden test wrote it.
const THEME_SET: &str = include_str!("../../../assets/theme/ferrispass.theme.json");

/// The four themes, parsed. The file is generated and tested, so a parse
/// error is a build that should never have passed its tests.
pub fn theme_set() -> ThemeSet {
    serde_json::from_str(THEME_SET).expect("the embedded theme set parses")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

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
}

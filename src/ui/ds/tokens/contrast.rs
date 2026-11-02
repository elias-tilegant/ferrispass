//! WCAG 2.x contrast and OKLCH, and the tests that hold the tokens to the
//! contrast contract in `design/tokens.json`.
//!
//! Everything here works on the hex values in `generated.rs` (`Color::rgb`),
//! not on `Hsla`, so no rounding sits between a token and its ratio.

/// One sRGB channel (the low byte of `value`), linearised as WCAG 2.x does,
/// with the 0.04045 threshold of the sRGB standard.
fn channel(value: u32) -> f64 {
    let c = f64::from(value & 0xFF) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Relative luminance of an opaque `0xRRGGBB` color, from 0 to 1.
pub fn luminance(rgb: u32) -> f64 {
    0.2126 * channel(rgb >> 16) + 0.7152 * channel(rgb >> 8) + 0.0722 * channel(rgb)
}

/// Contrast ratio of two opaque colors, from 1 to 21. The order does not
/// matter.
pub fn ratio(a: u32, b: u32) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// OKLCH of an opaque `0xRRGGBB` color: lightness from 0 to 1, chroma, and
/// hue in degrees. Matrices from Björn Ottosson's OKLab.
pub fn oklch(rgb: u32) -> [f64; 3] {
    let (r, g, b) = (channel(rgb >> 16), channel(rgb >> 8), channel(rgb));
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    let lightness = 0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s;
    let a = 1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s;
    let b = 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s;
    [
        lightness,
        a.hypot(b),
        b.atan2(a).to_degrees().rem_euclid(360.0),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::Value;

    use super::super::codegen::{Tokens, repo_file};
    use super::super::{Color, Mode};
    use super::*;

    fn color(name: &str, mode: Mode) -> Color {
        mode.colors()
            .by_name(name)
            .unwrap_or_else(|| panic!("{name} is not a token"))
    }

    fn mode_named(name: &str) -> Mode {
        Mode::ALL
            .into_iter()
            .find(|mode| mode.name() == name)
            .unwrap_or_else(|| panic!("no mode {name:?}"))
    }

    /// One line of `contrast.pairs`.
    struct Pair<'a> {
        fg: &'a str,
        also: Vec<&'a str>,
        /// The surfaces, as written: a set name or a list.
        on_key: String,
        on: Vec<&'a str>,
        /// Normal and increased contrast. `None` is decorative.
        min: [Option<f64>; 2],
        documented: Option<&'a Value>,
    }

    impl Pair<'_> {
        /// The lowest ratio over the pair's surfaces.
        fn lowest(&self, mode: Mode) -> f64 {
            let fg = color(self.fg, mode).rgb;
            self.on
                .iter()
                .map(|surface| ratio(fg, color(surface, mode).rgb))
                .fold(f64::INFINITY, f64::min)
        }

        fn minimum(&self, mode: Mode) -> Option<f64> {
            self.min[usize::from(mode.increased_contrast())]
        }

        fn key(&self, mode: Mode) -> (String, String, &'static str) {
            (self.fg.to_string(), self.on_key.clone(), mode.name())
        }
    }

    fn strings(value: &Value) -> Vec<&str> {
        value
            .as_array()
            .map(|list| list.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }

    fn pairs(json: &Value) -> Vec<Pair<'_>> {
        let contrast = &json["contrast"];
        contrast["pairs"]
            .as_array()
            .expect("contrast.pairs")
            .iter()
            .map(|pair| {
                let on = match &pair["on"] {
                    Value::String(set) => strings(&contrast["sets"][set]),
                    list => strings(list),
                };
                assert!(!on.is_empty(), "{pair}: no surfaces");
                let min = pair["min"].as_array().expect("min is [normal, increased]");
                Pair {
                    fg: pair["fg"].as_str().expect("fg"),
                    also: strings(&pair["also"]),
                    on_key: pair["on"].to_string(),
                    on,
                    min: [min[0].as_f64(), min[1].as_f64()],
                    documented: pair.get("documented"),
                }
            })
            .collect()
    }

    /// The exemptions as (fg, surfaces, mode), each with its reason checked.
    fn exemptions(json: &Value) -> BTreeSet<(String, String, &'static str)> {
        let mut exempt = BTreeSet::new();
        for entry in json["exempt"].as_array().expect("exempt") {
            let reason = entry["reason"].as_str().unwrap_or_default();
            assert!(reason.len() > 20, "{entry}: an exemption needs a reason");
            for mode in strings(&entry["modes"]) {
                exempt.insert((
                    entry["fg"].as_str().expect("fg").to_string(),
                    entry["on"].to_string(),
                    mode_named(mode).name(),
                ));
            }
        }
        exempt
    }

    /// Every pair reaches its minimum in every mode, unless it is exempt
    /// there. The ratio is not rounded first, as WCAG asks.
    #[test]
    fn contract_holds_in_all_modes() {
        let tokens = Tokens::load();
        let exempt = exemptions(tokens.json());
        let mut failures = Vec::new();
        for pair in pairs(tokens.json()) {
            for mode in Mode::ALL {
                for alias in &pair.also {
                    assert_eq!(
                        color(alias, mode),
                        color(pair.fg, mode),
                        "{alias} in {mode:?}"
                    );
                }
                if exempt.contains(&pair.key(mode)) {
                    continue;
                }
                let lowest = pair.lowest(mode);
                match pair.minimum(mode) {
                    Some(min) if lowest >= min => {}
                    Some(min) => failures.push(format!(
                        "{} on {} in {}: {lowest:.3} < {min}",
                        pair.fg,
                        pair.on_key,
                        mode.name()
                    )),
                    None => failures.push(format!(
                        "{} on {} in {}: decorative, but not exempt",
                        pair.fg,
                        pair.on_key,
                        mode.name()
                    )),
                }
            }
        }
        assert!(
            failures.is_empty(),
            "contrast contract broken:\n{}",
            failures.join("\n")
        );
    }

    /// The ratios the design spec states for light and dark still come out,
    /// rounded to two places. Changing a color means changing these on
    /// purpose.
    #[test]
    fn matches_documented_ratios() {
        let tokens = Tokens::load();
        let mut checked = 0;
        for pair in pairs(tokens.json()) {
            let Some(documented) = pair.documented.and_then(Value::as_object) else {
                continue;
            };
            for (mode, expected) in documented {
                let mode = mode_named(mode);
                let expected = expected.as_f64().expect("a documented ratio");
                let rounded = (pair.lowest(mode) * 100.0).round() / 100.0;
                assert!(
                    (rounded - expected).abs() <= 0.005,
                    "{} on {} in {}: {rounded:.2}, documented {expected:.2}",
                    pair.fg,
                    pair.on_key,
                    mode.name()
                );
                checked += 1;
            }
        }
        assert!(checked >= 30, "only {checked} documented ratios found");
    }

    /// Where the contract does not hold (decorative pairs and pairs below
    /// their minimum) is exactly what `exempt` lists, each with a reason. A
    /// new gap without an entry fails, and so does an entry nothing needs.
    #[test]
    fn exemptions_are_explicit() {
        let tokens = Tokens::load();
        let json = tokens.json();
        // bg.selected against bg.surface, border.separator and border.keycap.
        // A fourth one is a design decision: change this number with it.
        assert_eq!(json["exempt"].as_array().map(Vec::len), Some(3));
        let mut gaps = BTreeSet::new();
        for pair in pairs(json) {
            for mode in Mode::ALL {
                let met = pair
                    .minimum(mode)
                    .is_some_and(|min| pair.lowest(mode) >= min);
                if !met {
                    gaps.insert(pair.key(mode));
                }
            }
        }
        assert_eq!(gaps, exemptions(json));
    }

    /// One warm neutral family: the listed backgrounds and borders sit
    /// between hue 55 and 90, and in the dark modes their chroma stays under
    /// the limit.
    #[test]
    fn neutral_family() {
        let tokens = Tokens::load();
        let family = &tokens.json()["neutral_family"];
        let hue = family["hue"].as_array().expect("hue range");
        let (low, high) = (hue[0].as_f64().unwrap(), hue[1].as_f64().unwrap());
        let dark_max = family["dark_max_chroma"].as_f64().expect("dark_max_chroma");
        for mode in Mode::ALL {
            for name in strings(&family["tokens"]) {
                let [_, chroma, hue] = oklch(color(name, mode).rgb);
                // White has no hue to speak of.
                if chroma > 0.001 {
                    assert!(
                        (low..=high).contains(&hue),
                        "{name} in {}: hue {hue:.1} is outside {low} to {high}",
                        mode.name()
                    );
                }
                if mode.is_dark() {
                    assert!(
                        chroma < dark_max,
                        "{name} in {}: chroma {chroma:.4} is not below {dark_max}",
                        mode.name()
                    );
                }
            }
        }
    }

    fn hue_distance(a: f64, b: f64) -> f64 {
        let d = (a - b).rem_euclid(360.0);
        d.min(360.0 - d)
    }

    /// Vault colors keep their distance from the hues that carry a meaning
    /// (copper, ochre, carmine, blue). A team color too grey to read as a
    /// hue is exempt only when `low_chroma` lists it.
    #[test]
    fn hue_separation() {
        let tokens = Tokens::load();
        let rule = &tokens.json()["hue_separation"];
        let min_distance = rule["min_distance"].as_f64().expect("min_distance");
        let min_chroma = rule["min_chroma"].as_f64().expect("min_chroma");
        let low_chroma = strings(&rule["low_chroma"]);
        for mode in Mode::ALL {
            for team in strings(&rule["team"]) {
                let [_, chroma, hue] = oklch(color(team, mode).rgb);
                assert_eq!(
                    chroma < min_chroma,
                    low_chroma.contains(&team),
                    "{team} in {}: chroma {chroma:.3}, low_chroma lists it: {}",
                    mode.name(),
                    low_chroma.contains(&team)
                );
                if chroma < min_chroma {
                    continue;
                }
                for reserved in strings(&rule["reserved"]) {
                    let [_, _, other] = oklch(color(reserved, mode).rgb);
                    let distance = hue_distance(hue, other);
                    assert!(
                        distance >= min_distance,
                        "{team} is {distance:.1} degrees from {reserved} in {}",
                        mode.name()
                    );
                }
            }
        }
    }

    /// Every color in the generated GPUI Kit theme set is the hex of the
    /// token `kit.colors` maps it to, in each of the four themes, and the
    /// radii are the ones in `dimensions`.
    #[test]
    fn kit_theme_matches_tokens() {
        let tokens = Tokens::load();
        let kit = &tokens.json()["kit"];
        let mapping = kit["colors"].as_object().expect("kit.colors");
        let path = repo_file("assets/theme/ferrispass.theme.json");
        let set: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("theme set"))
            .expect("theme set json");
        let themes = set["themes"].as_array().expect("themes");
        assert_eq!(themes.len(), 4);
        let dimensions = &tokens.json()["dimensions"];
        for theme in themes {
            let name = theme["name"].as_str().expect("theme name");
            let mode = Mode::ALL
                .into_iter()
                .find(|mode| kit["themes"][mode.name()] == name)
                .unwrap_or_else(|| panic!("{name} is not in kit.themes"));
            assert_eq!(theme["mode"], if mode.is_dark() { "dark" } else { "light" });
            assert_eq!(theme["radius"].as_f64(), dimensions["radius.md"].as_f64());
            assert_eq!(
                theme["radius.lg"].as_f64(),
                dimensions["radius.xl"].as_f64()
            );

            let colors = theme["colors"].as_object().expect("colors");
            let keys: BTreeSet<&String> = colors.keys().collect();
            let mapped: BTreeSet<&String> = mapping.keys().collect();
            assert_eq!(keys, mapped, "{name}: keys differ from kit.colors");
            for (key, value) in colors {
                let token = mapping[key].as_str().expect("token name");
                let expected = color(token, mode);
                let hex = if expected.alpha() == 0xFF {
                    format!("#{:06X}", expected.rgb)
                } else {
                    format!("#{:06X}{:02X}", expected.rgb, expected.alpha())
                };
                assert_eq!(
                    value.as_str(),
                    Some(hex.as_str()),
                    "{name}: {key} ({token})"
                );
            }
        }
    }

    #[test]
    fn luminance_and_ratio_know_the_ends() {
        assert_eq!(luminance(0x000000), 0.0);
        assert!((luminance(0xFFFFFF) - 1.0).abs() < 1e-12);
        assert!((ratio(0x000000, 0xFFFFFF) - 21.0).abs() < 1e-9);
        assert_eq!(ratio(0x777777, 0xFFFFFF), ratio(0xFFFFFF, 0x777777));
        // The threshold falls between the bytes 10 and 11: 10 is still on the
        // linear segment, 11 on the curve.
        assert!((channel(10) - 10.0 / 255.0 / 12.92).abs() < 1e-15);
        let curve = ((11.0 / 255.0 + 0.055) / 1.055_f64).powf(2.4);
        assert!((channel(11) - curve).abs() < 1e-15);
    }
}

//! The generators behind `generated.rs` and the GPUI Kit theme set, and the
//! golden tests that keep both in step with `design/tokens.json`.
//!
//! Nothing here runs in the app. The outputs are checked in and there is no
//! build script: each test writes its file again and compares it byte for
//! byte. After changing the JSON, `FP_UPDATE_GOLDEN=1 cargo test golden`
//! rewrites it.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::path::PathBuf;

use serde_json::{Map, Value};

use super::Mode;

/// A file in the repository.
pub(super) fn repo_file(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// The groups that hold color tokens. A token name is unique across all of
/// them.
const TOKEN_GROUPS: [&str; 4] = ["semantic", "aliases", "brand", "components"];

/// `design/tokens.json`, parsed.
pub(super) struct Tokens {
    json: Value,
}

/// A color value after following its references.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Resolved {
    pub rgb: u32,
    pub alpha: u8,
    /// The primitive it ends in, when it is that primitive unchanged.
    pub primitive: Option<String>,
}

impl Resolved {
    /// `#RRGGBB`, or `#RRGGBBAA` when the color is not opaque.
    pub fn hex(&self) -> String {
        if self.alpha == 0xFF {
            format!("#{:06X}", self.rgb)
        } else {
            format!("#{:06X}{:02X}", self.rgb, self.alpha)
        }
    }
}

impl Tokens {
    pub(super) fn load() -> Self {
        let path = repo_file("design/tokens.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let json = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()));
        Self { json }
    }

    pub(super) fn json(&self) -> &Value {
        &self.json
    }

    fn object(&self, key: &str) -> &Map<String, Value> {
        self.json[key]
            .as_object()
            .unwrap_or_else(|| panic!("tokens.json has no object {key:?}"))
    }

    /// The primitives by family, then by lightness.
    pub(super) fn primitives(&self) -> Vec<(&str, &Value)> {
        sorted(self.object("primitives"))
    }

    /// The color tokens of one group, in name order.
    fn group(&self, group: &str) -> Vec<(&str, &Value)> {
        sorted(self.object(group))
    }

    fn token(&self, name: &str) -> Option<&Value> {
        TOKEN_GROUPS
            .iter()
            .find_map(|group| self.json[group].get(name))
    }

    /// The color a token or primitive name stands for in `mode`.
    pub(super) fn resolve_name(&self, name: &str, mode: Mode) -> Resolved {
        self.resolve(&Value::String(name.to_string()), mode, 0)
    }

    /// Follows a value to its color: a primitive, another token, a hex color
    /// with transparency, or `{ "ref": .., "alpha": .. }`.
    fn resolve(&self, value: &Value, mode: Mode, depth: usize) -> Resolved {
        assert!(depth < 16, "tokens.json: the references loop at {value}");
        match value {
            Value::String(hex) if hex.starts_with('#') => {
                let (rgb, alpha) = parse_hex(hex);
                assert!(
                    hex.len() == 9 && alpha != 0xFF,
                    "tokens.json: {hex} is written out, but only colors with transparency may be; \
                     add a primitive instead"
                );
                Resolved {
                    rgb,
                    alpha,
                    primitive: None,
                }
            }
            Value::String(name) => {
                if let Some(primitive) = self.json["primitives"].get(name) {
                    let hex = primitive["hex"].as_str().expect("primitive hex");
                    let (rgb, alpha) = parse_hex(hex);
                    assert!(
                        hex.len() == 7,
                        "tokens.json: primitive {name} is not #RRGGBB"
                    );
                    return Resolved {
                        rgb,
                        alpha,
                        primitive: Some(name.clone()),
                    };
                }
                let entry = self
                    .token(name)
                    .unwrap_or_else(|| panic!("tokens.json: no token or primitive {name:?}"));
                self.resolve(value_in(name, entry, mode), mode, depth + 1)
            }
            Value::Object(map) if map.contains_key("ref") => {
                let base = self.resolve(&map["ref"], mode, depth + 1);
                assert_eq!(base.alpha, 0xFF, "tokens.json: {value} adds alpha twice");
                let alpha = map["alpha"]
                    .as_str()
                    .and_then(|alpha| u8::from_str_radix(alpha, 16).ok())
                    .unwrap_or_else(|| panic!("tokens.json: {value} needs a hex byte as alpha"));
                Resolved {
                    rgb: base.rgb,
                    alpha,
                    primitive: None,
                }
            }
            other => panic!("tokens.json: {other} is not a color"),
        }
    }
}

/// What an entry holds in `mode`. An entry with `value` is the same in every
/// mode; otherwise an increased contrast mode falls back to its base mode.
fn value_in<'a>(name: &str, entry: &'a Value, mode: Mode) -> &'a Value {
    if let Some(value) = entry.get("value") {
        return value;
    }
    let found = match mode {
        Mode::Light => entry.get("light"),
        Mode::Dark => entry.get("dark"),
        Mode::LightHc => entry.get("light-hc").or_else(|| entry.get("light")),
        Mode::DarkHc => entry.get("dark-hc").or_else(|| entry.get("dark")),
    };
    found.unwrap_or_else(|| panic!("tokens.json: {name} has no value for {}", mode.name()))
}

fn parse_hex(hex: &str) -> (u32, u8) {
    let digits = hex
        .strip_prefix('#')
        .filter(|digits| matches!(digits.len(), 6 | 8))
        .unwrap_or_else(|| panic!("tokens.json: {hex} is not #RRGGBB or #RRGGBBAA"));
    let value =
        u32::from_str_radix(digits, 16).unwrap_or_else(|_| panic!("tokens.json: {hex} is not hex"));
    if digits.len() == 6 {
        (value, 0xFF)
    } else {
        (value >> 8, (value & 0xFF) as u8)
    }
}

/// Dotted names in reading order: `space.2` before `space.12`, `bg.selected`
/// before `bg.selected.unfocused`.
fn natural(a: &str, b: &str) -> Ordering {
    let (mut left, mut right) = (a.split('.'), b.split('.'));
    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let order = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    _ => x.cmp(y),
                };
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

fn sorted(map: &Map<String, Value>) -> Vec<(&str, &Value)> {
    let mut entries: Vec<(&str, &Value)> = map.iter().map(|(k, v)| (k.as_str(), v)).collect();
    entries.sort_by(|a, b| natural(a.0, b.0));
    entries
}

/// `bg.selected.unfocused` as a field name.
fn field_name(name: &str) -> String {
    assert!(
        name.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_')),
        "tokens.json: {name:?} does not make a Rust name"
    );
    name.replace('.', "_")
}

/// `sand.974` or `space.12` as a constant name.
fn const_name(name: &str) -> String {
    field_name(name).to_ascii_uppercase()
}

/// A doc comment, wrapped at 80 columns of text.
fn doc(out: &mut String, indent: &str, text: &str) {
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > 80 {
            out.push_str(&format!("{indent}/// {line}\n"));
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push_str(&format!("{indent}/// {line}\n"));
    }
}

fn usage(entry: &Value) -> Option<&str> {
    entry.get("usage").and_then(Value::as_str)
}

/// A number from the JSON as an `f32` literal.
fn number(value: &Value) -> String {
    let number = value
        .as_f64()
        .unwrap_or_else(|| panic!("tokens.json: {value} is not a number"));
    format!("{:?}", number as f32)
}

/// A color as Rust source, at the indentation `indent` of its first line. The
/// `Hsla` is what GPUI Kit's own parser makes of the hex, so a token and a
/// Kit theme color built from the same hex compare equal.
fn color_source(color: &Resolved, indent: &str) -> String {
    if let Some(primitive) = &color.primitive {
        return format!("primitive::{}", const_name(primitive));
    }
    let hsla = gpui_component::try_parse_color(&format!("#{:06X}{:02X}", color.rgb, color.alpha))
        .expect("the Kit parses a hex color");
    format!(
        "Color {{\n\
         {indent}    rgb: 0x{rgb:06X},\n\
         {indent}    hsla: Hsla {{\n\
         {indent}        h: {h:?},\n\
         {indent}        s: {s:?},\n\
         {indent}        l: {l:?},\n\
         {indent}        a: {a:?},\n\
         {indent}    }},\n\
         {indent}}}",
        rgb = color.rgb,
        h = hsla.h,
        s = hsla.s,
        l = hsla.l,
        a = hsla.a,
    )
}

fn shadows(tokens: &Tokens, layers: &Value, mode: Mode) -> String {
    let mut out = String::from("[\n");
    for layer in layers.as_array().expect("a shadow is a list of layers") {
        let zero = Value::from(0);
        let color = tokens.resolve(&layer["color"], mode, 0);
        out.push_str(&format!(
            "        Shadow {{\n\
             \x20           offset_y: {y},\n\
             \x20           blur: {blur},\n\
             \x20           spread: {spread},\n\
             \x20           color: {color},\n\
             \x20           inset: {inset},\n\
             \x20       }},\n",
            y = number(layer.get("y").unwrap_or(&zero)),
            blur = number(layer.get("blur").unwrap_or(&zero)),
            spread = number(layer.get("spread").unwrap_or(&zero)),
            color = color_source(&color, "            "),
            inset = layer.get("inset").and_then(Value::as_bool).unwrap_or(false),
        ));
    }
    out.push_str("    ]");
    out
}

/// The shadow layers of an elevation in `mode`, with increased contrast
/// falling back to the base mode.
fn layers_in<'a>(name: &str, elevation: &'a Value, mode: Mode) -> &'a Value {
    let base = if mode.is_dark() { "dark" } else { "light" };
    let key = if mode.increased_contrast() {
        format!("{base}-hc")
    } else {
        base.to_string()
    };
    elevation
        .get(&key)
        .or_else(|| elevation.get(base))
        .unwrap_or_else(|| panic!("tokens.json: elevation {name} has no {base} layers"))
}

const STATICS: [(Mode, &str); 4] = [
    (Mode::Light, "LIGHT"),
    (Mode::Dark, "DARK"),
    (Mode::LightHc, "LIGHT_HC"),
    (Mode::DarkHc, "DARK_HC"),
];

/// `generated.rs`.
pub(super) fn rust(tokens: &Tokens) -> String {
    let mut out = String::new();
    out.push_str(
        "//! Generated from `design/tokens.json` by `codegen.rs`. Do not edit: change the\n\
         //! JSON and run `FP_UPDATE_GOLDEN=1 cargo test golden`.\n\
         \n\
         // The numbers are colors and sizes; one may land near a mathematical\n\
         // constant by chance.\n\
         #![allow(clippy::approx_constant)]\n\
         \n\
         use gpui::Hsla;\n\
         \n\
         use super::{Color, Shadow};\n\
         \n",
    );

    let typography = &tokens.json()["typography"];
    for (key, name, text) in [
        (
            "size.standard",
            "TEXT_SIZE_STANDARD",
            "The standard text size in points.",
        ),
        (
            "size.large",
            "TEXT_SIZE_LARGE",
            "The large text size in points.",
        ),
    ] {
        doc(&mut out, "", text);
        out.push_str(&format!(
            "pub const {name}: f32 = {};\n\n",
            number(&typography[key])
        ));
    }

    doc(
        &mut out,
        "",
        "The primitive colors. UI code reads the tokens in [`ColorSet`] instead.",
    );
    out.push_str("pub mod primitive {\n    use super::{Color, Hsla};\n");
    for (name, primitive) in tokens.primitives() {
        let hex = primitive["hex"].as_str().expect("primitive hex");
        let (rgb, alpha) = parse_hex(hex);
        let color = Resolved {
            rgb,
            alpha,
            primitive: None,
        };
        out.push_str(&format!(
            "\n    pub const {}: Color = {};\n",
            const_name(name),
            color_source(&color, "    ")
        ));
    }
    out.push_str("}\n\n");

    let notes = &tokens.json()["dimension_notes"];
    doc(
        &mut out,
        "",
        notes["units"]
            .as_str()
            .expect("dimension_notes.units explains the unit"),
    );
    out.push_str("pub mod dimension {\n");
    for (name, value) in sorted(tokens.object("dimensions")) {
        out.push_str(&format!(
            "    pub const {}: f32 = {};\n",
            const_name(name),
            number(value)
        ));
    }
    out.push_str("}\n\n");

    let groups: [(&str, &str); 4] = [
        ("semantic", "Semantic tokens."),
        (
            "aliases",
            "Aliases: a fixed name for a use, pointing at a semantic token.",
        ),
        (
            "brand",
            "Brand graphics only, never text and never a UI surface.",
        ),
        (
            "components",
            "Component tokens, where a component differs from the semantic layer.",
        ),
    ];
    let mut names = BTreeSet::new();
    let mut fields = BTreeSet::new();

    doc(
        &mut out,
        "",
        "Every color token for one mode, plus the shadows that go with it.",
    );
    out.push_str("#[derive(Debug, PartialEq)]\npub struct ColorSet {\n");
    for (group, title) in groups {
        out.push_str(&format!("    // {title}\n"));
        for (name, entry) in tokens.group(group) {
            assert!(names.insert(name), "tokens.json: {name} is defined twice");
            assert!(
                fields.insert(field_name(name)),
                "tokens.json: {name} clashes with another name"
            );
            if let Some(text) = usage(entry) {
                doc(&mut out, "    ", text);
            }
            out.push_str(&format!("    pub {}: Color,\n", field_name(name)));
        }
    }
    out.push_str("    // Shadows, one entry per layer.\n");
    let elevation = sorted(tokens.object("elevation"));
    for (name, entry) in &elevation {
        let count = entry["light"].as_array().map_or(0, Vec::len);
        if let Some(text) = usage(entry) {
            doc(&mut out, "    ", text);
        }
        out.push_str(&format!(
            "    pub {}: [Shadow; {count}],\n",
            field_name(name)
        ));
    }
    out.push_str("}\n");

    for (mode, static_name) in STATICS {
        out.push_str(&format!(
            "\npub static {static_name}: ColorSet = ColorSet {{\n"
        ));
        for (group, _) in groups {
            for (name, _) in tokens.group(group) {
                let color = tokens.resolve_name(name, mode);
                out.push_str(&format!(
                    "    {}: {},\n",
                    field_name(name),
                    color_source(&color, "    ")
                ));
            }
        }
        for (name, entry) in &elevation {
            let layers = layers_in(name, entry, mode);
            let count = entry["light"].as_array().map_or(0, Vec::len);
            assert_eq!(
                layers.as_array().map(Vec::len),
                Some(count),
                "tokens.json: elevation {name} needs {count} layers in every mode"
            );
            out.push_str(&format!(
                "    {}: {},\n",
                field_name(name),
                shadows(tokens, layers, mode)
            ));
        }
        out.push_str("};\n");
    }

    out.push_str(
        "\nimpl ColorSet {\n\
         \x20   /// The color of a token named as in `design/tokens.json`.\n\
         \x20   pub fn by_name(&self, name: &str) -> Option<Color> {\n\
         \x20       let color = match name {\n",
    );
    for (group, _) in groups {
        for (name, _) in tokens.group(group) {
            out.push_str(&format!(
                "            {name:?} => self.{},\n",
                field_name(name)
            ));
        }
    }
    out.push_str(
        "            _ => return None,\n\
         \x20       };\n\
         \x20       Some(color)\n\
         \x20   }\n\
         }\n",
    );
    out
}

/// `assets/theme/ferrispass.theme.json`: one GPUI Kit theme per mode, every
/// color key of the Kit schema set from a token.
pub(super) fn kit_theme_set(tokens: &Tokens) -> String {
    let kit = &tokens.json()["kit"];
    let settings = sorted(kit["settings"].as_object().expect("kit.settings"));
    // The Kit applies a config's font size on every theme change, over the
    // text size the app chose, so `ds::apply` sets it after the change.
    assert!(
        settings.iter().all(|(key, _)| *key != "font.size"),
        "tokens.json: kit.settings must not set font.size"
    );
    let mut colors: Vec<(&String, &Value)> = kit["colors"]
        .as_object()
        .expect("kit.colors")
        .iter()
        .collect();
    colors.sort_by(|a, b| a.0.cmp(b.0));

    let mut out = format!("{{\n  \"name\": {},\n  \"themes\": [\n", kit["name"]);
    for (index, mode) in Mode::ALL.into_iter().enumerate() {
        let name = &kit["themes"][mode.name()];
        assert!(
            name.is_string(),
            "tokens.json: kit.themes has no {}",
            mode.name()
        );
        out.push_str(&format!("    {{\n      \"name\": {name},\n"));
        let kit_mode = if mode.is_dark() { "dark" } else { "light" };
        out.push_str(&format!("      \"mode\": \"{kit_mode}\",\n"));
        for (key, value) in &settings {
            out.push_str(&format!("      {}: {value},\n", Value::from(*key)));
        }
        out.push_str("      \"colors\": {\n");
        for (position, (key, token)) in colors.iter().enumerate() {
            let token = token
                .as_str()
                .unwrap_or_else(|| panic!("tokens.json: kit.colors.{key} names a token"));
            let separator = if position + 1 < colors.len() { "," } else { "" };
            out.push_str(&format!(
                "        {}: \"{}\"{separator}\n",
                Value::from(key.as_str()),
                tokens.resolve_name(token, mode).hex()
            ));
        }
        out.push_str("      }\n");
        out.push_str(if index + 1 < Mode::ALL.len() {
            "    },\n"
        } else {
            "    }\n"
        });
    }
    out.push_str("  ]\n}\n");
    out
}

/// Compares a checked-in file with what the generator makes now, or writes it
/// when `FP_UPDATE_GOLDEN` is set.
fn check_golden(relative: &str, actual: &str) {
    let path = repo_file(relative);
    if std::env::var_os("FP_UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, actual)
            .unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    if expected == actual {
        return;
    }
    let line = expected
        .lines()
        .zip(actual.lines())
        .position(|(old, new)| old != new)
        .unwrap_or_else(|| expected.lines().count().min(actual.lines().count()));
    panic!(
        "{relative} does not match design/tokens.json (first difference at line {}). \
         Run FP_UPDATE_GOLDEN=1 cargo test golden and commit the result.",
        line + 1
    );
}

mod golden {
    use super::*;

    #[test]
    fn generated_rs_is_current() {
        check_golden("src/ui/ds/tokens/generated.rs", &rust(&Tokens::load()));
    }

    #[test]
    fn kit_theme_json_is_current() {
        check_golden(
            "assets/theme/ferrispass.theme.json",
            &kit_theme_set(&Tokens::load()),
        );
    }
}

mod tests {
    use super::*;

    #[test]
    fn natural_order_reads_numbers_as_numbers() {
        let mut names = vec![
            "space.12",
            "bg.selected.unfocused",
            "space.2",
            "bg.selected",
        ];
        names.sort_by(|a, b| natural(a, b));
        assert_eq!(
            names,
            [
                "bg.selected",
                "bg.selected.unfocused",
                "space.2",
                "space.12"
            ]
        );
    }

    /// Every name the JSON mentions outside the token groups points at a
    /// token, so a typo fails here rather than in the generated output.
    #[test]
    fn every_reference_resolves() {
        let tokens = Tokens::load();
        let json = tokens.json();
        for mode in Mode::ALL {
            for group in TOKEN_GROUPS {
                for (name, _) in tokens.group(group) {
                    tokens.resolve_name(name, mode);
                }
            }
            for (_, token) in sorted(tokens.object("css_aliases")) {
                assert!(
                    token.as_str().and_then(|t| tokens.token(t)).is_some(),
                    "{token}"
                );
            }
            for (key, token) in sorted(json["kit"]["colors"].as_object().expect("kit colors")) {
                let token = token.as_str().unwrap_or_else(|| panic!("kit {key}"));
                assert!(tokens.token(token).is_some(), "kit {key}: {token}");
            }
        }
        let contrast = &json["contrast"];
        let sets = contrast["sets"].as_object().expect("contrast sets");
        let mut named: Vec<&Value> = sets
            .values()
            .flat_map(|set| set.as_array().unwrap().iter())
            .collect();
        for pair in contrast["pairs"]
            .as_array()
            .unwrap()
            .iter()
            .chain(json["exempt"].as_array().unwrap())
        {
            named.push(&pair["fg"]);
            if let Some(also) = pair.get("also").and_then(Value::as_array) {
                named.extend(also);
            }
            match &pair["on"] {
                Value::String(set) => assert!(sets.contains_key(set), "unknown set {set}"),
                Value::Array(list) => named.extend(list),
                other => panic!("on: {other}"),
            }
        }
        for key in ["neutral_family", "hue_separation"] {
            for list in json[key].as_object().unwrap().values() {
                // Token lists; the number pairs are ranges.
                if let Some(list) = list.as_array() {
                    named.extend(list.iter().filter(|item| item.is_string()));
                }
            }
        }
        for name in named {
            let name = name
                .as_str()
                .unwrap_or_else(|| panic!("{name} is not a name"));
            assert!(tokens.token(name).is_some(), "{name} is not a token");
        }
    }
}

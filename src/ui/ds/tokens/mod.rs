//! Design tokens: the values in `design/tokens.json`, compiled into Rust.
//!
//! `generated.rs` holds every primitive colour as a constant, one
//! [`ColorSet`] per [`Mode`] and the dimensions. The golden test in
//! `codegen.rs` writes it; nobody edits it by hand. To change a value, change
//! `design/tokens.json` and run `FP_UPDATE_GOLDEN=1 cargo test golden`.
//!
//! UI code reads the semantic and component tokens through
//! [`ActiveTokens::ds`], never the primitives.

#[cfg(test)]
mod codegen;
pub mod contrast;
mod generated;

pub use generated::*;

use gpui::{App, Global, Hsla};

/// One colour: the hex value the contrast checks compute with, and the `Hsla`
/// GPUI paints with. A colour with transparency keeps its alpha in `hsla.a`;
/// `rgb` is always the opaque part.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub rgb: u32,
    pub hsla: Hsla,
}

impl Color {
    /// The alpha as a byte, 255 for an opaque colour.
    pub fn alpha(self) -> u8 {
        (self.hsla.a * 255.0).round() as u8
    }
}

impl From<Color> for Hsla {
    fn from(color: Color) -> Self {
        color.hsla
    }
}

/// One layer of a box shadow, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    pub offset_y: f32,
    pub blur: f32,
    pub spread: f32,
    pub color: Color,
    pub inset: bool,
}

/// The four colour modes. Light or dark follows the appearance setting;
/// increased contrast follows macOS alone, FerrisPass has no setting for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Mode {
    Light,
    Dark,
    LightHc,
    DarkHc,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Light, Mode::Dark, Mode::LightHc, Mode::DarkHc];

    pub fn new(dark: bool, increased_contrast: bool) -> Self {
        match (dark, increased_contrast) {
            (false, false) => Mode::Light,
            (true, false) => Mode::Dark,
            (false, true) => Mode::LightHc,
            (true, true) => Mode::DarkHc,
        }
    }

    pub fn is_dark(self) -> bool {
        matches!(self, Mode::Dark | Mode::DarkHc)
    }

    pub fn increased_contrast(self) -> bool {
        matches!(self, Mode::LightHc | Mode::DarkHc)
    }

    pub fn colors(self) -> &'static ColorSet {
        match self {
            Mode::Light => &LIGHT,
            Mode::Dark => &DARK,
            Mode::LightHc => &LIGHT_HC,
            Mode::DarkHc => &DARK_HC,
        }
    }

    /// The mode's key in `design/tokens.json`.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Light => "light",
            Mode::Dark => "dark",
            Mode::LightHc => "light-hc",
            Mode::DarkHc => "dark-hc",
        }
    }
}

/// The tokens in effect. `ds::apply` replaces this global on every change of
/// appearance, contrast or text size.
#[derive(Debug, Clone, Copy)]
pub struct DsTokens {
    pub mode: Mode,
    pub colors: &'static ColorSet,
    /// Base text size in points, 13 by default.
    pub text_size: f32,
}

impl Global for DsTokens {}

impl DsTokens {
    pub fn new(mode: Mode, text_size: f32) -> Self {
        Self {
            mode,
            colors: mode.colors(),
            text_size,
        }
    }
}

/// What [`ActiveTokens::ds`] returns before `ds::apply` first runs: light, at
/// the standard text size.
static BEFORE_APPLY: DsTokens = DsTokens {
    mode: Mode::Light,
    colors: &LIGHT,
    text_size: TEXT_SIZE_STANDARD,
};

/// `cx.ds()`: the design tokens, next to the GPUI Kit's `cx.theme()`.
pub trait ActiveTokens {
    fn ds(&self) -> &DsTokens;
}

impl ActiveTokens for App {
    fn ds(&self) -> &DsTokens {
        self.try_global::<DsTokens>().unwrap_or(&BEFORE_APPLY)
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::codegen::{Tokens, repo_file};

    /// OKLab to sRGB (Björn Ottosson's matrices), each channel rounded to a
    /// byte. Out-of-gamut values are clipped, as a browser does.
    fn oklch_to_rgb(l: f64, c: f64, h: f64) -> [u8; 3] {
        let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
        let l_ = l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
        let m_ = l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
        let s_ = l - 0.089_484_177_5 * a - 1.291_485_548_0 * b;
        let (l3, m3, s3) = (l_.powi(3), m_.powi(3), s_.powi(3));
        let linear = [
            4.076_741_662_1 * l3 - 3.307_711_591_3 * m3 + 0.230_969_929_2 * s3,
            -1.268_438_004_6 * l3 + 2.609_757_401_1 * m3 - 0.341_319_396_5 * s3,
            -0.004_196_086_3 * l3 - 0.703_418_614_7 * m3 + 1.707_614_701_0 * s3,
        ];
        linear.map(|x| {
            let encoded = if x <= 0.003_130_8 {
                12.92 * x
            } else {
                1.055 * x.powf(1.0 / 2.4) - 0.055
            };
            (encoded * 255.0).round().clamp(0.0, 255.0) as u8
        })
    }

    fn hex_channels(hex: &str) -> [u8; 3] {
        let rgb = u32::from_str_radix(&hex[1..], 16).expect("hex colour");
        [(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8]
    }

    /// OKLCH is where a new step is designed, the hex is what ships. Both
    /// have to describe the same colour to within one step per channel.
    #[test]
    fn primitives_roundtrip() {
        let tokens = Tokens::load();
        for (name, primitive) in tokens.primitives() {
            let oklch: Vec<f64> = primitive["oklch"]
                .as_array()
                .unwrap_or_else(|| panic!("{name} has no oklch"))
                .iter()
                .map(|value| value.as_f64().expect("number"))
                .collect();
            let [l, c, h] = oklch[..] else {
                panic!("{name}: oklch needs three numbers");
            };
            let hex = primitive["hex"].as_str().expect("hex");
            let expected = hex_channels(hex);
            let computed = oklch_to_rgb(l, c, h);
            for (channel, (want, got)) in expected.iter().zip(computed).enumerate() {
                assert!(
                    want.abs_diff(got) <= 1,
                    "{name}: oklch {oklch:?} gives {computed:?}, hex {hex} is {expected:?} \
                     (channel {channel})"
                );
            }
        }
    }

    /// Primitive names are `<family>.<OKLCH lightness x 1000>`, so they sort by
    /// lightness, and only the families of the palette exist.
    #[test]
    fn primitive_names() {
        const FAMILIES: [&str; 11] = [
            "sand", "kupfer", "marke", "patina", "ocker", "blau", "karmin", "petrol", "pflaume",
            "moos", "schiefer",
        ];
        let tokens = Tokens::load();
        for (name, primitive) in tokens.primitives() {
            let (family, number) = name
                .split_once('.')
                .unwrap_or_else(|| panic!("{name} is not <family>.<lightness>"));
            assert!(FAMILIES.contains(&family), "{name}: unknown family");
            let lightness = primitive["oklch"][0].as_f64().expect("lightness");
            assert_eq!(
                number.parse::<i64>().ok(),
                Some((lightness * 1000.0).round() as i64),
                "{name}: the number is not the lightness {lightness} x 1000"
            );
            let hex = primitive["hex"].as_str().expect("hex");
            assert!(
                hex.len() == 7
                    && hex.starts_with('#')
                    && u32::from_str_radix(&hex[1..], 16).is_ok(),
                "{name}: a primitive is an opaque #RRGGBB, not {hex}"
            );
        }
    }

    fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                rust_files(&path, found);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                found.push(path);
            }
        }
    }

    /// Where `needle` starts a call rather than the end of a longer name.
    fn calls(line: &str, needle: &str) -> bool {
        line.match_indices(needle).any(|(at, _)| {
            !line[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
    }

    /// A string literal that is a colour: `"#` plus six or eight hex digits.
    fn hex_literal(line: &str) -> bool {
        line.match_indices("\"#").any(|(at, _)| {
            let digits: String = line[at + 2..]
                .chars()
                .take_while(char::is_ascii_hexdigit)
                .collect();
            matches!(digits.len(), 6 | 8) && line[at + 2 + digits.len()..].starts_with('"')
        })
    }

    // Built from pieces so this file does not match its own patterns.
    const BANNED_CALLS: [&str; 3] = [
        concat!("rgb", "(0x"),
        concat!("rgba", "(0x"),
        concat!("hsla", "("),
    ];

    fn raw_color(line: &str) -> bool {
        hex_literal(line) || BANNED_CALLS.iter().any(|call| calls(line, call))
    }

    /// The scan below finds each form it is meant to find, and leaves longer
    /// names and non-colour strings alone.
    #[test]
    fn raw_color_scan_finds_every_form() {
        let quote = '"';
        for line in [
            format!("let c = {}0x112233);", BANNED_CALLS[0]),
            format!("let c = gpui::{}0x11223344);", BANNED_CALLS[1]),
            format!(".bg({}0.1, 0.2, 0.3, 1.0))", BANNED_CALLS[2]),
            format!("parse({quote}#A1B2C3{quote})"),
            format!("parse({quote}#a1b2c3d4{quote})"),
        ] {
            assert!(raw_color(&line), "missed: {line}");
        }
        for line in [
            format!("let c = my_{}0x1);", BANNED_CALLS[0]),
            format!("label({quote}#12345{quote})"),
            format!("label({quote}#A1B2C3 is a colour{quote})"),
            "let rgb = 0x112233;".to_string(),
        ] {
            assert!(!raw_color(&line), "false alarm: {line}");
        }
    }

    /// Colours in the design system come from the tokens. Only the generated
    /// file spells one out.
    #[test]
    fn no_raw_colors() {
        let generated = repo_file("src/ui/ds/tokens/generated.rs");
        let mut files = Vec::new();
        rust_files(&repo_file("src/ui/ds"), &mut files);
        rust_files(&repo_file("src/ui/windows"), &mut files);
        assert!(files.len() > 2, "the scan found no files: {files:?}");

        let mut found = Vec::new();
        for file in files.iter().filter(|file| **file != generated) {
            let text = std::fs::read_to_string(file).expect("read source");
            for (number, line) in text.lines().enumerate() {
                if raw_color(line) {
                    found.push(format!(
                        "{}:{}: {}",
                        file.display(),
                        number + 1,
                        line.trim()
                    ));
                }
            }
        }
        assert!(
            found.is_empty(),
            "colours outside generated.rs, use a token instead:\n{}",
            found.join("\n")
        );
    }
}

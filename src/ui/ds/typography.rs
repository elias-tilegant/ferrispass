//! The text styles of the design system and the two font families they use.
//!
//! The numbers are the `typography` section of `design/tokens.json`, and a
//! test holds this file to it. Sizes are points at text size 13. [`u`] turns
//! them into rems, so the larger text size scales text, heights and spacing
//! together.
//!
//! Two things here differ from what CSS would suggest:
//!
//! - Medium of the system font is [`MEDIUM`], weight 530. GPUI maps the
//!   weights of the system font onto CSS weights piecewise, and SF Medium
//!   sits near 530 on that scale: `FontWeight(500)` gives Regular, 510 to 530
//!   give Medium. In Atkinson Hyperlegible Mono, Medium is 500 as usual.
//! - Secrets are set in the bundled Atkinson Hyperlegible Mono, or in Menlo
//!   when it did not load. GPUI draws a family it cannot find in Helvetica,
//!   which is no font for telling `0` from `O` or `1` from `l`.

use std::sync::{Arc, OnceLock};

use gpui::{App, FontFeatures, FontWeight, Rems, Styled, rems};

use super::tokens::TEXT_SIZE_STANDARD;

/// The UI font. GPUI maps it to the macOS system font. Naming that font
/// directly ("SF Pro", "SF Pro Text") or as a CSS font stack ends up in
/// Helvetica.
pub const UI_FAMILY: &str = ".SystemUIFont";

/// The font for secrets, generated passwords, one-time codes and large type
/// for typing a secret by hand. The app bundles its Regular and Medium.
pub const MONO_FAMILY: &str = "Atkinson Hyperlegible Mono";

/// What [`MONO_FAMILY`] text is set in when the bundled font did not load.
/// It ships with macOS.
pub const MONO_FALLBACK: &str = "Menlo";

/// Medium of the system font. 500 would land on Regular.
pub const MEDIUM: FontWeight = FontWeight(530.0);

/// Medium of the mono font, at 500 like in CSS. 530 would ask the mono font
/// for its SemiBold, which the app does not bundle.
pub const MONO_MEDIUM: FontWeight = FontWeight(500.0);

const REGULAR: FontWeight = FontWeight::NORMAL;
const SEMIBOLD: FontWeight = FontWeight::SEMIBOLD;

/// The family a text style is set in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    /// [`UI_FAMILY`].
    Ui,
    /// [`MONO_FAMILY`], or [`MONO_FALLBACK`] when it did not load.
    Mono,
}

impl Family {
    /// The family name to draw with. For [`Family::Mono`] that is what
    /// [`mono_family`] decided at startup; before that, the fallback, which
    /// is always there.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ui => UI_FAMILY,
            Self::Mono => MONO.get().copied().unwrap_or(MONO_FALLBACK),
        }
    }
}

/// A step of the type scale. Not to be confused with `gpui::TextStyle`, the
/// resolved style GPUI lays a run of text out with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextStyle {
    /// The largest title.
    Display,
    Title1,
    /// The search field of the quick panel.
    Search,
    Title2,
    Title3,
    /// Buttons, section headers and the selected row.
    Headline,
    /// A row title while the row is not selected.
    RowTitle,
    Body,
    /// A second line, and the caption of a field.
    Callout,
    /// Labels, counters and keycaps.
    Caption,
    /// A secret, in groups of four.
    Secret,
    /// A generated password.
    Generator,
    /// A one-time code.
    Code,
    /// Large type for typing a secret by hand.
    Typing,
}

impl TextStyle {
    /// Every style, from the largest UI style down, then the mono ones.
    pub const ALL: [TextStyle; 14] = [
        Self::Display,
        Self::Title1,
        Self::Search,
        Self::Title2,
        Self::Title3,
        Self::Headline,
        Self::RowTitle,
        Self::Body,
        Self::Callout,
        Self::Caption,
        Self::Secret,
        Self::Generator,
        Self::Code,
        Self::Typing,
    ];

    /// Size, line height (both in points at text size 13), weight, family.
    const fn spec(self) -> (f32, f32, FontWeight, Family) {
        use Family::{Mono, Ui};
        match self {
            Self::Display => (26.0, 32.0, SEMIBOLD, Ui),
            Self::Title1 => (22.0, 28.0, SEMIBOLD, Ui),
            Self::Search => (20.0, 26.0, REGULAR, Ui),
            Self::Title2 => (17.0, 22.0, SEMIBOLD, Ui),
            Self::Title3 => (15.0, 20.0, SEMIBOLD, Ui),
            Self::Headline => (13.0, 16.0, SEMIBOLD, Ui),
            Self::RowTitle => (13.0, 16.0, MEDIUM, Ui),
            Self::Body => (13.0, 18.0, REGULAR, Ui),
            Self::Callout => (12.0, 16.0, REGULAR, Ui),
            Self::Caption => (11.0, 14.0, MEDIUM, Ui),
            Self::Secret => (15.0, 20.0, REGULAR, Mono),
            Self::Generator => (20.0, 26.0, REGULAR, Mono),
            Self::Code => (22.0, 28.0, MONO_MEDIUM, Mono),
            Self::Typing => (48.0, 56.0, REGULAR, Mono),
        }
    }

    /// Text size in points at text size 13.
    pub const fn size(self) -> f32 {
        self.spec().0
    }

    /// Line height in points at text size 13.
    pub const fn line_height(self) -> f32 {
        self.spec().1
    }

    pub const fn weight(self) -> FontWeight {
        self.spec().2
    }

    pub const fn family(self) -> Family {
        self.spec().3
    }

    /// Whether the style sets digits of equal width. A number that changes
    /// in another style, such as a counter in [`TextStyle::Caption`], adds
    /// [`tabular_figures`] itself.
    pub const fn tabular(self) -> bool {
        matches!(self, Self::Code)
    }
}

/// `points` at text size 13, in rems, so it grows with the text size.
pub fn u(points: f32) -> Rems {
    rems(points / TEXT_SIZE_STANDARD)
}

/// Digits of equal width (`tnum`), for numbers that change while they are
/// shown: counters, countdowns, one-time codes.
pub fn tabular_figures() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}

/// Sets a [`TextStyle`] on an element: family, size, line height, weight,
/// and tabular figures where the style has them.
pub trait TextStyled: Styled {
    fn text_style(self, style: TextStyle) -> Self {
        let styled = self
            .font_family(style.family().name())
            .text_size(u(style.size()))
            .line_height(u(style.line_height()))
            .font_weight(style.weight());
        if style.tabular() {
            styled.font_features(tabular_figures())
        } else {
            styled
        }
    }
}

impl<E: Styled> TextStyled for E {}

/// The mono family, once [`mono_family`] has decided it. Fonts belong to the
/// process, so this does too.
static MONO: OnceLock<&'static str> = OnceLock::new();

/// The family secrets are set in: [`MONO_FAMILY`] when the text system has
/// it, [`MONO_FALLBACK`] when it does not. The first call decides, for the
/// rest of the process, so `ds::init` calls it right after the app added its
/// fonts.
pub fn mono_family(cx: &App) -> &'static str {
    MONO.get_or_init(|| check_mono_family(cx))
}

/// Looks for the bundled mono font among the families the text system
/// knows, and says so on stderr when it falls back.
fn check_mono_family(cx: &App) -> &'static str {
    let family = pick_mono_family(&cx.text_system().all_font_names());
    if family != MONO_FAMILY {
        eprintln!("ferrispass: the bundled font {MONO_FAMILY} did not load, using {family}");
    }
    family
}

fn pick_mono_family(installed: &[String]) -> &'static str {
    if installed.iter().any(|name| name == MONO_FAMILY) {
        MONO_FAMILY
    } else {
        MONO_FALLBACK
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    use gpui::{AbsoluteLength, DefiniteLength, TestAppContext, div};
    use serde_json::Value;

    use super::*;

    fn repo_file(relative: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
    }

    fn tokens() -> Value {
        let path = repo_file("design/tokens.json");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
    }

    /// The name of a style under `typography.styles` in `tokens.json`.
    fn token_name(style: TextStyle) -> &'static str {
        match style {
            TextStyle::Display => "display",
            TextStyle::Title1 => "title1",
            TextStyle::Search => "search",
            TextStyle::Title2 => "title2",
            TextStyle::Title3 => "title3",
            TextStyle::Headline => "headline",
            TextStyle::RowTitle => "row-title",
            TextStyle::Body => "body",
            TextStyle::Callout => "callout",
            TextStyle::Caption => "caption",
            TextStyle::Secret => "secret",
            TextStyle::Generator => "generator",
            TextStyle::Code => "code",
            TextStyle::Typing => "typing",
        }
    }

    #[test]
    fn typography_matches_tokens() {
        let json = tokens();
        let typography = &json["typography"];
        assert_eq!(
            typography["size.standard"].as_f64(),
            Some(f64::from(TEXT_SIZE_STANDARD))
        );
        assert_eq!(typography["families"]["ui"], UI_FAMILY);
        assert_eq!(typography["families"]["mono"], MONO_FAMILY);
        let kit = &json["kit"]["settings"];
        assert_eq!(kit["font.family"], UI_FAMILY, "the Kit theme set's UI font");
        assert_eq!(
            kit["mono_font.family"], MONO_FAMILY,
            "the Kit theme set's mono font"
        );

        let styles = typography["styles"].as_object().expect("typography.styles");
        let ours: BTreeSet<&str> = TextStyle::ALL.into_iter().map(token_name).collect();
        let theirs: BTreeSet<&str> = styles.keys().map(String::as_str).collect();
        assert_eq!(ours.len(), TextStyle::ALL.len(), "a style is listed twice");
        assert_eq!(
            ours, theirs,
            "TextStyle and tokens.json list different styles"
        );

        for style in TextStyle::ALL {
            let entry = &styles[token_name(style)];
            let number = |key: &str| {
                entry[key]
                    .as_f64()
                    .unwrap_or_else(|| panic!("{style:?}: {key} is not a number"))
            };
            assert_eq!(number("size"), f64::from(style.size()), "{style:?} size");
            assert_eq!(
                number("line"),
                f64::from(style.line_height()),
                "{style:?} line height"
            );
            assert_eq!(
                number("weight"),
                f64::from(style.weight().0),
                "{style:?} weight"
            );
            let family = match entry.get("family").and_then(Value::as_str) {
                None | Some("ui") => Family::Ui,
                Some("mono") => Family::Mono,
                Some(other) => panic!("{style:?}: unknown family {other}"),
            };
            assert_eq!(style.family(), family, "{style:?} family");
            let tnum = entry
                .get("features")
                .and_then(Value::as_array)
                .is_some_and(|features| features.iter().any(|feature| feature == "tnum"));
            assert_eq!(style.tabular(), tnum, "{style:?} tabular figures");
        }
    }

    #[test]
    fn medium_is_530_for_the_system_font_and_500_for_mono() {
        assert_eq!(MEDIUM, FontWeight(530.0));
        assert_eq!(MONO_MEDIUM, FontWeight(500.0));
        assert_eq!(TextStyle::RowTitle.weight(), MEDIUM);
        assert_eq!(TextStyle::Caption.weight(), MEDIUM);
        assert_eq!(TextStyle::Code.weight(), MONO_MEDIUM);
        // Only weights that hit a face of their own. In the system font 500
        // is Regular again; the mono font comes as Regular and Medium only.
        for style in TextStyle::ALL {
            let faces: &[f32] = match style.family() {
                Family::Ui => &[400.0, 530.0, 600.0],
                Family::Mono => &[400.0, 500.0],
            };
            assert!(
                faces.contains(&style.weight().0),
                "{style:?} asks for weight {} in {:?}",
                style.weight().0,
                style.family()
            );
        }
    }

    #[test]
    fn nothing_below_11_points() {
        for style in TextStyle::ALL {
            assert!(style.size() >= 11.0, "{style:?} is {} pt", style.size());
            assert!(
                style.line_height() >= style.size(),
                "{style:?} has a line shorter than its text"
            );
        }
        for (name, entry) in tokens()["typography"]["styles"]
            .as_object()
            .expect("typography.styles")
        {
            let size = entry["size"].as_f64().expect("size");
            assert!(size >= 11.0, "tokens.json: {name} is {size} pt");
        }
    }

    #[test]
    fn u_is_points_at_text_size_13() {
        assert_eq!(u(13.0), rems(1.0));
        assert_eq!(u(26.0), rems(2.0));
        assert_eq!(u(0.0), rems(0.0));
    }

    #[test]
    fn text_style_sets_family_size_line_weight_and_figures() {
        for style in TextStyle::ALL {
            let mut element = div().text_style(style);
            let text = Styled::text_style(&mut element).clone();
            assert_eq!(
                text.font_family.as_deref(),
                Some(style.family().name()),
                "{style:?}"
            );
            assert_eq!(
                text.font_size,
                Some(AbsoluteLength::Rems(u(style.size()))),
                "{style:?}"
            );
            assert_eq!(
                text.line_height,
                Some(DefiniteLength::Absolute(AbsoluteLength::Rems(u(
                    style.line_height()
                )))),
                "{style:?}"
            );
            assert_eq!(text.font_weight, Some(style.weight()), "{style:?}");
            assert_eq!(
                text.font_features.map(|features| features.0.to_vec()),
                style.tabular().then(|| vec![("tnum".to_string(), 1)]),
                "{style:?}"
            );
        }
    }

    /// The test platform knows no font at all, which is a text system the
    /// bundled font did not make it into.
    #[gpui::test]
    fn mono_family_falls_back_only_when_the_font_is_missing(cx: &mut TestAppContext) {
        let listed =
            |names: &[&str]| -> Vec<String> { names.iter().map(|name| name.to_string()).collect() };
        assert_eq!(
            pick_mono_family(&listed(&["Helvetica", MONO_FAMILY, "Menlo"])),
            MONO_FAMILY
        );
        assert_eq!(
            pick_mono_family(&listed(&["Helvetica", "Menlo"])),
            MONO_FALLBACK
        );
        cx.update(|cx| {
            assert!(
                !cx.text_system()
                    .all_font_names()
                    .iter()
                    .any(|name| name == MONO_FAMILY)
            );
            assert_eq!(check_mono_family(cx), MONO_FALLBACK);
        });
    }

    /// The folders with the new UI.
    const NEW_MODULES: [&str; 2] = ["src/ui/ds", "src/ui/windows"];

    fn rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                rust_files(&path, files);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }

    /// Lines in the new modules that contain one of `needles`, as
    /// `file:line: text`, leaving out the files in `skip`.
    fn lines_with(needles: &[&str], skip: &[&str]) -> Vec<String> {
        let skip: Vec<PathBuf> = skip.iter().map(|file| repo_file(file)).collect();
        let mut files = Vec::new();
        for dir in NEW_MODULES {
            rust_files(&repo_file(dir), &mut files);
        }
        assert!(files.len() > 2, "the scan found no files: {files:?}");
        files.sort();
        let mut found = Vec::new();
        for file in files.iter().filter(|file| !skip.contains(file)) {
            let text = std::fs::read_to_string(file).expect("read source");
            for (number, line) in text.lines().enumerate() {
                if needles.iter().any(|needle| line.contains(needle)) {
                    let relative = file.strip_prefix(repo_file("")).unwrap_or(file);
                    found.push(format!(
                        "{}:{}: {}",
                        relative.display(),
                        number + 1,
                        line.trim()
                    ));
                }
            }
        }
        found
    }

    #[test]
    fn no_mono_literal_in_new_modules() {
        let found = lines_with(
            &["\"JetBrains Mono\"", "\"Atkinson Hyperlegible Mono\""],
            &["src/ui/ds/typography.rs"],
        );
        assert!(
            found.is_empty(),
            "name the mono font as typography::MONO_FAMILY, or set a TextStyle:\n{}",
            found.join("\n")
        );
    }

    #[test]
    fn no_family_literal_in_new_modules() {
        let found = lines_with(&["font_family(\""], &[]);
        assert!(
            found.is_empty(),
            "font families come from typography, set a TextStyle instead:\n{}",
            found.join("\n")
        );
    }

    #[test]
    fn no_uppercase_in_new_modules() {
        // Built from pieces so this file does not match its own pattern.
        const UPPERCASE: &str = concat!("to_upper", "case()");
        let found = lines_with(&[UPPERCASE], &[]);
        assert!(
            found.is_empty(),
            "the design has no capitals, write the text as it is shown:\n{}",
            found.join("\n")
        );
    }
}

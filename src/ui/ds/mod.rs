//! The FerrisPass design system.
//!
//! Colours, dimensions and the GPUI Kit theme all come from
//! `design/tokens.json`. `design/README.md` explains how to change them.

#![forbid(unsafe_code)]

pub mod a11y_prefs;
pub mod kit_theme;
pub mod tokens;

use gpui::{App, Window};

pub use a11y_prefs::A11yPrefs;
pub use tokens::{ActiveTokens, Color, ColorSet, DsTokens, Mode};

use crate::app::settings::ThemeChoice;

/// Everything the look of the app depends on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Appearance {
    /// Light, dark, or whatever macOS shows.
    pub choice: ThemeChoice,
    /// What macOS says about contrast, motion and VoiceOver.
    pub prefs: A11yPrefs,
    /// Base text size in points. There is no setting for it yet, so it is
    /// always [`tokens::TEXT_SIZE_STANDARD`].
    pub text_size: f32,
}

impl Appearance {
    /// `choice` with the accessibility settings macOS reports right now.
    pub fn current(choice: ThemeChoice) -> Self {
        Self {
            choice,
            prefs: a11y_prefs::read(),
            text_size: tokens::TEXT_SIZE_STANDARD,
        }
    }
}

/// Startup: after the fonts are added and `gpui_component::init` ran, before
/// the first window opens, so the first frame is already right.
pub fn init(choice: ThemeChoice, cx: &mut App) {
    kit_theme::install(cx);
    apply(cx, Appearance::current(choice));
}

/// Puts `appearance` into effect: the Kit theme, the tokens behind `cx.ds()`,
/// the colors the old screens read, and reduce motion.
pub fn apply(cx: &mut App, appearance: Appearance) {
    apply_in(appearance, None, cx);
}

/// Switches to `choice` with the current accessibility settings. `System`
/// reads the appearance again, so this also handles a change in macOS while
/// the app runs. Takes the same arguments as the old `theme::apply_choice`.
pub fn apply_choice(choice: ThemeChoice, window: Option<&mut Window>, cx: &mut App) {
    apply_in(Appearance::current(choice), window, cx);
}

fn apply_in(appearance: Appearance, window: Option<&mut Window>, cx: &mut App) {
    let mode = kit_theme::switch(
        appearance.choice,
        appearance.prefs.increase_contrast,
        appearance.text_size,
        window,
        cx,
    );
    cx.set_global(DsTokens::new(mode, appearance.text_size));
    // The old screens read their colors without a context, through this.
    #[allow(deprecated)]
    crate::ui::palette::set_mode(mode);
    // gpui-base reads the system setting once at startup and never again, so
    // it is set here on every change.
    cx.set_reduce_motion(appearance.prefs.reduce_motion);
}

#[cfg(test)]
#[allow(deprecated)] // the palette shim, checked from here
mod tests {
    use std::path::{Path, PathBuf};

    use std::sync::Mutex;

    use gpui::{Hsla, TestAppContext};
    use gpui_component::Theme;

    use super::*;
    use crate::ui::palette;

    /// The palette mode is one flag for the whole process. The tests that
    /// switch it hold this, so they don't switch it under each other.
    static MODE_LOCK: Mutex<()> = Mutex::new(());

    fn lock_mode() -> std::sync::MutexGuard<'static, ()> {
        MODE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    type Accessor = fn() -> Hsla;

    const ACCESSORS: [(&str, Accessor); 22] = [
        ("bg", palette::bg),
        ("panel", palette::panel),
        ("sidebar", palette::sidebar),
        ("border", palette::border),
        ("border_strong", palette::border_strong),
        ("text", palette::text),
        ("text_muted", palette::text_muted),
        ("text_faint", palette::text_faint),
        ("blue", palette::blue),
        ("blue_hover", palette::blue_hover),
        ("blue_soft", palette::blue_soft),
        ("blue_border", palette::blue_border),
        ("orange", palette::orange),
        ("orange_deep", palette::orange_deep),
        ("orange_soft", palette::orange_soft),
        ("orange_border", palette::orange_border),
        ("green", palette::green),
        ("green_soft", palette::green_soft),
        ("green_border", palette::green_border),
        ("red", palette::red),
        ("yellow", palette::yellow),
        ("transparent_overlay", palette::transparent_overlay),
    ];

    /// The rows of the mapping table in `design/README.md`: accessor and
    /// token, both in backticks.
    fn readme_table() -> Vec<(String, String)> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("design/README.md");
        let readme = std::fs::read_to_string(path).expect("read design/README.md");
        readme
            .lines()
            .filter_map(|line| {
                let cells: Vec<&str> = line.split('|').map(str::trim).collect();
                let accessor = cells.get(1)?.strip_prefix('`')?.strip_suffix("()`")?;
                let token = cells.get(2)?.strip_prefix('`')?.strip_suffix('`')?;
                Some((accessor.to_string(), token.to_string()))
            })
            .collect()
    }

    /// In every mode, each of the 22 palette accessors returns the token the
    /// table in `design/README.md` names, and the table names all 22.
    #[test]
    fn palette_follows_the_readme_table() {
        let table = readme_table();
        let named: Vec<&str> = table
            .iter()
            .map(|(accessor, _)| accessor.as_str())
            .collect();
        let accessors: Vec<&str> = ACCESSORS.iter().map(|(name, _)| *name).collect();
        assert_eq!(named, accessors, "the table and the accessors differ");

        let _lock = lock_mode();
        for mode in Mode::ALL {
            palette::set_mode(mode);
            assert_eq!(palette::mode(), mode);
            for ((name, accessor), (_, token)) in ACCESSORS.iter().zip(&table) {
                let expected = mode
                    .colors()
                    .by_name(token)
                    .unwrap_or_else(|| panic!("{token} is not a token"));
                assert_eq!(
                    accessor(),
                    expected.hsla,
                    "{name} in {mode:?} is not {token}"
                );
            }
        }
        palette::set_mode(Mode::Light);
    }

    /// `ds::apply` sets every piece it owns: the Kit theme, `cx.ds()`, the
    /// palette mode and reduce motion, for each combination it can be given.
    #[gpui::test]
    fn apply_sets_every_piece(cx: &mut TestAppContext) {
        let _lock = lock_mode();
        cx.update(|cx| {
            gpui_component::init(cx);
            assert_eq!(cx.ds().mode, Mode::Light, "light before the first apply");
            let cases = [
                (ThemeChoice::Dark, false, true, Mode::Dark),
                (ThemeChoice::Light, true, false, Mode::LightHc),
                (ThemeChoice::Dark, true, true, Mode::DarkHc),
                (ThemeChoice::Light, false, false, Mode::Light),
            ];
            for (choice, increase_contrast, reduce_motion, expected) in cases {
                let prefs = A11yPrefs {
                    increase_contrast,
                    reduce_motion,
                    voiceover: false,
                };
                apply(
                    cx,
                    Appearance {
                        choice,
                        prefs,
                        text_size: tokens::TEXT_SIZE_STANDARD,
                    },
                );
                assert_eq!(cx.ds().mode, expected);
                assert!(std::ptr::eq(cx.ds().colors, expected.colors()));
                assert_eq!(cx.ds().text_size, 13.0);
                assert_eq!(palette::mode(), expected);
                assert_eq!(palette::bg(), expected.colors().bg_window.hsla);
                assert_eq!(cx.reduce_motion(), reduce_motion);
                assert_eq!(
                    Theme::global(cx).background,
                    expected.colors().bg_window.hsla
                );
                assert_eq!(
                    Theme::global(cx).primary,
                    expected.colors().accent_fill.hsla
                );
            }
        });
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

    /// Whether the code part of `line` (before any `//`) names `palette` as a
    /// word of its own, which is how every path to the module starts.
    fn names_palette(line: &str) -> bool {
        let code = line.split("//").next().unwrap_or_default();
        code.match_indices("palette").any(|(at, word)| {
            let is_name = |c: char| c.is_alphanumeric() || c == '_';
            !code[..at].chars().next_back().is_some_and(is_name)
                && !code[at + word.len()..].chars().next().is_some_and(is_name)
        })
    }

    /// The palette shim serves the old screens and widgets only. New code
    /// reads `cx.ds()`; the only other place that names the module is the
    /// switch in `ds::apply` (this file) and the declaration in `ui/mod.rs`.
    #[test]
    fn no_palette_outside_legacy() {
        assert!(names_palette("use crate::ui::palette;"));
        assert!(names_palette(".bg(palette::panel())"));
        assert!(names_palette("use crate::ui::{palette, icons};"));
        assert!(!names_palette("palette_index: 0,"));
        assert!(!names_palette("FAVICON_PALETTE_SIZE"));
        assert!(!names_palette("// the old palette"));

        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let allowed = |relative: &str| {
            relative.starts_with("src/ui/screens/")
                || relative.starts_with("src/ui/widgets/")
                || relative.starts_with("src/ui/legacy/")
                || [
                    "src/ui/app_shell.rs",
                    "src/ui/palette.rs",
                    "src/ui/mod.rs",
                    "src/ui/ds/mod.rs",
                ]
                .contains(&relative)
        };
        let mut files = Vec::new();
        for dir in ["src", "tests", "examples"] {
            rust_files(&root.join(dir), &mut files);
        }
        assert!(
            files.len() > 50,
            "the scan found only {} files",
            files.len()
        );
        let mut found = Vec::new();
        for file in &files {
            let relative = file
                .strip_prefix(root)
                .expect("under the repo")
                .to_string_lossy()
                .replace('\\', "/");
            if allowed(&relative) {
                continue;
            }
            let text = std::fs::read_to_string(file).expect("read source");
            for (number, line) in text.lines().enumerate() {
                if names_palette(line) {
                    found.push(format!("{relative}:{}: {}", number + 1, line.trim()));
                }
            }
        }
        assert!(
            found.is_empty(),
            "the palette shim is for the old screens only, read cx.ds() instead:\n{}",
            found.join("\n")
        );
    }
}

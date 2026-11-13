//! The FerrisPass design system.
//!
//! Colours, dimensions, text styles and the GPUI Kit theme all come from
//! `design/tokens.json`. `design/README.md` explains how to change them.

#![forbid(unsafe_code)]

pub mod a11y_prefs;
pub mod components;
pub mod icons;
pub mod kit_theme;
pub mod motion;
pub mod tokens;
pub mod typography;

use gpui::{App, Window};

pub use a11y_prefs::A11yPrefs;
pub use tokens::{ActiveTokens, Color, ColorSet, DsTokens, Mode};

use crate::app::settings::{TextSize, ThemeChoice};

/// Everything the look of the app depends on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Appearance {
    /// Light, dark, or whatever macOS shows.
    pub choice: ThemeChoice,
    /// What macOS says about contrast, motion and VoiceOver.
    pub prefs: A11yPrefs,
    /// Base text size in points, from the text size setting through
    /// [`text_size_points`].
    pub text_size: f32,
}

impl Appearance {
    /// `choice` at `text_size`, with the accessibility settings macOS reports
    /// right now.
    pub fn current(choice: ThemeChoice, text_size: TextSize) -> Self {
        Self {
            choice,
            prefs: a11y_prefs::read(),
            text_size: text_size_points(text_size),
        }
    }
}

/// The base text size in points for the setting: the two text size tokens.
pub fn text_size_points(text_size: TextSize) -> f32 {
    match text_size {
        TextSize::Standard => tokens::TEXT_SIZE_STANDARD,
        TextSize::Large => tokens::TEXT_SIZE_LARGE,
    }
}

/// Startup: after the fonts are added and `gpui_component::init` ran, before
/// the first window opens, so the first frame is already right.
pub fn init(choice: ThemeChoice, text_size: TextSize, cx: &mut App) {
    // The bundled fonts are in by now, so this is where the mono family is
    // decided: Atkinson Hyperlegible Mono if the text system has it, else
    // the fallback.
    typography::mono_family(cx);
    kit_theme::install(cx);
    apply(cx, Appearance::current(choice, text_size));
}

/// Puts `appearance` into effect: the Kit theme, the tokens behind `cx.ds()`,
/// the colors the old screens read, and reduce motion.
pub fn apply(cx: &mut App, appearance: Appearance) {
    apply_in(appearance, None, cx);
}

/// Switches to `choice` at `text_size` with the current accessibility
/// settings. `System` reads the appearance again, so this also handles a
/// change in macOS while the app runs. A new text size takes effect at once:
/// the Kit sets the rem size from it on the next frame.
pub fn apply_choice(
    choice: ThemeChoice,
    text_size: TextSize,
    window: Option<&mut Window>,
    cx: &mut App,
) {
    apply_in(Appearance::current(choice, text_size), window, cx);
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
pub(crate) mod tests {
    use std::path::Path;
    use std::sync::{Mutex, MutexGuard};

    use gpui::{IntoElement as _, TestAppContext};
    use gpui_component::Theme;

    use super::components::probe;
    use super::*;

    /// `apply` also sets the mode of the old screens' colors, which is one
    /// flag for the whole process. Tests that call `apply` or read those
    /// colors hold this, so they don't switch the flag under each other.
    pub(crate) fn lock_mode() -> MutexGuard<'static, ()> {
        static MODE_LOCK: Mutex<()> = Mutex::new(());
        MODE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// `ds::apply` sets the Kit theme, `cx.ds()` and reduce motion for each
    /// combination it can be given. That it also switches the colors of the
    /// old screens is checked next to them, in `ui::palette_tests`.
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

    /// Large text is 15 points, in the Kit theme and in the tokens alike,
    /// from the setting alone: every theme choice takes it along.
    #[gpui::test]
    fn large_text_sets_fifteen_points_in_the_kit_theme_and_the_tokens(cx: &mut TestAppContext) {
        let _lock = lock_mode();
        assert_eq!(text_size_points(TextSize::Standard), 13.0);
        assert_eq!(text_size_points(TextSize::Large), 15.0);
        cx.update(|cx| {
            gpui_component::init(cx);
            for choice in [ThemeChoice::Light, ThemeChoice::Dark, ThemeChoice::System] {
                assert_eq!(Appearance::current(choice, TextSize::Large).text_size, 15.0);
                apply_choice(choice, TextSize::Large, None, cx);
                assert_eq!(f32::from(Theme::global(cx).font_size), 15.0, "{choice:?}");
                assert_eq!(cx.ds().text_size, 15.0, "{choice:?}");
                apply_choice(choice, TextSize::Standard, None, cx);
                assert_eq!(f32::from(Theme::global(cx).font_size), 13.0, "{choice:?}");
                assert_eq!(cx.ds().text_size, 13.0, "{choice:?}");
            }
        });
    }

    /// A new text size reaches an open window on its next frame: the Kit
    /// sets the window's rem size from it, and everything laid out in
    /// rems follows. Back and forth, without opening the window again.
    #[gpui::test]
    fn switching_the_text_size_applies_without_a_restart(cx: &mut TestAppContext) {
        let _lock = lock_mode();
        let cx = probe::window(cx, |_, _| gpui::div().into_any_element());
        for (size, points) in [
            (TextSize::Large, 15.0),
            (TextSize::Standard, 13.0),
            (TextSize::Large, 15.0),
        ] {
            cx.update(|window, cx| apply_choice(ThemeChoice::Light, size, Some(window), cx));
            cx.run_until_parked();
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let rem = cx.update(|window, _| f32::from(window.rem_size()));
            assert_eq!(rem, points, "{size:?}");
            assert_eq!(cx.update(|_, cx| cx.ds().text_size), points, "{size:?}");
        }
    }

    /// The fixed text sizes the scan looks for, built from pieces so that
    /// this file does not hold them.
    const FIXED_SIZES: [&str; 2] = [concat!("px(", "13"), concat!("px(", "15")];

    /// Each line of `code` that sets one of [`FIXED_SIZES`] in pixels, as
    /// `file:line: text`. A size only counts where no digit follows, so
    /// 130 or 150 pixels are left alone.
    fn fixed_sizes_in(file: &str, code: &str) -> Vec<String> {
        code.lines()
            .enumerate()
            .filter(|(_, line)| {
                FIXED_SIZES.iter().any(|size| {
                    line.match_indices(size).any(|(at, _)| {
                        !line[at + size.len()..].starts_with(|c: char| c.is_ascii_digit())
                    })
                })
            })
            .map(|(at, line)| format!("{file}:{}: {}", at + 1, line.trim()))
            .collect()
    }

    /// The text size is a number in the tokens and the setting only. No
    /// component and no window of the new screens sets 13 or 15 pixels: a
    /// fixed size would stay put when the text size changes. Comments and
    /// inline test modules do not count; the old screens are left out.
    #[test]
    fn no_fixed_text_size_outside_tokens() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        for dir in ["src/ui/ds", "src/ui/windows"] {
            probe::rust_files(&root.join(dir), &mut files);
        }
        files.sort();
        assert!(files.len() > 10, "the scan found {} files", files.len());
        let found: Vec<String> = files
            .iter()
            .flat_map(|file| {
                let source = std::fs::read_to_string(file).expect("read source");
                let name = file
                    .strip_prefix(root)
                    .unwrap_or(file)
                    .display()
                    .to_string();
                fixed_sizes_in(&name, &probe::code_only(&source))
            })
            .collect();
        assert!(
            found.is_empty(),
            "take the size from the tokens or the text styles:\n{}",
            found.join("\n")
        );
        // The scan does see a fixed size, and leaves a larger number alone.
        let [standard, large] = FIXED_SIZES;
        let sample = format!("div().text_size({large}.)).w({standard}0.))");
        assert_eq!(fixed_sizes_in("sample.rs", &sample).len(), 1);
        assert!(fixed_sizes_in("sample.rs", &format!("w({standard}0.))")).is_empty());
    }
}

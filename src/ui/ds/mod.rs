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
/// the app runs.
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
pub(crate) mod tests {
    use std::sync::{Mutex, MutexGuard};

    use gpui::TestAppContext;
    use gpui_component::Theme;

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
    /// combination it can be given. The colors of the old screens are
    /// checked where they are defined, in `ui::palette`.
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
}

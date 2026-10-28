//! The old entry points for the theme. The design system owns the theme now
//! (`ui::ds`); these forward to it until their callers move there.

use gpui::{App, Window};

use crate::app::settings::ThemeChoice;

/// See [`crate::ui::ds::apply_choice`].
pub fn apply_choice(choice: ThemeChoice, window: Option<&mut Window>, cx: &mut App) {
    crate::ui::ds::apply_choice(choice, window, cx);
}

/// See [`crate::ui::ds::init`].
pub fn init_from_settings(choice: ThemeChoice, cx: &mut App) {
    crate::ui::ds::init(choice, cx);
}

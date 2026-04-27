use gpui::{App, SharedString, Window};
use gpui_component::theme::{Theme, ThemeMode};

use crate::ui::palette;

/// Override gpui-component's default theme with the KeePass RS design tokens.
/// Reads the current `Theme.mode` and picks the matching palette so the call
/// works for both the initial light setup and runtime light/dark switches.
pub fn apply(cx: &mut App) {
    let mode = Theme::global(cx).mode;
    let is_dark = mode.is_dark();
    palette::set_dark(is_dark);
    let p = palette::current();

    // Foreground for high-saturation surfaces (primary blue, danger red, etc.).
    // Light mode: pure white reads well over saturated colors. Dark mode: our
    // saturated colors are slightly lighter, so a near-white still works — and
    // `panel` (= near-black in dark mode) would be invisible.
    let on_accent = if is_dark { p.text } else { p.panel };

    let theme = Theme::global_mut(cx);
    theme.font_family = SharedString::from(
        "-apple-system, BlinkMacSystemFont, \"SF Pro Text\", \"Helvetica Neue\", sans-serif",
    );
    theme.mono_font_family = SharedString::from("JetBrains Mono");

    let c = &mut theme.colors;

    c.background = p.bg;
    c.foreground = p.text;
    c.border = p.border;
    c.muted_foreground = p.text_muted;
    c.muted = p.sidebar;

    c.popover = p.panel;
    c.popover_foreground = p.text;

    c.sidebar = p.sidebar;
    c.sidebar_foreground = p.text;
    c.sidebar_border = p.border;
    c.sidebar_accent = p.blue;
    c.sidebar_accent_foreground = on_accent;
    c.sidebar_primary = p.blue;
    c.sidebar_primary_foreground = on_accent;

    c.primary = p.blue;
    c.primary_hover = p.blue_hover;
    c.primary_active = p.blue_hover;
    c.primary_foreground = on_accent;

    c.button_primary = p.blue;
    c.button_primary_hover = p.blue_hover;
    c.button_primary_active = p.blue_hover;
    c.button_primary_foreground = on_accent;

    c.accent = p.blue_soft;
    c.accent_foreground = p.text;

    c.secondary = p.sidebar;
    c.secondary_foreground = p.text;
    c.secondary_hover = p.border;
    c.secondary_active = p.border_strong;

    c.danger = p.red;
    c.danger_foreground = on_accent;
    c.success = p.green;
    c.success_foreground = on_accent;
    c.info = p.blue;
    c.info_foreground = on_accent;
    c.warning = p.yellow;
    c.warning_foreground = on_accent;

    c.input = p.border_strong;
    c.ring = p.blue_border;
    c.caret = p.blue;
    c.selection = p.blue_soft;

    c.list = p.panel;
    c.list_hover = p.sidebar;
    c.list_active = p.blue_soft;
    c.list_active_border = p.blue_border;

    c.switch = p.border_strong;
    c.switch_thumb = p.panel;

    c.title_bar = p.panel;
    c.title_bar_border = p.border;

    c.red = p.red;
    c.green = p.green;
    c.blue = p.blue;
    c.yellow = p.yellow;

    theme.radius = gpui::px(6.0);
    theme.radius_lg = gpui::px(10.0);
}

/// Switch to the opposite mode and re-apply our palette overrides. Triggers a
/// window refresh so existing renders pick up the new colors immediately.
pub fn toggle(window: &mut Window, cx: &mut App) {
    let next = if Theme::global(cx).mode.is_dark() {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    };
    Theme::change(next, Some(window), cx);
    apply(cx);
}

/// Initial setup: read the OS appearance once, then apply our palette so the
/// startup paint matches the system mode without flashing the wrong theme.
pub fn init_from_system(cx: &mut App) {
    Theme::sync_system_appearance(None, cx);
    apply(cx);
}

use gpui::{App, SharedString};
use gpui_component::theme::Theme;

use crate::ui::palette;

/// Override gpui-component's default light theme with the KeePass RS design tokens.
/// Call after `gpui_component::init(cx)`.
pub fn apply(cx: &mut App) {
    let theme = Theme::global_mut(cx);

    theme.font_family = SharedString::from(
        "-apple-system, BlinkMacSystemFont, \"SF Pro Text\", \"Helvetica Neue\", sans-serif",
    );
    theme.mono_font_family = SharedString::from("JetBrains Mono");

    let c = &mut theme.colors;

    c.background = palette::BG;
    c.foreground = palette::TEXT;
    c.border = palette::BORDER;
    c.muted_foreground = palette::TEXT_MUTED;
    c.muted = palette::SIDEBAR;

    c.popover = palette::PANEL;
    c.popover_foreground = palette::TEXT;

    c.sidebar = palette::SIDEBAR;
    c.sidebar_foreground = palette::TEXT;
    c.sidebar_border = palette::BORDER;
    c.sidebar_accent = palette::BLUE;
    c.sidebar_accent_foreground = palette::PANEL;
    c.sidebar_primary = palette::BLUE;
    c.sidebar_primary_foreground = palette::PANEL;

    c.primary = palette::BLUE;
    c.primary_hover = palette::BLUE_HOVER;
    c.primary_active = palette::BLUE_HOVER;
    c.primary_foreground = palette::PANEL;

    c.button_primary = palette::BLUE;
    c.button_primary_hover = palette::BLUE_HOVER;
    c.button_primary_active = palette::BLUE_HOVER;
    c.button_primary_foreground = palette::PANEL;

    c.accent = palette::BLUE_SOFT;
    c.accent_foreground = palette::TEXT;

    c.secondary = palette::SIDEBAR;
    c.secondary_foreground = palette::TEXT;
    c.secondary_hover = palette::BORDER;
    c.secondary_active = palette::BORDER_STRONG;

    c.danger = palette::RED;
    c.danger_foreground = palette::PANEL;
    c.success = palette::GREEN;
    c.success_foreground = palette::PANEL;
    c.info = palette::BLUE;
    c.info_foreground = palette::PANEL;
    c.warning = palette::YELLOW;
    c.warning_foreground = palette::PANEL;

    c.input = palette::BORDER_STRONG;
    c.ring = palette::BLUE_BORDER;
    c.caret = palette::BLUE;
    c.selection = palette::BLUE_SOFT;

    c.list = palette::PANEL;
    c.list_hover = palette::SIDEBAR;
    c.list_active = palette::BLUE_SOFT;
    c.list_active_border = palette::BLUE_BORDER;

    c.switch = palette::BORDER_STRONG;
    c.switch_thumb = palette::PANEL;

    c.title_bar = palette::PANEL;
    c.title_bar_border = palette::BORDER;

    c.red = palette::RED;
    c.green = palette::GREEN;
    c.blue = palette::BLUE;
    c.yellow = palette::YELLOW;

    theme.radius = gpui::px(6.0);
    theme.radius_lg = gpui::px(10.0);
}

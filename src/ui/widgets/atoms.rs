use gpui::{
    AnyElement, App, Hsla, IntoElement as _, ParentElement as _, Styled as _, div, px,
};
use gpui_component::{ActiveTheme as _, h_flex};

use crate::ui::palette;

/// Small uppercase muted label used above form fields and in section headers.
pub fn label(text: impl Into<String>) -> AnyElement {
    div()
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(palette::TEXT_MUTED)
        .child(text.into())
        .into_any_element()
}

/// Section heading used inside sidebar / settings panels.
pub fn section_heading(text: impl Into<String>) -> AnyElement {
    div()
        .text_xs()
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(palette::TEXT_FAINT)
        .child(text.into())
        .into_any_element()
}

#[derive(Clone, Copy)]
pub enum ChipTone {
    Blue,
    Orange,
    Green,
    Gray,
}

impl ChipTone {
    fn colors(self) -> (Hsla, Hsla, Hsla) {
        match self {
            ChipTone::Blue => (palette::BLUE_SOFT, palette::BLUE, palette::BLUE_BORDER),
            ChipTone::Orange => (palette::ORANGE_SOFT, palette::ORANGE_DEEP, palette::ORANGE_BORDER),
            ChipTone::Green => (palette::GREEN_SOFT, palette::GREEN, palette::GREEN_BORDER),
            ChipTone::Gray => (palette::SIDEBAR, palette::TEXT_MUTED, palette::BORDER),
        }
    }
}

/// Small pill chip ("Personal", "Work", "2FA", "Connected", …).
pub fn chip(text: impl Into<String>, tone: ChipTone) -> AnyElement {
    let (bg, fg, bd) = tone.colors();
    div()
        .h(px(18.))
        .px(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .bg(bg)
        .text_color(fg)
        .border_1()
        .border_color(bd)
        .child(text.into())
        .into_any_element()
}

/// Round status dot.
pub fn dot(color: Hsla, size: f32) -> AnyElement {
    div()
        .size(px(size))
        .rounded_full()
        .bg(color)
        .into_any_element()
}

/// Vertical 1px divider with theme border color.
pub fn vrule(cx: &App) -> AnyElement {
    div()
        .w(px(1.))
        .h(px(18.))
        .bg(cx.theme().border)
        .into_any_element()
}

/// Status badge in title row (e.g. "Synced", "Locked", "Open").
pub fn status_badge(text: impl Into<String>, tone: ChipTone) -> AnyElement {
    let (bg, fg, bd) = tone.colors();
    h_flex()
        .gap_1()
        .h(px(20.))
        .px(px(8.))
        .rounded_full()
        .bg(bg)
        .border_1()
        .border_color(bd)
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(fg)
        .child(text.into())
        .into_any_element()
}

/// Right-aligned tabular number text.
pub fn count(value: usize, color: Hsla) -> AnyElement {
    div()
        .text_xs()
        .text_color(color)
        .child(value.to_string())
        .into_any_element()
}

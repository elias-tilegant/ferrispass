use gpui::{AnyElement, Hsla, IntoElement as _, ParentElement as _, Styled as _, div, px};

use crate::ui::palette;

/// `"1 entry"` / `"4 entries"`. Three call sites open-coded this and two more
/// printed "1 entries", so the count strings disagreed across the app.
pub fn plural(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("{count} {singular}")
    } else {
        format!("{count} {plural}")
    }
}

/// Small uppercase muted label used above form fields and in section headers.
pub fn label(text: impl Into<String>) -> AnyElement {
    div()
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(palette::text_muted())
        .child(text.into())
        .into_any_element()
}

/// Section heading used inside sidebar / settings panels.
pub fn section_heading(text: impl Into<String>) -> AnyElement {
    div()
        .text_xs()
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(palette::text_faint())
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
            ChipTone::Blue => (
                palette::blue_soft(),
                palette::blue(),
                palette::blue_border(),
            ),
            ChipTone::Orange => (
                palette::orange_soft(),
                palette::orange_deep(),
                palette::orange_border(),
            ),
            ChipTone::Green => (
                palette::green_soft(),
                palette::green(),
                palette::green_border(),
            ),
            ChipTone::Gray => (palette::sidebar(), palette::text_muted(), palette::border()),
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

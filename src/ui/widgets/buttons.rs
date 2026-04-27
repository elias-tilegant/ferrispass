use gpui::{
    AnyElement, ElementId, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, div, px,
};
use gpui_component::{ActiveTheme as _, h_flex};

use crate::ui::palette;

/// Orange "accent" CTA — used sparingly (the design uses it primarily on Generate-style
/// affordances). Behaves like a button but is rendered as a styled flex row so we can
/// match the design's gradient + shadow exactly.
pub fn accent_button(id: impl Into<ElementId>, label: impl Into<String>) -> AnyElement {
    let id: ElementId = id.into();
    let label = label.into();
    div()
        .id(id)
        .h(px(30.))
        .px(px(12.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .gap_1p5()
        .bg(palette::orange())
        .text_color(palette::panel())
        .border_1()
        .border_color(palette::orange_deep())
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .child(label)
        .into_any_element()
}

/// Pure visual "ghost" button — flat, matches design's `kind: 'ghost'`.
pub fn ghost_pill(id: impl Into<ElementId>, label: impl Into<String>) -> AnyElement {
    let id: ElementId = id.into();
    let label = label.into();
    div()
        .id(id)
        .h(px(28.))
        .px(px(10.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .child(label)
        .into_any_element()
}

/// Step indicator like "1 — 2 — 3" used on the connect screen.
pub fn step_indicator<'a>(steps: &'a [(usize, &'a str)], active: usize, cx: &gpui::App) -> AnyElement {
    let theme_border = cx.theme().border;
    let mut row = h_flex().items_center().gap_3().w_full();

    let total = steps.len();
    for (i, (number, label)) in steps.iter().enumerate() {
        let is_active = *number == active;
        let bullet_bg = if is_active {
            palette::blue()
        } else {
            palette::border_strong()
        };
        let label_color = if is_active {
            palette::blue()
        } else {
            palette::text_muted()
        };

        row = row.child(
            h_flex()
                .gap_1p5()
                .items_center()
                .child(
                    div()
                        .size(px(16.))
                        .rounded_full()
                        .bg(bullet_bg)
                        .text_color(palette::panel())
                        .text_xs()
                        .font_weight(gpui::FontWeight::BOLD)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(number.to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(label_color)
                        .child((*label).to_string()),
                ),
        );
        if i < total - 1 {
            row = row.child(div().h(px(1.)).flex_1().bg(theme_border));
        }
    }
    row.into_any_element()
}

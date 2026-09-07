use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, div, px};
use gpui_component::{ActiveTheme as _, h_flex};

use crate::ui::palette;

/// Step indicator like "1 - 2 - 3" used on the connect screen.
pub fn step_indicator<'a>(
    steps: &'a [(usize, &'a str)],
    active: usize,
    cx: &gpui::App,
) -> AnyElement {
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

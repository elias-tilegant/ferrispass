use gpui::{
    AnyElement, ElementId, Hsla, InteractiveElement as _, IntoElement as _, ParentElement as _,
    Styled as _, div, px,
};
use gpui_component::{h_flex, v_flex};

use crate::ui::palette;

pub struct Provider {
    pub id: ElementId,
    pub name: &'static str,
    pub meta: &'static str,
    pub letter: &'static str,
    pub color: Hsla,
    pub selected: bool,
}

pub fn provider_row(provider: Provider) -> AnyElement {
    let Provider {
        id,
        name,
        meta,
        letter,
        color,
        selected,
    } = provider;
    let bg = if selected { palette::blue_soft() } else { palette::panel() };
    let border = if selected { palette::blue() } else { palette::border() };

    div()
        .id(id)
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .p_3()
                .rounded(px(8.))
                .bg(bg)
                .border_1()
                .border_color(border)
                .child(
                    div()
                        .size(px(36.))
                        .rounded(px(7.))
                        .bg(color)
                        .text_color(palette::panel())
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_lg()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(letter),
                )
                .child(
                    v_flex()
                        .gap_0p5()
                        .flex_1()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(palette::text())
                                .child(name),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette::text_muted())
                                .child(meta),
                        ),
                )
                .child(
                    div()
                        .size(px(18.))
                        .rounded_full()
                        .border_1()
                        .border_color(if selected { palette::blue() } else { palette::border_strong() })
                        .bg(if selected { palette::blue() } else { palette::panel() })
                        .text_color(palette::panel())
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_xs()
                        .child(if selected { "✓" } else { "" }),
                ),
        )
        .into_any_element()
}

use gpui::{
    AnyElement, IntoElement as _, ParentElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{h_flex, v_flex};

use crate::ui::palette;

pub fn toggle_row(label: &'static str, detail: &'static str, on: bool, last: bool) -> AnyElement {
    h_flex()
        .gap_3()
        .items_center()
        .p_3p5()
        .when(!last, |this| {
            this.border_b_1().border_color(palette::border())
        })
        .child(
            v_flex()
                .gap_0p5()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(palette::text())
                        .child(label),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette::text_muted())
                        .child(detail),
                ),
        )
        .child(switch_visual(on))
        .into_any_element()
}

pub(crate) fn switch_visual(on: bool) -> AnyElement {
    div()
        .relative()
        .w(px(32.))
        .h(px(18.))
        .rounded_full()
        .bg(if on {
            palette::blue()
        } else {
            palette::border_strong()
        })
        .child(
            div()
                .absolute()
                .top(px(2.))
                .left(px(if on { 16. } else { 2. }))
                .size(px(14.))
                .rounded_full()
                .bg(palette::panel()),
        )
        .into_any_element()
}

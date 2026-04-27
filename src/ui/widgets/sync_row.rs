use gpui::{
    AnyElement, Hsla, IntoElement as _, ParentElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{h_flex, v_flex};

use crate::ui::palette;

pub enum SyncOutcome {
    Success,
    Merge,
    Error,
}

impl SyncOutcome {
    fn colors(&self) -> (Hsla, Hsla, &'static str) {
        match self {
            SyncOutcome::Success => (palette::green_soft(), palette::green(), "✓"),
            SyncOutcome::Merge => (palette::orange_soft(), palette::orange(), "↻"),
            SyncOutcome::Error => (palette::sidebar(), palette::red(), "!"),
        }
    }
}

pub fn sync_row(
    outcome: SyncOutcome,
    title: &'static str,
    detail: &'static str,
    time: &'static str,
    last: bool,
) -> AnyElement {
    let (bg, fg, glyph) = outcome.colors();
    h_flex()
        .gap_3()
        .items_center()
        .p_3()
        .when(!last, |this| {
            this.border_b_1().border_color(palette::border())
        })
        .child(
            div()
                .size(px(26.))
                .rounded(px(6.))
                .bg(bg)
                .text_color(fg)
                .font_weight(gpui::FontWeight::BOLD)
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .child(glyph),
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
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette::text_muted())
                        .child(detail),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette::text_faint())
                .font_family("JetBrains Mono")
                .child(time),
        )
        .into_any_element()
}

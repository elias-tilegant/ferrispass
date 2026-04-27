use gpui::{
    AnyElement, IntoElement as _, ParentElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{h_flex, v_flex};

use crate::domain::Strength;
use crate::ui::palette;
use crate::ui::widgets::atoms::label;

/// Render the entry-detail "Password health" card.
pub fn strength_card(strength: Strength, length: usize) -> AnyElement {
    let (color, soft, label_text) = match strength {
        Strength::Weak => (palette::RED, palette::SIDEBAR, "Weak"),
        Strength::Fair => (palette::YELLOW, palette::SIDEBAR, "Fair"),
        Strength::Strong => (palette::GREEN, palette::SIDEBAR, "Strong"),
    };

    let segments = strength.fill_segments(10);
    let bits_estimate = (length as f32 * 6.5) as usize;

    v_flex()
        .gap_2()
        .p_2p5()
        .rounded(px(6.))
        .bg(palette::PANEL)
        .border_1()
        .border_color(palette::BORDER)
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(color)
                        .child(label_text),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette::TEXT_FAINT)
                        .child(format!("· {length} chars · {bits_estimate} bits")),
                ),
        )
        .child({
            let mut row = h_flex().gap(px(3.));
            for i in 0..10 {
                let filled = i < segments;
                row = row.child(
                    div()
                        .h(px(4.))
                        .w(px(20.))
                        .rounded(px(1.))
                        .bg(if filled { color } else { soft }),
                );
            }
            row
        })
        .into_any_element()
}

/// Generator card from the AddEntry modal: length slider + character class chips.
pub fn generator_card(length: usize, strength_label: &'static str, bits: usize) -> AnyElement {
    let position = ((length as f32) / 32.0).clamp(0.0, 1.0);
    v_flex()
        .gap_3()
        .p_3()
        .rounded(px(6.))
        .bg(palette::SIDEBAR)
        .border_1()
        .border_color(palette::BORDER)
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .text_xs()
                .text_color(palette::TEXT_MUTED)
                .child(
                    h_flex()
                        .gap_1()
                        .child("Length:")
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(palette::TEXT)
                                .child(length.to_string()),
                        ),
                )
                .child(
                    div()
                        .text_color(palette::GREEN)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(format!("{strength_label} · {bits} bits")),
                ),
        )
        .child(
            div()
                .relative()
                .h(px(4.))
                .w_full()
                .rounded(px(2.))
                .bg(palette::PANEL)
                .border_1()
                .border_color(palette::BORDER)
                .child(
                    div()
                        .h_full()
                        .w(gpui::relative(position))
                        .bg(palette::BLUE)
                        .rounded(px(2.)),
                ),
        )
        .child({
            let classes = ["A-Z", "a-z", "0-9", "!@#"];
            let mut row = h_flex().gap_3p5();
            for class in classes {
                row = row.child(
                    h_flex()
                        .items_center()
                        .gap_1p5()
                        .child(
                            div()
                                .size(px(13.))
                                .rounded(px(3.))
                                .bg(palette::BLUE)
                                .border_1()
                                .border_color(palette::BLUE)
                                .text_color(palette::PANEL)
                                .text_xs()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child("✓"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette::TEXT)
                                .child(class),
                        ),
                );
            }
            row
        })
        .into_any_element()
}

/// Used in the welcome footer: "AES-256 · Argon2id" line.
pub fn footer_chip(text: impl Into<String>, _cx: &gpui::App) -> AnyElement {
    div()
        .text_xs()
        .text_color(palette::TEXT_FAINT)
        .child(text.into())
        .into_any_element()
}

/// Form field row with a label above an input-like display value.
pub fn detail_row(
    label_text: &'static str,
    value: impl Into<String>,
    mono: bool,
    masked: bool,
) -> AnyElement {
    let value = value.into();
    let display = if masked { "••••••••••••••••".to_string() } else { value };

    v_flex()
        .gap_1()
        .child(label(label_text))
        .child(
            div()
                .min_h(px(34.))
                .w_full()
                .rounded(px(6.))
                .border_1()
                .border_color(palette::BORDER)
                .bg(palette::SIDEBAR)
                .px_3()
                .py_2()
                .text_sm()
                .when(mono, |this| this.font_family("JetBrains Mono"))
                .child(display),
        )
        .into_any_element()
}


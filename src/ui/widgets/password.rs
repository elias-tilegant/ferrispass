use gpui::{
    AnyElement, IntoElement as _, ParentElement as _, Styled as _, div, px,
};
use gpui_component::{h_flex, v_flex};

use crate::domain::Strength;
use crate::ui::palette;

/// Render the entry-detail "Password health" card.
///
/// `bits` is the real `log2(zxcvbn.guesses())` when available; the legacy callers
/// that only know length pass `None` and we fall back to a rough length-based
/// estimate so the card never lies about being "0 bits" for unknown passwords.
pub fn strength_card(strength: Strength, length: usize, bits: Option<u32>) -> AnyElement {
    let (color, soft, label_text) = match strength {
        Strength::Weak => (palette::red(), palette::sidebar(), "Weak"),
        Strength::Fair => (palette::yellow(), palette::sidebar(), "Fair"),
        Strength::Strong => (palette::green(), palette::sidebar(), "Strong"),
    };

    let segments = strength.fill_segments(10);
    let bits_display = bits
        .map(|b| b as usize)
        .unwrap_or_else(|| (length as f32 * 6.5) as usize);

    v_flex()
        .gap_2()
        .p_2p5()
        .rounded(px(6.))
        .bg(palette::panel())
        .border_1()
        .border_color(palette::border())
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
                        .text_color(palette::text_faint())
                        .child(format!("· {length} chars · {bits_display} bits")),
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

/// Used in the welcome footer: "AES-256 · Argon2id" line.
pub fn footer_chip(text: impl Into<String>, _cx: &gpui::App) -> AnyElement {
    div()
        .text_xs()
        .text_color(palette::text_faint())
        .child(text.into())
        .into_any_element()
}


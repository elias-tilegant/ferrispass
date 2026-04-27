use gpui::{AnyElement, Hsla, IntoElement as _, ParentElement as _, Styled as _, div, px};

use crate::ui::palette;

/// Color palette used by the synthesized entry favicons.
const FAVICON_COLORS: &[(u8, u8, u8)] = &[
    (0x42, 0x85, 0xf4), // Google blue
    (0x1c, 0x1f, 0x24), // GitHub black
    (0x18, 0x77, 0xf2), // Figma blue
    (0xff, 0x90, 0x00), // AWS orange
    (0x58, 0x65, 0xf2), // Discord violet
    (0x16, 0xa3, 0x4a), // Notion-ish green
    (0x0a, 0x66, 0xc2), // LinkedIn blue
    (0x00, 0x66, 0xff), // Vercel blue
    (0xf3, 0x80, 0x20), // Cloudflare orange
    (0xa3, 0x55, 0xf7), // purple
    (0xe5, 0x39, 0x35), // red
    (0x0f, 0x76, 0x6e), // teal
];

fn rgb_to_hsla(r: u8, g: u8, b: u8) -> Hsla {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) * 0.5;
    let delta = max - min;
    let s = if delta == 0.0 {
        0.0
    } else if l <= 0.5 {
        delta / (max + min)
    } else {
        delta / (2.0 - max - min)
    };
    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        let segment = (g - b) / delta;
        let shift = if segment < 0.0 { 6.0 } else { 0.0 };
        (segment + shift) / 6.0
    } else if max == g {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };
    gpui::hsla(h, s, l, 1.0)
}

pub fn favicon_color(palette_index: u8) -> Hsla {
    let (r, g, b) = FAVICON_COLORS[(palette_index as usize) % FAVICON_COLORS.len()];
    rgb_to_hsla(r, g, b)
}

/// Square colored favicon with a single letter, like the design's `<Fav letter="G" />`.
pub fn favicon(letter: &str, palette_index: u8, size: f32) -> AnyElement {
    let bg = favicon_color(palette_index);
    div()
        .size(px(size))
        .rounded(px((size / 4.5).max(6.0)))
        .bg(bg)
        .text_color(palette::panel())
        .font_weight(gpui::FontWeight::BOLD)
        .text_sm()
        .flex()
        .items_center()
        .justify_center()
        .child(letter.to_string())
        .into_any_element()
}

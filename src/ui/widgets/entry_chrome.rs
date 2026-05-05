use gpui::{
    AnyElement, Hsla, InteractiveElement as _, IntoElement as _, ObjectFit, ParentElement as _,
    Styled as _, StyledImage as _, div, img, px,
};

use crate::domain::Favicon;
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

/// Square favicon. Renders the entry's custom-icon image (extracted from
/// the KeePass `custom_icons` table) when present, otherwise falls back to
/// the synthesized colored letter. If the cached image bytes turn out to be
/// undecodable (corrupt blob, format we sniffed wrong), GPUI's image loader
/// hits `with_fallback` and we render the letter view there too.
pub fn favicon(fav: &Favicon, size: f32) -> AnyElement {
    let radius = px((size / 4.5).max(6.0));
    if let Some(image) = fav.image.as_ref() {
        // Snapshot the letter bits up-front so the fallback closure (which
        // must be `'static`) doesn't need to capture `&Favicon`.
        let letter = fav.letter.clone();
        let palette_index = fav.palette_index;
        return div()
            .id("favicon-img")
            .size(px(size))
            .rounded(radius)
            .overflow_hidden()
            .bg(palette::panel())
            .child(
                img(image.0.clone())
                    .object_fit(ObjectFit::Cover)
                    .size(px(size))
                    .with_fallback(move || {
                        letter_favicon(&letter, palette_index, size).into_any_element()
                    }),
            )
            .into_any_element();
    }
    letter_favicon(&fav.letter, fav.palette_index, size)
}

fn letter_favicon(letter: &str, palette_index: u8, size: f32) -> AnyElement {
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

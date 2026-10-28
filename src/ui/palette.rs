//! The colors the old screens and widgets read, now taken from the design
//! tokens.
//!
//! The old screens ask for a color without a context (`palette::panel()`),
//! so the current mode sits in an atomic that `ui::ds::apply` sets on every
//! change of appearance or contrast. Each accessor returns the token the
//! table in `design/README.md` maps it to. New code reads `cx.ds()` instead;
//! this module goes away with the old screens.

#![deprecated(note = "use ui::ds")]

use std::sync::atomic::{AtomicU8, Ordering};

use gpui::Hsla;

use crate::ui::ds::{ColorSet, Mode};

/// The mode `ui::ds::apply` set last, as `Mode as u8`. Light until then.
static MODE: AtomicU8 = AtomicU8::new(Mode::Light as u8);

/// Switches every accessor to `mode`. Only `ui::ds::apply` calls this.
pub fn set_mode(mode: Mode) {
    MODE.store(mode as u8, Ordering::Relaxed);
}

/// The mode the accessors read.
pub fn mode() -> Mode {
    let stored = MODE.load(Ordering::Relaxed);
    Mode::ALL
        .into_iter()
        .find(|mode| *mode as u8 == stored)
        .unwrap_or(Mode::Light)
}

#[inline]
fn colors() -> &'static ColorSet {
    mode().colors()
}

// The accessors, in the order of the table in design/README.md.
#[inline]
pub fn bg() -> Hsla {
    colors().bg_window.hsla
}
#[inline]
pub fn panel() -> Hsla {
    colors().bg_surface.hsla
}
#[inline]
pub fn sidebar() -> Hsla {
    colors().bg_sidebar.hsla
}
#[inline]
pub fn border() -> Hsla {
    colors().border_separator.hsla
}
#[inline]
pub fn border_strong() -> Hsla {
    colors().border_control.hsla
}
#[inline]
pub fn text() -> Hsla {
    colors().text_primary.hsla
}
#[inline]
pub fn text_muted() -> Hsla {
    colors().text_secondary.hsla
}
#[inline]
pub fn text_faint() -> Hsla {
    colors().text_tertiary.hsla
}
#[inline]
pub fn blue() -> Hsla {
    colors().accent_fill.hsla
}
#[inline]
pub fn blue_hover() -> Hsla {
    colors().accent_fill_hover.hsla
}
#[inline]
pub fn blue_soft() -> Hsla {
    colors().bg_selected.hsla
}
#[inline]
pub fn blue_border() -> Hsla {
    colors().focus_ring.hsla
}
#[inline]
pub fn orange() -> Hsla {
    colors().accent_text.hsla
}
#[inline]
pub fn orange_deep() -> Hsla {
    colors().accent_fill_hover.hsla
}
#[inline]
pub fn orange_soft() -> Hsla {
    colors().bg_selected.hsla
}
#[inline]
pub fn orange_border() -> Hsla {
    colors().focus_ring.hsla
}
#[inline]
pub fn green() -> Hsla {
    colors().status_ok_text.hsla
}
#[inline]
pub fn green_soft() -> Hsla {
    colors().status_ok_soft.hsla
}
#[inline]
pub fn green_border() -> Hsla {
    colors().status_ok_text.hsla
}
#[inline]
pub fn red() -> Hsla {
    colors().status_danger_text.hsla
}
#[inline]
pub fn yellow() -> Hsla {
    colors().status_attention_text.hsla
}
#[inline]
pub fn transparent_overlay() -> Hsla {
    colors().scrim.hsla
}

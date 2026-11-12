//! The quick access panel. The global shortcut ⌃⌥⌘P opens and closes it
//! through [`toggle`].

use gpui::{App, Global};

/// How often the panel was asked to open or close.
#[derive(Default)]
struct Toggles(usize);

impl Global for Toggles {}

/// Opens the panel when it is closed and closes it when it is open.
pub fn toggle(cx: &mut App) {
    cx.default_global::<Toggles>().0 += 1;
}

/// How often [`toggle`] ran.
pub fn toggles(cx: &App) -> usize {
    cx.try_global::<Toggles>().map_or(0, |toggles| toggles.0)
}

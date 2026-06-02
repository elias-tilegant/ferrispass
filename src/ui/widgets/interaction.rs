//! Shared interaction-feedback helpers — one consistent hover / pressed /
//! cursor language for every clickable surface in the app.
//!
//! Why this exists: GPUI only tracks hover and active (pressed) state reliably
//! on *stateful* elements — those that carry an `.id(...)`. `.active(...)` is a
//! [`StatefulInteractiveElement`] method, so every helper here is bounded on
//! that trait. The practical effect is that you can only reach these helpers
//! *after* calling `.id(...)`, which is exactly the element that needs the
//! feedback. A clickable surface with no press feedback becomes a shape the
//! type system nudges you away from rather than a silent UX gap.
//!
//! The vocabulary:
//! - [`Interaction::pressable`] — the floor every clickable gets: a hand cursor
//!   plus a press dim. Composes on top of an element that already styles its own
//!   hover, so you can sprinkle it onto existing rows/icons without rework.
//! - [`Interaction::pressable_dim`] — `pressable` plus a gentle whole-element
//!   hover dim. For "wrapper" buttons whose coloured visual lives in a child
//!   (so we can't recolour a background here) — the dim reads through the child.
//! - [`Interaction::hover_press`] / [`Interaction::hover_press_border`] — the
//!   full solid-button treatment: recolour the fill (and optionally the border)
//!   on hover, dim on press.

use gpui::{Hsla, StatefulInteractiveElement, Styled};

/// Opacity while a control is held down — the "click effect". A touch stronger
/// than [`HOVER_OPACITY`] so press reads as a distinct state from hover.
pub const PRESS_OPACITY: f32 = 0.72;

/// Opacity on hover for controls whose own fill we don't recolour (wrapper
/// buttons, icon buttons that already tint their glyph, etc.).
pub const HOVER_OPACITY: f32 = 0.88;

/// Lighten an `Hsla` toward white by `amount` (fraction of the remaining
/// lightness headroom, `0.0..=1.0`).
pub fn lighten(color: Hsla, amount: f32) -> Hsla {
    Hsla {
        l: (color.l + (1.0 - color.l) * amount).clamp(0.0, 1.0),
        ..color
    }
}

/// Darken an `Hsla` toward black by `amount` (`0.0..=1.0`).
pub fn darken(color: Hsla, amount: f32) -> Hsla {
    Hsla {
        l: (color.l * (1.0 - amount)).clamp(0.0, 1.0),
        ..color
    }
}

/// Nudge a fill toward an accent colour by `t` (`0.0` = unchanged, `1.0` =
/// fully `accent`). Operates per HSLA channel — handy for soft hover tints on
/// near-neutral rows (e.g. easing a panel row toward the brand blue).
pub fn mix(color: Hsla, accent: Hsla, t: f32) -> Hsla {
    let t = t.clamp(0.0, 1.0);
    Hsla {
        h: color.h + (accent.h - color.h) * t,
        s: color.s + (accent.s - color.s) * t,
        l: color.l + (accent.l - color.l) * t,
        a: color.a + (accent.a - color.a) * t,
    }
}

/// Adaptive hover fill: nudge `base` toward the theme foreground a little so a
/// button visibly reacts whether the palette is light or dark. Used as a
/// sensible default when a control has no dedicated hover token.
pub fn hover_shift(base: Hsla, is_dark: bool) -> Hsla {
    if is_dark {
        lighten(base, 0.10)
    } else {
        darken(base, 0.05)
    }
}

/// Press / hover / cursor helpers for any stateful, styled element. Available
/// only after `.id(...)` (the `StatefulInteractiveElement` bound), which is the
/// whole point — see the module docs.
pub trait Interaction: StatefulInteractiveElement + Styled + Sized {
    /// The floor for every clickable surface: a pointer cursor and a press dim.
    /// Leaves any hover the element already declares untouched, so it layers
    /// cleanly onto existing rows and icon buttons.
    fn pressable(self) -> Self {
        self.cursor_pointer().active(|s| s.opacity(PRESS_OPACITY))
    }

    /// `pressable` plus a subtle whole-element hover dim. For wrapper buttons
    /// whose coloured visual lives in a child element (where recolouring a
    /// background here would be painted over by the child).
    fn pressable_dim(self) -> Self {
        self.cursor_pointer()
            .hover(|s| s.opacity(HOVER_OPACITY))
            .active(|s| s.opacity(PRESS_OPACITY))
    }

    /// Full solid-button feedback: recolour the fill on hover, dim on press.
    /// `hover_bg` is typically a palette hover token or a `lighten`/`darken` of
    /// the base fill. Applies to the element that owns the background.
    fn hover_press(self, hover_bg: Hsla) -> Self {
        self.cursor_pointer()
            .hover(move |s| s.bg(hover_bg))
            .active(|s| s.opacity(PRESS_OPACITY))
    }

    /// Like [`hover_press`](Interaction::hover_press) but also shifts the border
    /// on hover — for outlined / ghost buttons where the border is the main
    /// affordance.
    fn hover_press_border(self, hover_bg: Hsla, hover_border: Hsla) -> Self {
        self.cursor_pointer()
            .hover(move |s| s.bg(hover_bg).border_color(hover_border))
            .active(|s| s.opacity(PRESS_OPACITY))
    }
}

impl<T: StatefulInteractiveElement + Styled + Sized> Interaction for T {}

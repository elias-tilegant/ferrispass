//! The keyboard focus ring of every component: 2 px in `focus_ring`, with
//! 1 px of space between it and the control, which shows whatever is behind
//! the control. It lies outside the control, so it takes no room, and focus
//! from a click shows none.

use gpui::{
    App, Div, Hsla, InteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _,
    px,
};

use crate::ui::ds::ActiveTokens as _;
use crate::ui::ds::tokens::dimension;

/// The outline of the control a ring goes around.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outline {
    /// Corners of `dimension::RADIUS_MD`, as on a button or a text field.
    Rounded,
    /// Fully rounded, as on the capsule button and the track of a switch.
    Capsule,
}

/// The ring for a control, when it has focus and the focus came from the
/// keyboard. The control is `relative()` and takes the ring as a child.
pub(super) fn keyboard_focus_ring(
    focused: bool,
    outline: Outline,
    window: &Window,
    cx: &App,
) -> Option<Div> {
    (focused && window.last_input_was_keyboard())
        .then(|| ring(cx.ds().colors.focus_ring.hsla, outline))
}

fn ring(color: Hsla, outline: Outline) -> Div {
    let outset = px(dimension::FOCUS_GAP + dimension::FOCUS_WIDTH);
    div()
        .debug_selector(|| "fp-focus-ring".into())
        .absolute()
        .top(-outset)
        .left(-outset)
        .right(-outset)
        .bottom(-outset)
        .border(px(dimension::FOCUS_WIDTH))
        .border_color(color)
        .map(|this| match outline {
            Outline::Capsule => this.rounded_full(),
            Outline::Rounded => this.rounded(px(dimension::RADIUS_MD) + outset),
        })
}

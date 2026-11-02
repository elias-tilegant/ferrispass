//! The shortcut pair: keycaps next to the word for what they do, as in the
//! panel footer ("⌘C Password").
//!
//! A pair cannot be built without its word, so a keycap never stands alone
//! on a screen. The pair is plain text for VoiceOver, like the footer it sits
//! in: the element the keys act on carries them in its `aria_keyshortcuts`,
//! and [`Shortcut::keys_text`] gives the text for it.

use gpui::{
    App, InteractiveElement as _, IntoElement, Keystroke, ParentElement as _, Refineable as _,
    RenderOnce, SharedString, StyleRefinement, Styled, Window, div,
};

use super::keycap::{Keycap, keys_text, parse_keys};
use crate::ui::ds::ActiveTokens as _;
use crate::ui::ds::tokens::dimension;
use crate::ui::ds::typography::{TextStyle, TextStyled as _, u};

/// Keycaps and their word. The text is `Callout` in `text_secondary` unless
/// the caller styles it otherwise, such as `Headline` for the main action in
/// the footer.
#[derive(IntoElement)]
pub struct Shortcut {
    keys: Vec<Keystroke>,
    label: SharedString,
    style: StyleRefinement,
}

impl Shortcut {
    /// `keys` is a GPUI key string such as `cmd-shift-c`, or several
    /// separated by spaces for a sequence. `label` is the word for what they
    /// do, from a text key or from the caller.
    pub fn from_keystroke(keys: &str, label: impl Into<SharedString>) -> Self {
        Self {
            keys: parse_keys(keys),
            label: label.into(),
            style: StyleRefinement::default(),
        }
    }

    /// The word next to the keycaps.
    pub fn label(&self) -> &SharedString {
        &self.label
    }

    /// The keys as the keycaps show them, which is also what the element
    /// they act on puts into `aria_keyshortcuts`.
    pub fn keys_text(&self) -> SharedString {
        keys_text(&self.keys)
    }
}

impl Styled for Shortcut {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Shortcut {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut pair = div()
            .debug_selector(|| "fp-shortcut".into())
            .flex()
            .items_center()
            .gap(u(dimension::SPACE_4))
            .text_style(TextStyle::Callout)
            .text_color(cx.ds().colors.text_secondary.hsla)
            .children(self.keys.iter().map(Keycap::new))
            .child(
                div()
                    .debug_selector(|| "fp-shortcut-label".into())
                    .child(self.label),
            );
        pair.style().refine(&self.style);
        pair
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Bounds, Pixels, Role, TestAppContext, px};

    use super::super::{Button, probe};
    use super::*;

    fn word(text: &'static str) -> SharedString {
        SharedString::new_static(text)
    }

    fn inside(inner: Bounds<Pixels>, outer: Bounds<Pixels>) -> bool {
        inner.left() >= outer.left()
            && inner.right() <= outer.right()
            && inner.top() >= outer.top()
            && inner.bottom() <= outer.bottom()
    }

    #[test]
    fn shortcut_reads_the_keystroke_text() {
        assert_eq!(
            Shortcut::from_keystroke("cmd-shift-c", word("Password")).keys_text(),
            "⇧⌘C"
        );
        assert_eq!(
            Shortcut::from_keystroke("enter", word("Sign in")).keys_text(),
            "⏎"
        );
        assert_eq!(
            Shortcut::from_keystroke("escape", word("Close")).keys_text(),
            "Esc"
        );
    }

    /// Every keycap has a parent with a name: the word of its pair, or the
    /// button it sits in.
    #[gpui::test]
    fn shortcut_has_a_named_parent(cx: &mut TestAppContext) {
        // In a pair, the keycap sits right before its word, inside the pair.
        let pair = Shortcut::from_keystroke("cmd-c", word("Password"));
        assert_eq!(pair.label().as_ref(), "Password");
        let cx_pair = probe::window(cx, |_, _| {
            Shortcut::from_keystroke("cmd-c", word("Password")).into_any_element()
        });
        let outer = cx_pair.debug_bounds("fp-shortcut").expect("the pair");
        let keycap = cx_pair.debug_bounds("fp-keycap").expect("its keycap");
        let label = cx_pair.debug_bounds("fp-shortcut-label").expect("its word");
        for (part, bounds) in [("keycap", keycap), ("word", label)] {
            assert!(
                inside(bounds, outer),
                "the {part} {bounds:?} lies outside the pair {outer:?}"
            );
        }
        // Layout snaps to device pixels, so the gap is within one of 4 points.
        let gap = cx_pair.update(|window, _| u(dimension::SPACE_4).to_pixels(window.rem_size()));
        let measured = label.left() - keycap.right();
        assert!(
            (measured - gap).abs() <= px(1.),
            "the word follows its keycap: {measured:?}, not {gap:?}"
        );
    }

    /// In a button, the keycap sits inside the button, whose name says what
    /// it does and whose `aria_keyshortcuts` holds the keys.
    #[gpui::test]
    fn a_button_keycap_sits_inside_its_named_button(cx: &mut TestAppContext) {
        let (role, node) = probe::in_frame(cx, |window, cx| {
            let base = Button::secondary("copy", word("Copy password"))
                .shortcut("cmd-shift-c")
                .into_base(window, cx);
            probe::node(RenderOnce::render(base, window, cx))
        });
        assert_eq!(role, Some(Role::Button));
        assert_eq!(node.label(), Some("Copy password"));
        assert_eq!(node.keyboard_shortcut(), Some("⇧⌘C"));

        let cx = probe::window(cx, |_, _| {
            div()
                .debug_selector(|| "fp-test-button".into())
                .child(Button::secondary("copy", word("Copy password")).shortcut("cmd-shift-c"))
                .into_any_element()
        });
        let button = cx.debug_bounds("fp-test-button").expect("the button");
        let keycap = cx.debug_bounds("fp-keycap").expect("its keycap");
        assert!(
            inside(keycap, button),
            "the keycap {keycap:?} lies outside the button {button:?}"
        );
    }
}

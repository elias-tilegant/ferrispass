//! The shortcut pair: keycaps next to the word for what they do, as in the
//! panel footer ("⌘C Password").
//!
//! A pair cannot be built without its word, so a keycap never stands alone
//! on a screen. A pair for a command takes both from the command table in
//! `src/app/registry`: the command's first key and its word, so a command
//! never goes by two names. The pair is not in the accessibility tree at all: it has no
//! id and no role, and neither have its keycaps and its word, so GPUI writes
//! no node for any of them. The keys reach VoiceOver only through
//! `aria_keyshortcuts` on the control they act on, for which
//! [`Shortcut::keys_text`] gives the text, and the macOS adapter of AccessKit
//! in `Cargo.lock` (0.26.3) does not pass that on. A hint row of shortcut
//! pairs that VoiceOver users are to hear has to put its text into the
//! description of the focused element.

use gpui::{
    App, InteractiveElement as _, IntoElement, Keystroke, ParentElement as _, Refineable as _,
    RenderOnce, SharedString, StyleRefinement, Styled, Window, div,
};

use super::keycap::{Keycap, keys_text, parse_keys};
use crate::app::registry::{self, CommandId};
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
    /// do, from a text key or from the caller. Panics if it is empty.
    pub fn from_keystroke(keys: &str, label: impl Into<SharedString>) -> Self {
        let label = label.into();
        // Without its word the pair would be keycaps alone.
        assert!(!label.is_empty(), "a shortcut needs its word");
        Self {
            keys: parse_keys(keys),
            label,
            style: StyleRefinement::default(),
        }
    }

    /// The pair for a command: the first of its keys in the command table
    /// and its word, `(spec.label)()`, both read from the table. A command
    /// without keys shows its word alone, with no keycap.
    ///
    /// Panics if the command has no row in the table. Every `CommandId` is
    /// one of the names the table declares, and the registry's tests give
    /// each of them a row.
    pub fn for_command(id: CommandId) -> Self {
        let spec = registry::command(id)
            .unwrap_or_else(|| panic!("{} has no row in the command table", id.as_str()));
        let keys = spec.keys.first().copied().unwrap_or_default();
        Self::from_keystroke(keys, (spec.label)())
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
    use crate::app::registry::{self, CommandId};

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

    /// Without its word a pair is keycaps alone, so an empty word is a
    /// mistake in the code, in a release build too.
    #[test]
    #[should_panic(expected = "a shortcut needs its word")]
    fn a_shortcut_needs_its_word_in_every_build() {
        let _ = Shortcut::from_keystroke("cmd-c", SharedString::default());
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

    /// The pair for copying the password shows ⌘C, the first key the
    /// command table gives the command, and the table's word for it. Closing
    /// the panel has two keys, ⌘W and Esc, and its pair shows the first.
    #[gpui::test]
    fn a_shortcut_for_a_command_shows_its_first_key_and_its_word(cx: &mut TestAppContext) {
        let pair = Shortcut::for_command(CommandId::ENTRY_COPY_PASSWORD);
        assert_eq!(pair.keys_text(), "⌘C");
        // The English of `action.copy_password.label`, written out.
        assert_eq!(pair.label().as_ref(), "Copy password");
        let close = Shortcut::for_command(CommandId::PANEL_CLOSE);
        assert_eq!(close.keys_text(), "⌘W", "the first of ⌘W and Esc");
        assert_eq!(close.label().as_ref(), "Close");

        let cx = probe::window(cx, |_, _| {
            Shortcut::for_command(CommandId::ENTRY_COPY_PASSWORD).into_any_element()
        });
        let keycap = cx.debug_bounds("fp-keycap").expect("its keycap");
        let label = cx.debug_bounds("fp-shortcut-label").expect("its word");
        assert!(keycap.right() <= label.left(), "the keycap comes first");
    }

    /// Signing in to an SAP system has no keys of its own; Return reaches
    /// it through the main action. Its pair is the word alone, with no
    /// keycap.
    #[gpui::test]
    fn a_command_without_keys_shows_only_its_word(cx: &mut TestAppContext) {
        let spec = registry::command(CommandId::ENTRY_SAP_LOGIN).expect("a row");
        assert!(spec.keys.is_empty(), "the command has no keys");
        let pair = Shortcut::for_command(CommandId::ENTRY_SAP_LOGIN);
        assert_eq!(pair.keys_text(), "");
        assert_eq!(pair.label().as_ref(), "Sign in");

        let cx = probe::window(cx, |_, _| {
            Shortcut::for_command(CommandId::ENTRY_SAP_LOGIN).into_any_element()
        });
        assert_eq!(cx.debug_bounds("fp-keycap"), None, "no keycap");
        assert!(cx.debug_bounds("fp-shortcut-label").is_some(), "the word");
    }

    /// Every command in the table makes a pair: its first key as the
    /// keycaps show it, or none, and the word the table gives it.
    #[test]
    fn every_command_with_keys_makes_a_shortcut() {
        let mut with_keys = 0;
        for spec in registry::commands() {
            let pair = Shortcut::for_command(spec.id);
            let name = spec.id.as_str();
            match spec.keys.first() {
                Some(first) => {
                    with_keys += 1;
                    let shown = keys_text(&parse_keys(first));
                    assert!(!shown.is_empty(), "{name}: {first} reads as no key");
                    assert_eq!(pair.keys_text(), shown, "{name}");
                }
                None => assert_eq!(pair.keys_text(), "", "{name}"),
            }
            assert_eq!(pair.label(), &(spec.label)(), "{name}");
        }
        assert!(with_keys > 1, "the table has commands with keys");
    }
}

//! The switch: the GPUI Kit's switch with its label next to it and a
//! sentence below that says what changes.
//!
//! The Kit paints a switch that is on in its primary color, which in this
//! theme is copper. A switch is a setting, not the action a view is for, so
//! "on" is `switch_on_bg`, the color of text, and copper stays with the one
//! button that has it. The Kit writes its own label at 11.4 px; this one
//! stands beside the switch in `Body` and is the switch's name for
//! VoiceOver. The track keeps the Kit's fixed size of 36 by 20 pixels at
//! both text sizes.
//!
//! The Kit switch takes no description, so the sentence below is a node of
//! its own, a label with the sentence as its value, which VoiceOver reaches
//! right after the switch.
//!
//! The Kit's own focus ring, a tinted edge on the track with a 3 px band at
//! half opacity and no gap, is off. The track shows the ring of every
//! component instead, for focus from the keyboard. The Kit switch keeps its
//! focus handle to itself, so the track sits in a wrapper that tracks a
//! handle of its own, which is never a tab stop. Focus inside the wrapper
//! is the switch's: nothing else in there can take focus, and a press on
//! the switch focuses the switch, not the wrapper. A disabled switch takes
//! no focus, and its wrapper tracks none.

use std::rc::Rc;

use gpui::{
    App, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_base::TestSupportExt as _;
use gpui_component::switch::Switch as KitSwitch;
use gpui_component::{Disableable as _, FocusableExt as _};

use super::focus_ring::{Outline, keyboard_focus_ring};
use crate::ui::ds::ActiveTokens as _;
use crate::ui::ds::tokens::dimension;
use crate::ui::ds::typography::{TextStyle, TextStyled as _, u};

type ChangeHandler = Rc<dyn Fn(&bool, &mut Window, &mut App)>;

/// A labelled switch. The screen owns the value: the handler gets the value
/// asked for, and the screen passes it back through [`Switch::checked`].
#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    label: SharedString,
    note: Option<SharedString>,
    checked: bool,
    disabled: bool,
    on_change: Option<ChangeHandler>,
}

impl Switch {
    /// `label` stands next to the switch and is its name for VoiceOver.
    /// Panics if it is empty.
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        let label = label.into();
        assert!(!label.is_empty(), "a switch needs a name");
        Self {
            id: id.into(),
            label,
            note: None,
            checked: false,
            disabled: false,
            on_change: None,
        }
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The sentence under the switch about what changes when it is on.
    pub fn note(mut self, sentence: impl Into<SharedString>) -> Self {
        self.note = Some(sentence.into());
        self
    }

    /// Gets the value a click, Space or VoiceOver asks for. A disabled switch
    /// does not call it.
    pub fn on_change(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.ds().colors;
        // Focus inside the wrapper around the track is the switch's.
        let frame = window
            .use_keyed_state((self.id.clone(), "focus-frame"), cx, |_, cx| {
                cx.focus_handle()
            })
            .read(cx)
            .clone();
        let focused = !self.disabled && frame.contains_focused(window, cx);
        let ring = keyboard_focus_ring(focused, Outline::Capsule, window, cx);
        let note = self
            .note
            .map(|sentence| note_line(self.id.clone(), sentence, cx));
        let switch = KitSwitch::new(self.id)
            .checked(self.checked)
            .disabled(self.disabled)
            .accessibility_label(self.label.clone())
            .color(colors.switch_on_bg.hsla)
            .focus_ring(false)
            .when_some(
                self.on_change.filter(|_| !self.disabled),
                |switch, handler| {
                    switch.on_change(move |checked, window, cx| handler(checked, window, cx))
                },
            );
        div()
            .flex()
            .flex_col()
            .gap(u(dimension::SPACE_4))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(u(dimension::SPACE_8))
                    .child(
                        div()
                            .debug_selector(|| "fp-switch-track".into())
                            .relative()
                            .when(!self.disabled, |this| this.track_focus(&frame))
                            .child(switch)
                            .children(ring),
                    )
                    .child(
                        div()
                            .text_style(TextStyle::Body)
                            .text_color(if self.disabled {
                                colors.text_tertiary.hsla
                            } else {
                                colors.text_primary.hsla
                            })
                            .child(self.label),
                    ),
            )
            .children(note)
    }
}

/// The sentence under a switch. The Kit switch takes no description, so the
/// sentence is a node of its own, which VoiceOver reaches right after the
/// switch. A text without an id and a role writes no node at all.
fn note_line(switch: ElementId, sentence: SharedString, cx: &App) -> impl IntoElement {
    div()
        .id((switch, "note"))
        // The tests find the node through this; in the app it is the plain
        // element.
        .test_support()
        .role(Role::Label)
        .aria_value(sentence.clone())
        .text_style(TextStyle::Callout)
        .text_color(cx.ds().colors.text_secondary.hsla)
        .child(sentence)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{Hsla, Modifiers, TestAppContext, px, size};

    use super::super::probe;
    use super::*;
    use crate::app::settings::ThemeChoice;
    use crate::ui::ds::tokens::{Mode, TEXT_SIZE_STANDARD};
    use crate::ui::ds::{self, A11yPrefs, Appearance};

    fn text(value: &'static str) -> SharedString {
        SharedString::new_static(value)
    }

    fn apply(cx: &mut TestAppContext, mode: Mode) {
        cx.update(|cx| {
            gpui_component::init(cx);
            ds::apply(
                cx,
                Appearance {
                    choice: if mode.is_dark() {
                        ThemeChoice::Dark
                    } else {
                        ThemeChoice::Light
                    },
                    prefs: A11yPrefs {
                        increase_contrast: mode.increased_contrast(),
                        ..A11yPrefs::default()
                    },
                    text_size: TEXT_SIZE_STANDARD,
                },
            );
        });
    }

    /// The opaque fills a window with one switch paints.
    fn fills(cx: &mut TestAppContext, checked: bool) -> Vec<Hsla> {
        let cx = probe::window(cx, move |_, _| {
            Switch::new("autolock", text("Lock when idle"))
                .checked(checked)
                .into_any_element()
        });
        cx.update(|window, _| {
            window
                .painted_quads()
                .into_iter()
                .filter_map(|quad| quad.background.as_solid())
                .map(|color| Hsla { a: 1.0, ..color })
                .collect()
        })
    }

    #[test]
    #[should_panic(expected = "a switch needs a name")]
    fn a_switch_needs_a_name_in_every_build() {
        let _ = Switch::new("unnamed", SharedString::default());
    }

    #[gpui::test]
    fn switch_on_is_never_copper(cx: &mut TestAppContext) {
        let _lock = ds::tests::lock_mode();
        for mode in Mode::ALL {
            apply(cx, mode);
            let colors = mode.colors();
            let copper = [
                colors.accent_fill.hsla,
                colors.accent_fill_hover.hsla,
                colors.button_primary_bg.hsla,
            ];
            for checked in [true, false] {
                let painted = fills(cx, checked);
                assert!(
                    !painted.iter().any(|color| copper.contains(color)),
                    "{mode:?}, on: {checked}: {painted:?}"
                );
            }
            assert!(
                fills(cx, true).contains(&colors.switch_on_bg.hsla),
                "{mode:?}: on is switch_on_bg"
            );
        }
    }

    #[gpui::test]
    fn switch_has_role_name_and_state(cx: &mut TestAppContext) {
        let asked = Rc::new(Cell::new(None));
        let seen = asked.clone();
        let cx = probe::window(cx, move |_, _| {
            let seen = seen.clone();
            Switch::new("autolock", text("Lock when idle"))
                .checked(true)
                .note(text("After four minutes without input."))
                .on_change(move |checked, _, _| seen.set(Some(*checked)))
                .into_any_element()
        });
        let switch = cx.update(|window, _| {
            gpui_base::test_support::snapshots(window)
                .into_iter()
                .find(|element| element.role() == Some(Role::Switch))
                .expect("a switch node")
        });
        assert_eq!(switch.label(), Some("Lock when idle"));
        assert_eq!(switch.checked(), Some(true));

        cx.simulate_click(switch.bounds().center(), Modifiers::default());
        assert_eq!(asked.get(), Some(false), "a click asks for off");
    }

    /// Tab shows the ring around the track and a click does not. The Kit's
    /// own ring, which tints the track's edge and lays a band at half
    /// opacity around it, is off, so the ring is the only edge focus adds.
    #[gpui::test]
    fn the_switch_shows_the_focus_ring_for_the_keyboard_only(cx: &mut TestAppContext) {
        let cx = probe::window(cx, |_, _| {
            div()
                .p(px(8.))
                .child(Switch::new("autolock", text("Lock when idle")).checked(true))
                .into_any_element()
        });
        let track = probe::assert_keyboard_focus_ring(cx, "fp-switch-track");
        assert_eq!(
            track.size,
            size(px(36.), px(20.)),
            "the ring goes around the track"
        );
    }

    /// The sentence under a switch is a node of its own, a label with the
    /// sentence as its value, drawn below the switch, because the Kit switch
    /// takes no description that would carry it.
    #[gpui::test]
    fn a_switch_note_is_a_node_with_its_sentence(cx: &mut TestAppContext) {
        const NOTE: &str = "After four minutes without input.";
        let cx = probe::window(cx, |_, _| {
            Switch::new("autolock", text("Lock when idle"))
                .checked(true)
                .note(text(NOTE))
                .into_any_element()
        });
        let nodes = cx.update(|window, _| gpui_base::test_support::snapshots(window));
        let switch = nodes
            .iter()
            .find(|node| node.role() == Some(Role::Switch))
            .expect("a switch node");
        let note = nodes
            .iter()
            .find(|node| node.role() == Some(Role::Label) && node.value() == Some(NOTE))
            .unwrap_or_else(|| panic!("a label node with the sentence: {nodes:?}"));
        assert!(note.visible(), "the sentence is drawn: {note:?}");
        assert!(
            note.bounds().top() >= switch.bounds().bottom(),
            "the sentence is below the switch: {note:?}, switch {switch:?}"
        );
    }
}

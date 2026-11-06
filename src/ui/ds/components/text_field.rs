//! The text field: the GPUI Kit's input at the design's size, with its
//! label above it and, when something is wrong, a sentence below it.
//!
//! The Kit input keeps what is hard to get right (editing, selection, input
//! methods, the context menu). This wrapper changes four things about it:
//!
//! - Its height and text size. Kit Medium is 26 high at text size 13 and
//!   writes 11.4 px; the field is 28 high and writes `Body` at 13, and both
//!   grow with the larger text size.
//! - Its name. Without one, the Kit names a field after its placeholder, and
//!   macOS then reads the placeholder as name and value at once. The field's
//!   name is always its label.
//! - Its edge, which turns carmine with an error.
//! - Its focus ring. The Kit tints the edge and lays a 3 px band at half
//!   opacity around it, without a gap; the field turns that off and shows
//!   the ring of every component instead, for focus from the keyboard.

use gpui::{
    App, Div, ElementId, Entity, FocusHandle, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, Role, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::input::{Input, InputState};
use gpui_component::{FocusableExt as _, Icon, IconName};

use super::AccessibilityId;
use super::focus_ring::{Outline, keyboard_focus_ring};
use crate::ui::ds::ActiveTokens as _;
use crate::ui::ds::tokens::dimension;
use crate::ui::ds::typography::{TextStyle, TextStyled as _, u};

/// Size of the glyph before an error sentence, in points.
const ERROR_GLYPH: f32 = 12.0;

/// A labelled text field. The text lives in the Kit's `InputState`, which the
/// screen owns.
#[derive(IntoElement)]
pub struct TextField {
    id: ElementId,
    state: Entity<InputState>,
    label: SharedString,
    error: Option<SharedString>,
    disabled: bool,
    accessibility_id: Option<AccessibilityId>,
}

impl TextField {
    /// `label` stands above the field and is its name for VoiceOver.
    /// Panics if it is empty.
    pub fn new(
        id: impl Into<ElementId>,
        state: &Entity<InputState>,
        label: impl Into<SharedString>,
    ) -> Self {
        let label = label.into();
        assert!(!label.is_empty(), "a text field needs a name");
        Self {
            id: id.into(),
            state: state.clone(),
            label,
            error: None,
            disabled: false,
            accessibility_id: None,
        }
    }

    /// A sentence under the field that says what is wrong and what to do.
    pub fn error(mut self, sentence: impl Into<SharedString>) -> Self {
        self.error = Some(sentence.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn accessibility_id(mut self, id: AccessibilityId) -> Self {
        self.accessibility_id = Some(id);
        self
    }

    /// The focus handle of the Kit input, which holds keyboard focus while
    /// someone types in the field.
    pub(in crate::ui::ds) fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.state.focus_handle(cx)
    }

    /// The Kit input, sized, named and colored for this field.
    pub(in crate::ui::ds) fn input(&self, cx: &App) -> Input {
        let colors = cx.ds().colors;
        let edge = if self.error.is_some() {
            colors.field_border_error
        } else {
            colors.field_border
        };
        let input = Input::new(&self.state)
            .id((self.id.clone(), "input"))
            .aria_label(self.label.clone())
            .disabled(self.disabled)
            .focus_ring(false)
            .when_some(self.accessibility_id.as_ref(), |input, id| {
                id.apply_to_input(input)
            });
        // `Input::h` sets the height of a multi-line field only; the style
        // sets it for this one.
        Styled::h(input, u(dimension::HEIGHT_CONTROL))
            .rounded(px(dimension::RADIUS_MD))
            .border_color(edge.hsla)
            .bg(colors.field_bg.hsla)
            .text_style(TextStyle::Body)
    }
}

impl RenderOnce for TextField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.ds().colors;
        let input = self.input(cx);
        let focused = !self.disabled && self.state.focus_handle(cx).is_focused(window);
        let ring = keyboard_focus_ring(focused, Outline::Rounded, window, cx);
        div()
            .flex()
            .flex_col()
            .gap(u(dimension::SPACE_4))
            .child(
                div()
                    .text_style(TextStyle::Callout)
                    .text_color(colors.text_secondary.hsla)
                    .child(self.label),
            )
            .child(
                div()
                    .debug_selector(|| "fp-text-field-input".into())
                    .relative()
                    .child(input)
                    .children(ring),
            )
            .when_some(self.error, |this, sentence| {
                this.child(error_line(self.id, sentence, cx))
            })
    }
}

/// The sentence under a field with an error. The Kit input takes no
/// description, so the sentence is a node of its own, which VoiceOver
/// reaches right after the field.
fn error_line(field: ElementId, sentence: SharedString, cx: &App) -> Stateful<Div> {
    let color = cx.ds().colors.status_danger_text.hsla;
    div()
        .id((field, "error"))
        .role(Role::Label)
        .aria_value(sentence.clone())
        .flex()
        .items_center()
        .gap(u(dimension::SPACE_4))
        .text_style(TextStyle::Callout)
        .text_color(color)
        .child(
            Icon::new(IconName::CircleX)
                .size(u(ERROR_GLYPH))
                .text_color(color),
        )
        .child(sentence)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{
        AppContext as _, Bounds, EntityInputHandler as _, Font, Hsla, Pixels, TestAppContext,
        TextRun, VisualTestContext, font,
    };

    use super::super::{Button, probe};
    use super::*;
    use crate::app::settings::ThemeChoice;
    use crate::ui::ds::tokens::{LIGHT, TEXT_SIZE_LARGE, TEXT_SIZE_STANDARD};
    use crate::ui::ds::{self, A11yPrefs, Appearance};

    fn text(value: &'static str) -> SharedString {
        SharedString::new_static(value)
    }

    fn apply_text_size(cx: &mut TestAppContext, text_size: f32) {
        cx.update(|cx| {
            gpui_component::init(cx);
            ds::apply(
                cx,
                Appearance {
                    choice: ThemeChoice::Light,
                    prefs: A11yPrefs::default(),
                    text_size,
                },
            );
        });
    }

    /// The height of the field's input in a window at `text_size`.
    fn input_height(cx: &mut TestAppContext, text_size: f32) -> f32 {
        apply_text_size(cx, text_size);
        let cx = probe::window(cx, |window, cx| {
            let state = cx.new(|cx| InputState::new(window, cx));
            TextField::new("server", &state, text("Server")).into_any_element()
        });
        let bounds = cx.debug_bounds("fp-text-field-input").expect("the input");
        f32::from(bounds.size.height)
    }

    #[gpui::test]
    #[should_panic(expected = "a text field needs a name")]
    fn a_text_field_needs_a_name_in_every_build(cx: &mut TestAppContext) {
        probe::in_frame(cx, |window, cx| {
            let state = cx.new(|cx| InputState::new(window, cx));
            let _ = TextField::new("unnamed", &state, SharedString::default());
        });
    }

    #[gpui::test]
    fn text_field_is_28_high_at_13_and_32_at_15(cx: &mut TestAppContext) {
        let _lock = ds::tests::lock_mode();
        assert_eq!(input_height(cx, TEXT_SIZE_STANDARD), 28.0);
        // 28 points at 13 are 32.3 at 15: the field grows with the text.
        // Layout snaps to device pixels, half a point at the test's scale.
        let large = input_height(cx, TEXT_SIZE_LARGE);
        let expected = 28.0 * TEXT_SIZE_LARGE / TEXT_SIZE_STANDARD;
        assert!((large - expected).abs() <= 0.5, "{large}, not {expected}");
    }

    /// How wide the field's input sets `value` in a window at `text_size`,
    /// and how wide `value` is in `Body` there. The test platform paints no
    /// glyphs, but the input lays its text out with the size it was given,
    /// and the bounds it reports for a range of it come from that layout.
    fn input_text_widths(
        cx: &mut TestAppContext,
        text_size: f32,
        value: &'static str,
    ) -> (Pixels, Pixels) {
        apply_text_size(cx, text_size);
        let state: Rc<RefCell<Option<Entity<InputState>>>> = Rc::default();
        let kept = state.clone();
        let cx = probe::window(cx, move |window, cx| {
            let state = kept
                .borrow_mut()
                .get_or_insert_with(|| {
                    cx.new(|cx| InputState::new(window, cx).default_value(value))
                })
                .clone();
            TextField::new("server", &state, text("Server")).into_any_element()
        });
        let state = state.borrow().clone().expect("the input state");
        let laid_out = cx.update(|window, cx| {
            state.update(cx, |state, cx| {
                state.bounds_for_range(0..value.len(), Bounds::default(), window, cx)
            })
        });
        let laid_out = laid_out.expect("the input laid out its text").size.width;
        (laid_out, text_width(cx, value, TextStyle::Body))
    }

    /// The Kit input writes 11.4 px at text size 13; the field writes
    /// `Body`, 13 points, and grows with the larger text size.
    #[gpui::test]
    fn text_field_writes_body_at_13_and_15(cx: &mut TestAppContext) {
        const VALUE: &str = "nhp.example.com:3200";
        let _lock = ds::tests::lock_mode();
        for text_size in [TEXT_SIZE_STANDARD, TEXT_SIZE_LARGE] {
            let (laid_out, body) = input_text_widths(cx, text_size, VALUE);
            assert!(
                (laid_out - body).abs() <= px(0.5),
                "at {text_size}: the input sets its text {laid_out:?} wide, in Body it is {body:?}"
            );
        }
    }

    #[gpui::test]
    fn text_field_has_its_own_name_not_the_placeholder(cx: &mut TestAppContext) {
        let names = probe::in_frame(cx, |window, cx| {
            let state =
                cx.new(|cx| InputState::new(window, cx).placeholder(text("name@example.com")));
            let field = TextField::new("user", &state, text("User name"));
            let (role, ours) = probe::node(RenderOnce::render(field.input(cx), window, cx));
            // What the Kit does without a name of its own.
            let (_, kit) = probe::node(RenderOnce::render(Input::new(&state), window, cx));
            (
                role,
                ours.label().map(str::to_string),
                ours.placeholder().map(str::to_string),
                kit.label().map(str::to_string),
            )
        });
        let (role, name, placeholder, kit_name) = names;
        assert_eq!(role, Some(Role::TextInput));
        assert_eq!(name.as_deref(), Some("User name"));
        assert_eq!(placeholder.as_deref(), Some("name@example.com"));
        assert_eq!(
            kit_name.as_deref(),
            Some("name@example.com"),
            "the Kit names a field after its placeholder"
        );
    }

    /// Tab shows the ring around the input and takes it to the button after
    /// the field, and a click does not show it. The Kit's own ring, which
    /// tints the edge and lays a band at half opacity around it, is off, so
    /// the ring is the only edge focus adds.
    #[gpui::test]
    fn the_text_field_shows_the_focus_ring_for_the_keyboard_only(cx: &mut TestAppContext) {
        let state: Rc<RefCell<Option<Entity<InputState>>>> = Rc::default();
        let cx = probe::window(cx, move |window, cx| {
            let state = state
                .borrow_mut()
                .get_or_insert_with(|| cx.new(|cx| InputState::new(window, cx)))
                .clone();
            div()
                .p(px(8.))
                .child(TextField::new("server", &state, text("Server")))
                .child(Button::secondary("connect", text("Connect")))
                .into_any_element()
        });
        probe::assert_keyboard_focus_ring(cx, "fp-text-field-input");
    }

    #[gpui::test]
    fn an_error_turns_the_edge_carmine_and_is_read_after_the_field(cx: &mut TestAppContext) {
        let edges = |cx: &mut TestAppContext, error: Option<&'static str>| -> Vec<Hsla> {
            let cx = probe::window(cx, move |window, cx| {
                let state = cx.new(|cx| InputState::new(window, cx));
                let field = TextField::new("server", &state, text("Server"));
                match error {
                    Some(sentence) => field.error(text(sentence)),
                    None => field,
                }
                .into_any_element()
            });
            cx.update(|window, _| {
                window
                    .painted_quads()
                    .into_iter()
                    .map(|quad| quad.border_color)
                    .collect()
            })
        };
        let plain = edges(cx, None);
        assert!(plain.contains(&LIGHT.field_border.hsla));
        assert!(!plain.contains(&LIGHT.field_border_error.hsla));
        let wrong = edges(cx, Some("The address needs a port."));
        assert!(wrong.contains(&LIGHT.field_border_error.hsla));

        let (role, node) = probe::in_frame(cx, |_, cx| {
            probe::node(error_line(
                "server".into(),
                text("The address needs a port."),
                cx,
            ))
        });
        assert_eq!(role, Some(Role::Label));
        assert_eq!(node.value(), Some("The address needs a port."));
    }

    /// A field drawn in a window under an underline, which its texts
    /// inherit. The test platform paints no glyphs, but it does paint the
    /// underline of each line of text, as wide as the line.
    fn underlined_field<'a>(
        cx: &'a mut TestAppContext,
        error: Option<&'static str>,
    ) -> &'a mut VisualTestContext {
        probe::window(cx, move |window, cx| {
            let state = cx.new(|cx| InputState::new(window, cx));
            let field = TextField::new("server", &state, text("Server"));
            let field = match error {
                Some(sentence) => field.error(text(sentence)),
                None => field,
            };
            div().underline().child(field).into_any_element()
        })
    }

    /// Each line of text the window painted, with its color.
    fn painted_lines(cx: &mut VisualTestContext) -> Vec<(Bounds<Pixels>, Hsla)> {
        cx.update(|window, _| {
            let scale = window.scale_factor();
            window
                .painted_underlines()
                .into_iter()
                .map(|line| (line.bounds.map(|value| px(value.0 / scale)), line.color))
                .collect()
        })
    }

    /// How wide `text` is when set in `style` in this window.
    fn text_width(cx: &mut VisualTestContext, text: &'static str, style: TextStyle) -> Pixels {
        cx.update(|window, _| {
            let run = TextRun {
                len: text.len(),
                font: Font {
                    weight: style.weight(),
                    ..font(style.family().name())
                },
                color: Hsla::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let size = u(style.size()).to_pixels(window.rem_size());
            window
                .text_system()
                .shape_line(text.into(), size, &[run], None)
                .width
        })
    }

    /// The error sentence is part of the field: the field draws it below
    /// the input, as text people see and not only as the value VoiceOver
    /// reads.
    #[gpui::test]
    fn an_error_shows_its_sentence_below_the_input(cx: &mut TestAppContext) {
        const SENTENCE: &str = "The address needs a port.";

        let plain = underlined_field(cx, None);
        let input = plain
            .debug_bounds("fp-text-field-input")
            .expect("the input");
        let lines = painted_lines(plain);
        assert!(
            lines.iter().any(|(line, _)| line.bottom() <= input.top()),
            "the label is painted above the input: {lines:?}"
        );
        assert!(
            lines.iter().all(|(line, _)| line.top() < input.bottom()),
            "without an error nothing is painted below the input: {lines:?}"
        );

        let wrong = underlined_field(cx, Some(SENTENCE));
        let input = wrong
            .debug_bounds("fp-text-field-input")
            .expect("the input");
        let lines = painted_lines(wrong);
        let below: Vec<_> = lines
            .iter()
            .filter(|(line, _)| line.top() >= input.bottom())
            .collect();
        assert_eq!(
            below.len(),
            1,
            "one line of text below the input, the sentence: {lines:?}"
        );
        let (line, color) = *below[0];
        let danger = wrong.update(|_, cx| cx.ds().colors.status_danger_text.hsla);
        assert_eq!(color, danger, "the sentence is in the danger color");
        let width = text_width(wrong, SENTENCE, TextStyle::Callout);
        assert!(
            (line.size.width - width).abs() <= px(0.5),
            "the line is the sentence in Callout: {:?} wide, not {width:?}",
            line.size.width
        );
        assert!(
            line.left() > input.left(),
            "the sentence follows its glyph: {line:?}, input {input:?}"
        );
    }
}

//! The text field: the GPUI Kit's input at the design's size, with its
//! label above it and, when something is wrong, a sentence below it.
//!
//! The Kit input keeps what is hard to get right (editing, selection, input
//! methods, the context menu). This wrapper changes three things about it:
//!
//! - Its height and text size. Kit Medium is 26 high at text size 13 and
//!   writes 11.4 px; the field is 28 high and writes `Body` at 13, and both
//!   grow with the larger text size.
//! - Its name. Without one, the Kit names a field after its placeholder, and
//!   macOS then reads the placeholder as name and value at once. The field's
//!   name is always its label.
//! - Its edge, which turns carmine with an error.

use gpui::{
    App, Div, ElementId, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, Role, SharedString, Stateful, StatefulInteractiveElement as _, Styled, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::input::{Input, InputState};
use gpui_component::{Icon, IconName};

use super::AccessibilityId;
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
    pub fn new(
        id: impl Into<ElementId>,
        state: &Entity<InputState>,
        label: impl Into<SharedString>,
    ) -> Self {
        let label = label.into();
        debug_assert!(!label.is_empty(), "a text field needs a name");
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
            .when_some(self.accessibility_id.clone(), |input, id| {
                input.accessibility_id(id)
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
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.ds().colors;
        let input = self.input(cx);
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
                    .child(input),
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
    use gpui::{AppContext as _, Hsla, TestAppContext};

    use super::super::probe;
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
    fn text_field_is_28_high_at_13_and_32_at_15(cx: &mut TestAppContext) {
        let _lock = ds::tests::lock_mode();
        assert_eq!(input_height(cx, TEXT_SIZE_STANDARD), 28.0);
        // 28 points at 13 are 32.3 at 15: the field grows with the text.
        // Layout snaps to device pixels, half a point at the test's scale.
        let large = input_height(cx, TEXT_SIZE_LARGE);
        let expected = 28.0 * TEXT_SIZE_LARGE / TEXT_SIZE_STANDARD;
        assert!((large - expected).abs() <= 0.5, "{large}, not {expected}");
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
}

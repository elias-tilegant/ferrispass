//! The secret field: a password, or another secret of an entry, as the
//! panel and an entry's details show it.
//!
//! Hidden, it shows twelve dots, whatever the length of the secret, so the
//! length cannot be read off the screen. Revealed, it shows the secret in
//! `Secret`, in groups of four, its digits in `secret_digit`, its symbols in
//! `secret_symbol` and its letters in `text_primary`. Whether it is revealed
//! is the caller's to say, and so is when that ends: on a new selection,
//! when focus leaves, when the app locks and after 60 seconds.
//!
//! For VoiceOver the field is a group named after what it holds. Inside it
//! the value is a button that copies, named "Copy password" and described
//! as hidden while it is; next to it the button that reveals the secret,
//! which says whether it is on, and the copy button. The secret reaches the
//! tree only as the value of the value button, and only while it is
//! revealed and assistive technology is running, which the caller says
//! through [`SecretField::assistive`]: the field does not find that out
//! itself. The revealed text is styled text without an id, which writes no
//! node of its own, and so are the dots.
//!
//! The field never copies. A click on the value or on the copy button runs
//! the handler the caller gave it, which hands the secret to the clipboard
//! service; a field without that handler has neither button. That a copy
//! flashes the value and shows a tick on the copy button is a state the
//! caller sets, [`CopyState`], and so is how long it lasts.

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, ClickEvent, Div, ElementId, HighlightStyle, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, Role, SharedString, Stateful, StatefulInteractiveElement as _,
    Styled as _, StyledText, Window, accesskit, div, prelude::FluentBuilder as _, px,
};
use gpui_component::IconName;
use zeroize::Zeroizing;

use super::button::{ClickHandler, voiceover_press};
use super::focus_ring::{Outline, keyboard_focus_ring};
use super::{AccessibilityId, IconButton, counted};
use crate::tr;
use crate::ui::ds::ActiveTokens as _;
use crate::ui::ds::tokens::{ColorSet, dimension};
use crate::ui::ds::typography::{TextStyle, TextStyled as _, u};

/// What a hidden secret shows, whatever its length: twelve dots.
const MASK: &str = "••••••••••••";

/// A revealed secret is shown in groups of this many characters.
const GROUP: usize = 4;

/// Between the groups of a revealed secret.
const GROUP_GAP: char = ' ';

/// What a copy shows, set by the caller for as long as it should show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CopyState {
    #[default]
    Idle,
    /// Right after a copy: the value flashes in `bg_selected`, and the copy
    /// button shows a tick.
    Flash,
    /// After the flash: the copy button still shows a tick.
    Check,
}

impl CopyState {
    pub const ALL: [CopyState; 3] = [Self::Idle, Self::Flash, Self::Check];
}

/// A secret, hidden unless the caller reveals it. The field has no `Debug`,
/// so it cannot be printed with its secret.
#[derive(Clone, IntoElement)]
pub struct SecretField {
    id: ElementId,
    label: SharedString,
    value: Option<Zeroizing<String>>,
    revealed: bool,
    assistive: bool,
    copied: CopyState,
    on_copy: Option<ClickHandler>,
    on_toggle_reveal: Option<ClickHandler>,
    accessibility_id: Option<AccessibilityId>,
}

impl SecretField {
    /// `label` names the field for VoiceOver, such as
    /// `tr!("quick.preview.password")`; the field does not show it. Panics on
    /// an empty label, in every build.
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        let label = label.into();
        assert!(!label.is_empty(), "a secret field needs a name");
        Self {
            id: id.into(),
            label,
            value: None,
            revealed: false,
            assistive: false,
            copied: CopyState::Idle,
            on_copy: None,
            on_toggle_reveal: None,
            accessibility_id: None,
        }
    }

    /// The secret. Without one the field shows the dots, revealed or not.
    pub fn value(mut self, value: Zeroizing<String>) -> Self {
        self.value = Some(value);
        self
    }

    /// Whether the secret shows. The caller ends it on a new selection,
    /// when focus leaves, when the app locks and after 60 seconds.
    pub fn revealed(mut self, revealed: bool) -> Self {
        self.revealed = revealed;
        self
    }

    /// Whether assistive technology such as VoiceOver is running. Only then,
    /// and only while revealed, does the secret go into the tree, as the
    /// value of the value button. The field never finds this out itself.
    pub fn assistive(mut self, assistive: bool) -> Self {
        self.assistive = assistive;
        self
    }

    pub fn copied(mut self, copied: CopyState) -> Self {
        self.copied = copied;
        self
    }

    /// Runs for a click on the value or on the copy button, for Space or
    /// Return while one of them has focus, and for VoiceOver's press. The
    /// handler copies; the field does not. Without it the field has no copy
    /// button, and its value is no button either.
    pub fn on_copy(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_copy = Some(Rc::new(handler));
        self
    }

    /// Runs for the button that reveals and hides the secret. Without it the
    /// field has no such button.
    pub fn on_toggle_reveal(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_toggle_reveal = Some(Rc::new(handler));
        self
    }

    pub fn accessibility_id(mut self, id: AccessibilityId) -> Self {
        self.accessibility_id = Some(id);
        self
    }

    /// Whether the secret shows: revealed, and there is one.
    fn shows_secret(&self) -> bool {
        self.revealed && self.value.is_some()
    }
}

/// What a character of a revealed secret is, which sets its color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CharClass {
    Letter,
    Digit,
    Symbol,
}

impl CharClass {
    fn of(c: char) -> Self {
        if c.is_numeric() {
            Self::Digit
        } else if c.is_alphabetic() {
            Self::Letter
        } else {
            Self::Symbol
        }
    }

    fn color(self, colors: &ColorSet) -> Hsla {
        match self {
            Self::Letter => colors.text_primary.hsla,
            Self::Digit => colors.secret_digit.hsla,
            Self::Symbol => colors.secret_symbol.hsla,
        }
    }
}

/// A revealed secret as the field shows it: in groups of four, with the
/// class of each character, by its place in the text.
fn grouped(value: &str) -> (Zeroizing<String>, Vec<(Range<usize>, CharClass)>) {
    let mut text = Zeroizing::new(String::with_capacity(value.len() + value.len() / GROUP));
    let mut classes = Vec::new();
    for (index, c) in value.chars().enumerate() {
        if index > 0 && index % GROUP == 0 {
            text.push(GROUP_GAP);
        }
        let start = text.len();
        text.push(c);
        classes.push((start..text.len(), CharClass::of(c)));
    }
    (text, classes)
}

/// What the value shows: the dots, or the revealed secret.
enum Shown {
    Mask(SharedString),
    Revealed(StyledText),
}

/// The value: a button that copies, or, without a copy handler, a plain
/// box that writes no node.
enum Value {
    Button(Stateful<Div>),
    Plain(Div),
}

/// Every element the field builds, before `assemble` puts them in place.
struct Built {
    group: Stateful<Div>,
    value: Value,
    shown: Shown,
    ring: Option<Div>,
    reveal: Option<IconButton>,
    copy: Option<IconButton>,
}

impl SecretField {
    fn build(self, window: &mut Window, cx: &mut App) -> Built {
        let colors = cx.ds().colors;
        let focus = window
            .use_keyed_state((self.id.clone(), "value-focus"), cx, |_, cx| {
                cx.focus_handle()
            })
            .read(cx)
            .clone();
        let shows_secret = self.shows_secret();
        let hidden = !shows_secret;
        // The one place the secret may reach the tree.
        let in_tree: Option<SharedString> = (shows_secret && self.assistive)
            .then(|| {
                self.value
                    .as_ref()
                    .map(|value| value.as_str().to_string().into())
            })
            .flatten();

        let shown = match self.value.as_ref().filter(|_| self.revealed) {
            Some(value) => {
                let (text, classes) = grouped(value);
                let highlights = classes.into_iter().map(|(range, class)| {
                    let style = HighlightStyle {
                        color: Some(class.color(colors)),
                        ..HighlightStyle::default()
                    };
                    (range, style)
                });
                Shown::Revealed(
                    StyledText::new(text.as_str().to_string()).with_highlights(highlights),
                )
            }
            None => Shown::Mask(SharedString::new_static(MASK)),
        };

        let body = counted(div(), "secret-value")
            .debug_selector(|| "fp-secret-value".into())
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .h(u(dimension::HEIGHT_CONTROL))
            .px(u(dimension::SPACE_6))
            .rounded(px(dimension::RADIUS_SM))
            .text_style(TextStyle::Secret)
            .text_color(colors.text_primary.hsla)
            .when(self.copied == CopyState::Flash, |this| {
                this.bg(colors.bg_selected.hsla)
            });
        let (value, ring) = match self.on_copy.clone() {
            Some(handler) => {
                let ring =
                    keyboard_focus_ring(focus.is_focused(window), Outline::Rounded, window, cx);
                let hover = colors.bg_hover.hsla;
                let button = body
                    .id("value")
                    .role(Role::Button)
                    .aria_label(tr!("a11y.copy_password"))
                    .when(hidden, |this| this.aria_description(tr!("a11y.hidden")))
                    .when_some(in_tree, |this, value| this.aria_value(value))
                    .track_focus(&focus.clone().tab_stop(true))
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .on_a11y_action(accesskit::Action::Click, voiceover_press(handler.clone()))
                    .on_click(move |event, window, cx| handler(event, window, cx));
                (Value::Button(button), ring)
            }
            None => (Value::Plain(body), None),
        };

        let reveal = self.on_toggle_reveal.clone().map(|handler| {
            let glyph = if self.revealed {
                IconName::EyeOff
            } else {
                IconName::Eye
            };
            IconButton::new("reveal", glyph, tr!("a11y.reveal_password"))
                .toggled(self.revealed)
                .on_click(move |event, window, cx| handler(event, window, cx))
        });
        let copy = self.on_copy.clone().map(|handler| {
            IconButton::new("copy", copy_glyph(self.copied), tr!("a11y.copy_password"))
                .on_click(move |event, window, cx| handler(event, window, cx))
        });

        // Without a value button the group says the secret is hidden.
        let says_hidden = hidden && copy.is_none();
        let group = counted(div(), "secret-group")
            .id(self.id)
            .role(Role::Group)
            .aria_label(self.label)
            .when(says_hidden, |this| {
                this.aria_description(tr!("a11y.hidden"))
            })
            .flex()
            .flex_none()
            .items_center()
            .gap(u(dimension::SPACE_4));
        let group = match &self.accessibility_id {
            Some(id) => id.apply(group),
            None => group,
        };
        Built {
            group,
            value,
            shown,
            ring,
            reveal,
            copy,
        }
    }

    /// Every element `render` builds, each read as the tests read it: the
    /// group, the value, the focus ring while it shows, the dots or the
    /// revealed text, and the buttons that reveal and copy. The group is
    /// read off the element `render` returns, so whatever is set on it on
    /// the way counts too.
    #[cfg(test)]
    pub(super) fn parts(
        &self,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<super::probe::ComponentPart> {
        use super::probe::part;
        let group = part("group", RenderOnce::render(self.clone(), window, cx));
        // Every field by name, so an element added to `Built` does not
        // compile until it is listed here. The group is read above.
        let Built {
            group: _,
            value,
            shown,
            ring,
            reveal,
            copy,
        } = self.clone().build(window, cx);
        let mut parts = vec![
            group,
            match value {
                Value::Button(value) => part("value", value),
                Value::Plain(value) => part("value", value),
            },
            match shown {
                Shown::Mask(text) => part("mask", text),
                Shown::Revealed(text) => part("revealed_text", text),
            },
        ];
        parts.extend(ring.map(|ring| part("ring", ring)));
        for (name, button) in [("reveal", reveal), ("copy", copy)] {
            if let Some(button) = button {
                let base = button.into_base(window, cx);
                parts.push(part(name, RenderOnce::render(base, window, cx)));
            }
        }
        parts
    }
}

/// The glyph of the copy button: a tick right after a copy.
fn copy_glyph(copied: CopyState) -> IconName {
    match copied {
        CopyState::Idle => IconName::Copy,
        CopyState::Flash | CopyState::Check => IconName::Check,
    }
}

/// Puts what the field builds in place: the value with what it shows and
/// its focus ring, then the buttons that reveal and copy.
fn assemble(built: Built) -> Stateful<Div> {
    let shown = match built.shown {
        Shown::Mask(text) => text.into_any_element(),
        Shown::Revealed(text) => text.into_any_element(),
    };
    let value = match built.value {
        Value::Button(value) => value.child(shown).children(built.ring).into_any_element(),
        Value::Plain(value) => value.child(shown).children(built.ring).into_any_element(),
    };
    built
        .group
        .child(value)
        .children(built.reveal)
        .children(built.copy)
}

impl RenderOnce for SecretField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        assemble(self.build(window, cx))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use gpui::{Modifiers, Pixels, TestAppContext, VisualTestContext};
    use gpui_component::IconNamed as _;

    use super::super::probe::{self, ComponentPart as Part};
    use super::*;
    use crate::ui::ds::tokens::LIGHT;

    /// A secret for the tests, with letters, digits and symbols.
    const SECRET: &str = "sp-test-7Qx!2026";

    fn text(value: &'static str) -> SharedString {
        SharedString::new_static(value)
    }

    fn secret(value: &str) -> Zeroizing<String> {
        Zeroizing::new(value.to_string())
    }

    /// A field with `value`, both handlers and the given state.
    fn field(value: &str, revealed: bool, assistive: bool) -> SecretField {
        SecretField::new("password", text("Password"))
            .value(secret(value))
            .revealed(revealed)
            .assistive(assistive)
            .on_copy(|_, _, _| {})
            .on_toggle_reveal(|_, _, _| {})
    }

    fn parts_of(cx: &mut TestAppContext, build: impl Fn() -> SecretField + 'static) -> Vec<Part> {
        probe::in_frame(cx, move |window, cx| build().parts(window, cx))
    }

    fn named<'a>(parts: &'a [Part], name: &str) -> Option<&'a Part> {
        parts.iter().find(|part| part.name == name)
    }

    /// The width of the value of a field drawn in a window of its own.
    fn value_width(cx: &mut TestAppContext, build: impl Fn() -> SecretField + 'static) -> Pixels {
        let cx = probe::window(cx, move |_, _| build().into_any_element());
        cx.debug_bounds("fp-secret-value")
            .expect("the value")
            .size
            .width
    }

    /// Hidden, a secret shows twelve dots, so a short one and a long one
    /// are drawn just as wide, and so is a field without a secret. Revealed,
    /// the long one is wider, so the width does follow what is drawn.
    #[gpui::test]
    fn a_hidden_secret_shows_twelve_dots_whatever_its_length(cx: &mut TestAppContext) {
        assert_eq!(MASK.chars().count(), 12);
        assert!(MASK.chars().all(|c| c == '•'), "{MASK}");
        let long = "x7!".repeat(13) + "a";
        assert_eq!(long.chars().count(), 40);
        let short_hidden = value_width(cx, || field("ab1!", false, false));
        let long_for_hidden = long.clone();
        let long_hidden = value_width(cx, move || field(&long_for_hidden, false, false));
        let none = value_width(cx, || SecretField::new("password", text("Password")));
        assert_eq!(short_hidden, long_hidden, "4 and 40 characters");
        assert_eq!(short_hidden, none, "a field without a secret");
        let long_for_revealed = long.clone();
        let long_revealed = value_width(cx, move || field(&long_for_revealed, true, false));
        assert!(
            long_revealed > long_hidden,
            "the revealed secret is drawn: {long_revealed:?}, hidden {long_hidden:?}"
        );
    }

    /// Revealed, a secret is set in groups of four, its letters in
    /// `text_primary`, its digits in `secret_digit` and everything else in
    /// `secret_symbol`; the space between groups is no character of the
    /// secret. The field draws it so, without a node.
    #[gpui::test]
    fn a_revealed_secret_is_grouped_in_fours_and_colored_by_class(cx: &mut TestAppContext) {
        let (shown, classes) = grouped(SECRET);
        assert_eq!(shown.as_str(), "sp-t est- 7Qx! 2026");
        let colors = &LIGHT;
        let painted: String = classes
            .iter()
            .map(|(range, class)| {
                let c = &shown[range.clone()];
                let color = class.color(colors);
                if color == colors.text_primary.hsla {
                    assert!(c.chars().all(char::is_alphabetic), "{c} as a letter");
                    'L'
                } else if color == colors.secret_digit.hsla {
                    assert!(c.chars().all(|c| c.is_ascii_digit()), "{c} as a digit");
                    'D'
                } else {
                    assert_eq!(color, colors.secret_symbol.hsla, "{c}");
                    'S'
                }
            })
            .collect();
        assert_eq!(
            painted, "LLSLLLLSDLLSDDDD",
            "s p - t e s t - 7 Q x ! 2 0 2 6"
        );
        assert_ne!(colors.secret_digit.hsla, colors.text_primary.hsla);
        assert_ne!(colors.secret_symbol.hsla, colors.text_primary.hsla);
        let (shown, _) = grouped("abcdefgh");
        assert_eq!(shown.as_str(), "abcd efgh", "no space after the last group");

        let parts = parts_of(cx, || field(SECRET, true, false));
        let revealed = named(&parts, "revealed_text").expect("the revealed text");
        assert_eq!((revealed.role, revealed.id.clone()), (None, None));
        assert!(named(&parts, "mask").is_none(), "no dots while revealed");
        let parts = parts_of(cx, || field(SECRET, false, false));
        assert!(named(&parts, "mask").is_some(), "the dots while hidden");
        assert!(named(&parts, "revealed_text").is_none());
    }

    /// The value is a button named "Copy password", described as hidden
    /// while it is and not once it is revealed, and VoiceOver can press it.
    /// The field is a group named after what it holds.
    #[gpui::test]
    fn the_value_button_is_named_copy_password_and_described_hidden(cx: &mut TestAppContext) {
        for revealed in [false, true] {
            let parts = parts_of(cx, move || field(SECRET, revealed, false));
            let group = named(&parts, "group").expect("the group");
            assert_eq!(group.role, Some(Role::Group));
            assert_eq!(group.node.label(), Some("Password"));
            assert_eq!(group.node.description(), None, "the value says it");
            let value = named(&parts, "value").expect("the value");
            assert_eq!(value.role, Some(Role::Button), "revealed: {revealed}");
            assert!(value.id.is_some(), "an id, so it reaches the tree");
            // The English of `a11y.copy_password` and `a11y.hidden`, written
            // out, so `no_unused_keys` fails if a literal takes their place.
            assert_eq!(value.node.label(), Some("Copy password"));
            let described = (!revealed).then_some("hidden");
            assert_eq!(value.node.description(), described, "revealed: {revealed}");
            assert!(value.node.supports_action(accesskit::Action::Click));
        }
        // A field without a secret has nothing to reveal and stays hidden.
        let parts = parts_of(cx, || {
            SecretField::new("password", text("Password"))
                .revealed(true)
                .on_copy(|_, _, _| {})
        });
        let value = named(&parts, "value").expect("the value");
        assert_eq!(value.node.description(), Some("hidden"));
    }

    /// The button that reveals the secret is named "Show password" and
    /// says whether it is on; a click runs the caller's handler.
    #[gpui::test]
    fn the_reveal_button_is_toggled(cx: &mut TestAppContext) {
        for (revealed, toggled) in [
            (false, accesskit::Toggled::False),
            (true, accesskit::Toggled::True),
        ] {
            let parts = parts_of(cx, move || field(SECRET, revealed, false));
            let reveal = named(&parts, "reveal").expect("the reveal button");
            assert_eq!(reveal.role, Some(Role::Button));
            assert_eq!(reveal.node.label(), Some("Show password"));
            assert_eq!(reveal.node.toggled(), Some(toggled), "revealed: {revealed}");
        }
        let parts = parts_of(cx, || {
            SecretField::new("password", text("Password")).value(secret(SECRET))
        });
        assert!(
            named(&parts, "reveal").is_none(),
            "no reveal button without its handler"
        );

        let toggles = Rc::new(Cell::new(0));
        let counted = toggles.clone();
        let cx = probe::window(cx, move |_, _| {
            let counted = counted.clone();
            SecretField::new("password", text("Password"))
                .value(secret(SECRET))
                .on_toggle_reveal(move |_, _, _| counted.set(counted.get() + 1))
                .into_any_element()
        });
        let value = cx.debug_bounds("fp-secret-value").expect("the value");
        // The reveal button is the only button, right after the value.
        let at = gpui::point(value.right() + px(14.), value.center().y);
        cx.simulate_click(at, Modifiers::default());
        assert_eq!(toggles.get(), 1);
    }

    /// Without a copy handler the field has no copy button, and its value
    /// is no button either: a plain box without an id or a role, which
    /// writes no node. The group then says that the secret is hidden.
    #[gpui::test]
    fn copy_without_a_handler_shows_no_button(cx: &mut TestAppContext) {
        let parts = parts_of(cx, || {
            SecretField::new("password", text("Password"))
                .value(secret(SECRET))
                .on_toggle_reveal(|_, _, _| {})
        });
        assert!(named(&parts, "copy").is_none(), "no copy button");
        let value = named(&parts, "value").expect("the value");
        assert_eq!((value.role, value.id.clone()), (None, None), "no button");
        assert!(!value.node.supports_action(accesskit::Action::Click));
        let group = named(&parts, "group").expect("the group");
        assert_eq!(group.node.description(), Some("hidden"));

        let parts = parts_of(cx, || field(SECRET, false, false));
        let copy = named(&parts, "copy").expect("a copy button with a handler");
        assert_eq!(copy.node.label(), Some("Copy password"));
        assert!(copy.node.supports_action(accesskit::Action::Click));
    }

    /// A click on the value and a click on the copy button each run the
    /// caller's handler once.
    #[gpui::test]
    fn a_click_on_the_value_or_the_copy_button_runs_the_handler(cx: &mut TestAppContext) {
        let copies = Rc::new(Cell::new(0));
        let counted = copies.clone();
        let cx = probe::window(cx, move |_, _| {
            let counted = counted.clone();
            SecretField::new("password", text("Password"))
                .value(secret(SECRET))
                .on_copy(move |_, _, _| counted.set(counted.get() + 1))
                .into_any_element()
        });
        let value = cx.debug_bounds("fp-secret-value").expect("the value");
        cx.simulate_click(value.center(), Modifiers::default());
        assert_eq!(copies.get(), 1, "the value");
        // Without a reveal button the copy button follows the value.
        let at = gpui::point(value.right() + px(14.), value.center().y);
        cx.simulate_click(at, Modifiers::default());
        assert_eq!(copies.get(), 2, "the copy button");
    }

    /// The value is the first tab stop of the field, before the buttons
    /// next to it, and shows the ring of every component for focus from the
    /// keyboard and none for a click. Return and Space copy, as a click
    /// does.
    #[gpui::test]
    fn the_value_copies_from_the_keyboard_and_shows_the_focus_ring(cx: &mut TestAppContext) {
        let window = probe::window(cx, |_, _| field(SECRET, false, false).into_any_element());
        probe::assert_keyboard_focus_ring(window, "fp-secret-value");

        let copies = Rc::new(Cell::new(0));
        let counted = copies.clone();
        let cx = probe::window(cx, move |_, _| {
            let counted = counted.clone();
            SecretField::new("password", text("Password"))
                .value(secret(SECRET))
                .on_copy(move |_, _, _| counted.set(counted.get() + 1))
                .into_any_element()
        });
        cx.update(|window, cx| window.focus_next(cx));
        // The focused element takes its keys from the next frame on.
        cx.update(|window, cx| window.draw(cx).clear(cx));
        // A key press is a key down and a key up, which makes the click.
        for (key, copied) in [("enter", 1), ("space", 2)] {
            let keystroke = gpui::Keystroke::parse(key).expect("a key");
            cx.simulate_event(gpui::KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            cx.simulate_event(gpui::KeyUpEvent { keystroke });
            assert_eq!(copies.get(), copied, "{key} copies");
        }
    }

    /// The surface painted in the place of the value. Painting snaps to
    /// device pixels, so a quad lies within a pixel of the value's bounds.
    fn value_surface(cx: &mut VisualTestContext) -> Option<Hsla> {
        let bounds = cx.debug_bounds("fp-secret-value").expect("the value");
        let near = move |quad: gpui::Bounds<Pixels>| {
            [
                (quad.left(), bounds.left()),
                (quad.top(), bounds.top()),
                (quad.right(), bounds.right()),
                (quad.bottom(), bounds.bottom()),
            ]
            .into_iter()
            .all(|(a, b)| (a - b).abs() <= px(1.))
        };
        cx.update(|window, _| {
            let scale = window.scale_factor();
            window
                .painted_quads()
                .into_iter()
                .filter(|quad| near(quad.bounds.map(|value| px(value.0 / scale))))
                .filter_map(|quad| quad.background.as_solid())
                .find(|color| !color.is_transparent())
        })
    }

    /// Right after a copy the value flashes in `bg_selected`, and the copy
    /// button shows a tick until the caller sets the field back to idle.
    #[gpui::test]
    fn a_copy_flashes_the_value_and_ticks_the_copy_button(cx: &mut TestAppContext) {
        for copied in CopyState::ALL {
            let cx = probe::window(cx, move |_, _| {
                field(SECRET, false, false)
                    .copied(copied)
                    .into_any_element()
            });
            // The test window starts with the pointer over the value.
            cx.simulate_mouse_move(gpui::point(px(1.), px(400.)), None, Modifiers::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let flash = (copied == CopyState::Flash).then_some(LIGHT.bg_selected.hsla);
            assert_eq!(value_surface(cx), flash, "{copied:?}");
            let tick = copied != CopyState::Idle;
            assert_eq!(
                copy_glyph(copied).path() == IconName::Check.path(),
                tick,
                "{copied:?}"
            );
        }
    }

    #[test]
    #[should_panic(expected = "a secret field needs a name")]
    fn a_secret_field_needs_a_name_in_every_build() {
        let _ = SecretField::new("password", SharedString::default());
    }

    /// The field never copies: it calls the handler it was given. Neither it
    /// nor the one-time code ring names the clipboard, the pasteboard or a
    /// method that writes to either, in any spelling. The files are read
    /// through `code_only`, so comments and their tests do not count; a call
    /// made through a macro or a function in another file is past it.
    #[test]
    fn the_secret_field_never_touches_the_clipboard() {
        let mut found = Vec::new();
        for file in ["secret_field.rs", "otp.rs"] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/ui/ds/components")
                .join(file);
            let Ok(source) = std::fs::read_to_string(&path) else {
                assert_eq!(file, "otp.rs", "{file} is there");
                continue;
            };
            let code = probe::code_only(&source).to_lowercase();
            for (at, line) in code.lines().enumerate() {
                for word in ["clipboard", "pasteboard", "write_text", "write_secret"] {
                    if line.contains(word) {
                        found.push(format!("{file}:{}: {}", at + 1, line.trim()));
                    }
                }
            }
        }
        assert!(
            found.is_empty(),
            "the copy handler copies, the component does not:\n{}",
            found.join("\n")
        );
    }
}

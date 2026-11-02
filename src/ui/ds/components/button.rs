//! The button, in six kinds and four sizes, and the icon button.
//!
//! The look is drawn here on `gpui_base::Button`, which brings focus, Tab,
//! Space and Return, and the accessibility role. Two things are added on top:
//!
//! - VoiceOver's press calls the handler directly. Without it, GPUI answers a
//!   press by clicking the middle of the button with a synthetic mouse, which
//!   misses when something covers the button.
//! - A disabled button has no handler at all, loses its color, and says "not
//!   available" in its description, because GPUI has no disabled flag for
//!   assistive technology yet.
//!
//! A button that is held down fades to [`PRESSED_OPACITY`] instead of
//! shrinking: GPUI cannot scale an element. Focus from the keyboard shows a
//! ring outside the button, 2 px in `focus_ring` with 1 px of space; it takes
//! no room, and a click does not show it.

use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, Hsla, InteractiveElement as _, IntoElement,
    Keystroke, ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled, Window, accesskit, div, prelude::FluentBuilder as _, px,
};
use gpui_component::Icon;
use gpui_component::spinner::Spinner;

use super::AccessibilityId;
use super::keycap::{Keycap, keys_text, parse_keys};
use crate::tr;
use crate::ui::ds::ActiveTokens as _;
use crate::ui::ds::tokens::{ColorSet, dimension};
use crate::ui::ds::typography::{TextStyle, TextStyled as _, u};

/// Opacity of a button while it is held down.
pub const PRESSED_OPACITY: f32 = 0.72;

/// How long a loading button shows its label alone before the spinner
/// joins it, so a quick action does not flash one.
pub const SPINNER_DELAY: Duration = Duration::from_millis(400);

/// Size of the glyph in an icon button, in points.
const ICON_SIZE: f32 = 16.0;

/// The key string of the default button.
const RETURN: &str = "enter";

/// What a button looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    /// The one copper surface of a view: the action the view is for.
    Primary,
    Secondary,
    /// No surface until the pointer is over it.
    Quiet,
    /// A verb inside a row, in copper text.
    Ghost,
    /// Anything that cannot be undone. Never the default button.
    Danger,
    /// The copper capsule of the lock screen and onboarding.
    Large,
}

impl ButtonKind {
    pub const ALL: [ButtonKind; 6] = [
        Self::Primary,
        Self::Secondary,
        Self::Quiet,
        Self::Ghost,
        Self::Danger,
        Self::Large,
    ];

    /// Whether a button of this kind is the view's copper surface.
    pub fn is_copper(self) -> bool {
        matches!(self, Self::Primary | Self::Large)
    }

    /// Surface, surface under the pointer, text.
    fn colors(self, colors: &ColorSet) -> (Option<Hsla>, Hsla, Hsla) {
        match self {
            Self::Primary | Self::Large => (
                Some(colors.button_primary_bg.hsla),
                colors.button_primary_hover.hsla,
                colors.button_primary_text.hsla,
            ),
            Self::Secondary => (
                Some(colors.button_secondary_bg.hsla),
                colors.button_secondary_hover.hsla,
                colors.button_secondary_text.hsla,
            ),
            Self::Quiet => (
                None,
                colors.button_quiet_hover_bg.hsla,
                colors.button_quiet_text.hsla,
            ),
            Self::Ghost => (
                None,
                colors.button_ghost_hover_bg.hsla,
                colors.button_ghost_text.hsla,
            ),
            Self::Danger => (
                Some(colors.button_danger_bg.hsla),
                colors.button_danger_hover.hsla,
                colors.button_danger_text.hsla,
            ),
        }
    }

    /// A disabled button keeps its shape and loses its color.
    fn disabled_colors(self, colors: &ColorSet) -> (Option<Hsla>, Hsla) {
        let surface = match self {
            Self::Quiet | Self::Ghost => None,
            _ => Some(colors.button_disabled_bg.hsla),
        };
        (surface, colors.button_disabled_text.hsla)
    }

    fn default_size(self) -> ButtonSize {
        match self {
            Self::Large => ButtonSize::Capsule44,
            _ => ButtonSize::M28,
        }
    }
}

/// The height of a button, in points at text size 13.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonSize {
    S24,
    M28,
    L32,
    /// Fully rounded at any text size.
    Capsule44,
}

impl ButtonSize {
    pub const ALL: [ButtonSize; 4] = [Self::S24, Self::M28, Self::L32, Self::Capsule44];

    pub fn height(self) -> f32 {
        match self {
            Self::S24 => dimension::HEIGHT_CONTROL_SM,
            Self::M28 => dimension::HEIGHT_CONTROL,
            Self::L32 => dimension::HEIGHT_CONTROL_LG,
            Self::Capsule44 => dimension::HEIGHT_CAPSULE,
        }
    }

    fn padding_x(self) -> f32 {
        match self {
            Self::S24 => dimension::SPACE_8,
            Self::M28 => dimension::SPACE_12,
            Self::L32 => dimension::SPACE_16,
            Self::Capsule44 => dimension::SPACE_24,
        }
    }
}

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// What a button shows, and the name VoiceOver reads for it.
enum Content {
    /// The label is the name.
    Label(SharedString),
    /// The glyph shows nothing VoiceOver could read, so the name is given.
    Icon { icon: Box<Icon>, name: SharedString },
}

impl Content {
    fn name(&self) -> &SharedString {
        match self {
            Self::Label(label) => label,
            Self::Icon { name, .. } => name,
        }
    }
}

/// A button with a label. Its label is also its name for VoiceOver, so it
/// should say what happens and to what ("Sign in to NHP, client 100"), and
/// it comes from a text key or from the caller, never from a literal here.
/// The constructors panic on an empty label or name.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    kind: ButtonKind,
    size: ButtonSize,
    content: Content,
    keys: Vec<Keystroke>,
    default: bool,
    disabled: bool,
    loading: bool,
    description: Option<SharedString>,
    accessibility_id: Option<AccessibilityId>,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, kind: ButtonKind, label: impl Into<SharedString>) -> Self {
        Self::with_content(id.into(), kind, Content::Label(label.into()))
    }

    pub fn primary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self::new(id, ButtonKind::Primary, label)
    }

    pub fn secondary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self::new(id, ButtonKind::Secondary, label)
    }

    pub fn quiet(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self::new(id, ButtonKind::Quiet, label)
    }

    pub fn ghost(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self::new(id, ButtonKind::Ghost, label)
    }

    pub fn danger(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self::new(id, ButtonKind::Danger, label)
    }

    pub fn large(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self::new(id, ButtonKind::Large, label)
    }

    fn with_content(id: ElementId, kind: ButtonKind, content: Content) -> Self {
        // VoiceOver reads a button by its name; one without is a mistake in
        // the code, in a release build too.
        assert!(!content.name().is_empty(), "a button needs a name");
        Self {
            id,
            kind,
            size: kind.default_size(),
            content,
            keys: Vec::new(),
            default: false,
            disabled: false,
            loading: false,
            description: None,
            accessibility_id: None,
            on_click: None,
        }
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    /// Shows the keys that trigger the button as keycaps after its label,
    /// and hands them to VoiceOver in `aria_keyshortcuts`. `keys` is a GPUI
    /// key string such as `cmd-shift-c`. The binding itself lives elsewhere.
    pub fn shortcut(mut self, keys: &str) -> Self {
        self.keys = parse_keys(keys);
        self
    }

    /// Marks the button Return presses: VoiceOver announces it as the
    /// default button, and unless the button has a shortcut of its own it
    /// shows ⏎. A danger button can never be it: Return never triggers
    /// something that cannot be undone.
    pub fn default_button(mut self) -> Self {
        debug_assert!(
            self.kind != ButtonKind::Danger,
            "a danger button is never the default button"
        );
        if self.kind == ButtonKind::Danger {
            return self;
        }
        self.default = true;
        if self.keys.is_empty() {
            self.keys = parse_keys(RETURN);
        }
        self
    }

    /// A disabled button has no handler and no color, and its description
    /// says it is not available.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// While loading, the label stays, a click does nothing, and after
    /// [`SPINNER_DELAY`] a spinner appears next to the label.
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    /// What VoiceOver reads after the name, such as "Production system".
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn accessibility_id(mut self, id: AccessibilityId) -> Self {
        self.accessibility_id = Some(id);
        self
    }

    /// Runs for a click, for Space or Return while the button has focus,
    /// and for VoiceOver's press.
    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    /// Whether a click reaches the handler.
    fn takes_clicks(&self) -> bool {
        !self.disabled && !self.loading
    }

    /// The unstyled button with this button's look, role, name and handlers.
    pub(in crate::ui::ds) fn into_base(
        self,
        window: &mut Window,
        cx: &mut App,
    ) -> gpui_base::Button {
        let colors = cx.ds().colors;
        #[cfg(debug_assertions)]
        if self.kind.is_copper() && !self.disabled {
            super::copper::note(window, cx);
        }

        let (surface, hover, text) = if self.disabled {
            let (surface, text) = self.kind.disabled_colors(colors);
            (surface, None, text)
        } else {
            let (surface, hover, text) = self.kind.colors(colors);
            (surface, Some(hover), text)
        };
        let description = if self.disabled {
            Some(tr!("a11y.not_available"))
        } else if self.loading {
            Some(tr!("a11y.in_progress"))
        } else {
            self.description.clone()
        };
        let spinner = self.loading && spinner_due(&self.id, window, cx);
        let handler = self.on_click.clone().filter(|_| self.takes_clicks());
        let height = self.size.height();
        let capsule = self.size == ButtonSize::Capsule44;
        let icon_only = matches!(self.content, Content::Icon { .. });
        let name = self.content.name().clone();
        let keys = (!self.keys.is_empty()).then(|| keys_text(&self.keys));
        let keycaps_on_copper = self.kind.is_copper() && !self.disabled;
        let keycaps: Vec<Keycap> = self
            .keys
            .iter()
            .map(|keystroke| Keycap::new(keystroke).on_accent(keycaps_on_copper))
            .collect();
        let focus = window
            .use_keyed_state((self.id.clone(), "focus"), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let ring = !self.disabled && focus.is_focused(window) && window.last_input_was_keyboard();

        let content: AnyElement = match self.content {
            Content::Label(label) => label.into_any_element(),
            Content::Icon { icon, .. } => (*icon)
                .size(u(ICON_SIZE))
                .text_color(text)
                .into_any_element(),
        };

        gpui_base::Button::new(self.id)
            .role(if self.default {
                Role::DefaultButton
            } else {
                Role::Button
            })
            .accessibility_label(name)
            .disabled(self.disabled)
            .track_focus(&focus)
            .when_some(self.accessibility_id, |this, id| this.accessibility_id(id))
            .when_some(description, |this, text| this.aria_description(text))
            .when_some(keys, |this, keys| this.aria_keyshortcuts(keys))
            .when_some(handler, |this, handler| {
                this.on_a11y_action(accesskit::Action::Click, voiceover_press(handler.clone()))
                    .on_click(move |event, window, cx| handler(event, window, cx))
            })
            .relative()
            .h(u(height))
            .flex_none()
            .map(|this| {
                if icon_only {
                    this.w(u(height))
                } else {
                    this.px(u(self.size.padding_x())).gap(u(dimension::SPACE_8))
                }
            })
            .map(|this| {
                if capsule {
                    this.rounded_full()
                } else {
                    this.rounded(px(dimension::RADIUS_MD))
                }
            })
            .text_style(TextStyle::Headline)
            .text_color(text)
            .when_some(surface, |this, surface| this.bg(surface))
            .when_some(hover, |this, hover| {
                this.cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .active(|style| style.opacity(PRESSED_OPACITY))
            })
            .when(spinner, |this| this.child(Spinner::new().color(text)))
            .child(content)
            .children(keycaps)
            .when(ring, |this| {
                this.child(focus_ring(colors.focus_ring.hsla, capsule))
            })
    }
}

/// The keyboard focus ring: outside the button, so it takes no room, and
/// with a gap that shows whatever is behind the button.
fn focus_ring(color: Hsla, capsule: bool) -> Div {
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
        .map(|this| {
            if capsule {
                this.rounded_full()
            } else {
                this.rounded(px(dimension::RADIUS_MD) + outset)
            }
        })
}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.into_base(window, cx)
    }
}

/// What VoiceOver's press does: the same as a click from the keyboard.
fn voiceover_press(
    handler: ClickHandler,
) -> impl FnMut(Option<&accesskit::ActionData>, &mut Window, &mut App) + 'static {
    move |_, window, cx| handler(&ClickEvent::default(), window, cx)
}

/// Whether the spinner of a button that has been loading since `since` is
/// due at `now`.
fn shows_spinner(since: Instant, now: Instant) -> bool {
    now.saturating_duration_since(since) >= SPINNER_DELAY
}

/// Remembers when the button started loading, and asks for frames until
/// the spinner is due. The memory goes away with the first frame the button
/// is not loading.
fn spinner_due(id: &ElementId, window: &mut Window, cx: &mut App) -> bool {
    let now = Instant::now();
    let since = window.use_keyed_state((id.clone(), "loading-since"), cx, move |_, _| now);
    let due = shows_spinner(*since.read(cx), now);
    if !due {
        window.request_animation_frame();
    }
    due
}

/// A button that shows a glyph instead of a label, 28 by 28. VoiceOver
/// cannot read a glyph, so there is no way to build one without a name.
#[derive(IntoElement)]
pub struct IconButton(Button);

impl IconButton {
    /// `name` says what the button does, such as `tr!("a11y.copy_password")`.
    pub fn new(
        id: impl Into<ElementId>,
        icon: impl Into<Icon>,
        name: impl Into<SharedString>,
    ) -> Self {
        let content = Content::Icon {
            icon: Box::new(icon.into()),
            name: name.into(),
        };
        Self(Button::with_content(id.into(), ButtonKind::Quiet, content))
    }

    pub fn disabled(self, disabled: bool) -> Self {
        Self(self.0.disabled(disabled))
    }

    pub fn description(self, description: impl Into<SharedString>) -> Self {
        Self(self.0.description(description))
    }

    pub fn accessibility_id(self, id: AccessibilityId) -> Self {
        Self(self.0.accessibility_id(id))
    }

    pub fn on_click(self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        Self(self.0.on_click(handler))
    }

    #[cfg(test)]
    pub(in crate::ui::ds) fn into_base(
        self,
        window: &mut Window,
        cx: &mut App,
    ) -> gpui_base::Button {
        self.0.into_base(window, cx)
    }
}

impl RenderOnce for IconButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{Modifiers, MouseButton, TestAppContext, VisualTestContext, point};
    use gpui_component::IconName;

    #[cfg(debug_assertions)]
    use super::super::copper;
    use super::super::probe;
    use super::*;
    use crate::ui::ds::tokens::LIGHT;

    fn label(text: &'static str) -> SharedString {
        SharedString::new_static(text)
    }

    /// The node a button writes for assistive technology.
    fn node_of(
        cx: &mut TestAppContext,
        build: impl Fn() -> Button + 'static,
    ) -> (Option<Role>, accesskit::Node) {
        probe::in_frame(cx, move |window, cx| {
            let base = build().into_base(window, cx);
            probe::node(RenderOnce::render(base, window, cx))
        })
    }

    #[gpui::test]
    fn button_has_role_and_name(cx: &mut TestAppContext) {
        for kind in ButtonKind::ALL {
            let (role, node) = node_of(cx, move || {
                Button::new("open", kind, label("Open website"))
                    .accessibility_id(AccessibilityId::new("test", "open", 0))
                    .on_click(|_, _, _| {})
            });
            assert_eq!(role, Some(Role::Button), "{kind:?}");
            assert_eq!(node.label(), Some("Open website"), "{kind:?}");
            assert_eq!(node.author_id(), Some("fp.test.open.0"), "{kind:?}");
            assert_eq!(node.description(), None, "{kind:?}");
        }
        let (_, node) = node_of(cx, || {
            Button::primary("sign-in", label("Sign in to NHP, client 100"))
                .description(label("Production system"))
        });
        assert_eq!(node.description(), Some("Production system"));
    }

    #[gpui::test]
    fn return_button_is_the_default_button(cx: &mut TestAppContext) {
        let (role, node) = node_of(cx, || {
            Button::primary("sign-in", label("Sign in")).default_button()
        });
        assert_eq!(role, Some(Role::DefaultButton));
        assert_eq!(node.label(), Some("Sign in"));
        assert_eq!(
            node.keyboard_shortcut(),
            Some("⏎"),
            "it shows and announces ⏎"
        );
        let (role, _) = node_of(cx, || Button::secondary("cancel", label("Cancel")));
        assert_eq!(role, Some(Role::Button), "only the button Return presses");
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "a danger button is never the default button")]
    fn danger_is_never_the_default_button() {
        let _ = Button::danger("delete", label("Delete")).default_button();
    }

    /// VoiceOver reads a button by its name, so a button without one is a
    /// mistake in every build, not only in a debug build.
    #[test]
    #[should_panic(expected = "a button needs a name")]
    fn a_button_needs_a_name_in_every_build() {
        let _ = Button::secondary("unnamed", SharedString::default());
    }

    #[test]
    #[should_panic(expected = "a button needs a name")]
    fn an_icon_button_needs_a_name_in_every_build() {
        let _ = IconButton::new("unnamed", IconName::Copy, SharedString::default());
    }

    /// Clicks the middle of the first button in a window that holds only
    /// `button`, and says how often the handler ran.
    fn clicks_reaching(
        cx: &mut TestAppContext,
        button: impl Fn(Rc<Cell<usize>>) -> Button + 'static,
    ) -> usize {
        let clicks = Rc::new(Cell::new(0));
        let counted = clicks.clone();
        let cx: &mut VisualTestContext =
            probe::window(cx, move |_, _| button(counted.clone()).into_any_element());
        cx.simulate_click(point(px(10.), px(10.)), Modifiers::default());
        clicks.get()
    }

    fn counting(clicks: Rc<Cell<usize>>) -> impl Fn(&ClickEvent, &mut Window, &mut App) {
        move |_, _, _| clicks.set(clicks.get() + 1)
    }

    #[gpui::test]
    fn disabled_button_has_no_handler_and_says_so(cx: &mut TestAppContext) {
        assert_eq!(
            clicks_reaching(cx, |clicks| Button::primary("save", label("Save"))
                .on_click(counting(clicks))),
            1,
            "an enabled button takes the click"
        );
        assert_eq!(
            clicks_reaching(cx, |clicks| {
                Button::primary("save", label("Save"))
                    .disabled(true)
                    .on_click(counting(clicks))
            }),
            0
        );

        let (role, node) = node_of(cx, || {
            Button::primary("save", label("Save"))
                .disabled(true)
                .on_click(|_, _, _| {})
        });
        assert_eq!(role, Some(Role::Button));
        assert_eq!(node.label(), Some("Save"));
        assert!(!node.supports_action(accesskit::Action::Click));
        // The English of `a11y.not_available`, written out instead of read
        // with `tr!`: the button stays the only code that reads the key, so
        // `no_unused_keys` fails if a literal ever takes the key's place.
        assert_eq!(node.description(), Some("not available"));

        // It loses its color: no copper, the disabled surface instead.
        let cx = probe::window(cx, |_, _| {
            Button::primary("save", label("Save"))
                .disabled(true)
                .into_any_element()
        });
        let quads = cx.update(|window, _| window.painted_quads());
        let surfaces: Vec<Hsla> = quads
            .iter()
            .filter_map(|quad| quad.background.as_solid())
            .collect();
        assert!(
            surfaces.contains(&LIGHT.button_disabled_bg.hsla),
            "{surfaces:?}"
        );
        assert!(
            !surfaces.contains(&LIGHT.button_primary_bg.hsla),
            "{surfaces:?}"
        );
        #[cfg(debug_assertions)]
        assert_eq!(cx.update(|window, cx| copper::surfaces(window, cx)), 0);
    }

    #[gpui::test]
    fn loading_keeps_the_label_and_takes_no_click(cx: &mut TestAppContext) {
        assert_eq!(
            clicks_reaching(cx, |clicks| {
                Button::primary("save", label("Save"))
                    .loading(true)
                    .on_click(counting(clicks))
            }),
            0
        );
        let (_, node) = node_of(cx, || Button::primary("save", label("Save")).loading(true));
        assert_eq!(node.label(), Some("Save"));
        // The English of `a11y.in_progress`, written out for the same reason.
        assert_eq!(node.description(), Some("in progress"));

        let start = Instant::now();
        assert!(!shows_spinner(start, start));
        assert!(!shows_spinner(
            start,
            start + SPINNER_DELAY - Duration::from_millis(1)
        ));
        assert!(shows_spinner(start, start + SPINNER_DELAY));
    }

    #[gpui::test]
    fn icon_button_requires_a_name(cx: &mut TestAppContext) {
        // There is no constructor without a name; the name is what VoiceOver
        // reads, the glyph adds nothing to it.
        let (role, node) = probe::in_frame(cx, |window, cx| {
            let button = IconButton::new("copy", IconName::Copy, label("Copy password"))
                .on_click(|_, _, _| {})
                .into_base(window, cx);
            probe::node(RenderOnce::render(button, window, cx))
        });
        assert_eq!(role, Some(Role::Button));
        assert_eq!(node.label(), Some("Copy password"));
        assert!(node.supports_action(accesskit::Action::Click));
    }

    /// Where `into_base` hands the click handler to the base button: the
    /// code from `.when_some(handler,` to the end of its closure, without
    /// whitespace.
    fn handler_registration() -> String {
        let source = include_str!("button.rs");
        let end = source
            .find("#[cfg(test)]\nmod tests")
            .expect("the test module");
        let code: String = source[..end].split_whitespace().collect();
        let start = code
            .find(".when_some(handler,")
            .expect("into_base hands the handler to the base button");
        let length = code[start..].find("})").expect("the end of the closure");
        code[start..start + length].to_string()
    }

    /// VoiceOver's press reaches the handler. The test platform never turns
    /// accessibility on, so it cannot deliver a press, and GPUI lists the
    /// press for any element with a click handler, so the node alone cannot
    /// show that the button registered its own. The test checks the three
    /// parts it is made of instead.
    #[gpui::test]
    fn button_click_reaches_voiceover(cx: &mut TestAppContext) {
        // VoiceOver is offered the press...
        let (_, node) = node_of(cx, || {
            Button::secondary("lock", label("Lock")).on_click(|_, _, _| {})
        });
        assert!(node.supports_action(accesskit::Action::Click));
        assert!(node.supports_action(accesskit::Action::Focus));

        // ...the handler a click gets is registered for the press too, or
        // GPUI answers the press with a synthetic click in the middle of the
        // button, which misses when something covers it...
        let registration = handler_registration();
        assert!(
            registration.contains(
                ".on_a11y_action(accesskit::Action::Click,voiceover_press(handler.clone()))"
            ),
            "VoiceOver's press is not registered with the click handler: {registration}"
        );
        assert!(registration.contains(".on_click("), "{registration}");

        // ...and the press runs the handler, as a click from the keyboard.
        let pressed = probe::in_frame(cx, |window, cx| {
            let pressed = Rc::new(Cell::new(0));
            let counted = pressed.clone();
            let handler: ClickHandler = Rc::new(move |event, _, _| {
                assert!(matches!(event, ClickEvent::Keyboard(_)));
                counted.set(counted.get() + 1);
            });
            voiceover_press(handler)(None, window, cx);
            pressed.get()
        });
        assert_eq!(pressed, 1);
    }

    /// How many copper surfaces a window with `buttons` drew. The count
    /// exists in debug builds only.
    #[cfg(debug_assertions)]
    fn copper_drawn(
        cx: &mut TestAppContext,
        buttons: impl Fn() -> Vec<AnyElement> + 'static,
    ) -> usize {
        let cx = probe::window(cx, move |_, _| {
            div().flex().children(buttons()).into_any_element()
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| copper::surfaces(window, cx))
    }

    /// A view has at most one copper surface. The counter sees every copper
    /// button a window draws, so a view with a second one fails here.
    #[gpui::test]
    #[cfg(debug_assertions)]
    fn one_copper_per_view(cx: &mut TestAppContext) {
        let drawn = copper_drawn(cx, || {
            vec![
                Button::primary("sign-in", label("Sign in"))
                    .default_button()
                    .into_any_element(),
                Button::secondary("other", label("Other vault")).into_any_element(),
                Button::quiet("why", label("Why?")).into_any_element(),
                Button::ghost("open", label("Open")).into_any_element(),
                Button::danger("forget", label("Forget")).into_any_element(),
                IconButton::new("copy", IconName::Copy, label("Copy password")).into_any_element(),
            ]
        });
        assert_eq!(drawn, 1, "one copper surface per view");
    }

    #[gpui::test]
    #[cfg(debug_assertions)]
    fn the_copper_counter_sees_both_copper_kinds(cx: &mut TestAppContext) {
        let drawn = copper_drawn(cx, || {
            vec![
                Button::primary("a", label("A")).into_any_element(),
                Button::large("b", label("B")).into_any_element(),
            ]
        });
        assert_eq!(drawn, 2);
        let drawn = copper_drawn(cx, || {
            vec![
                Button::primary("a", label("A"))
                    .disabled(true)
                    .into_any_element(),
                Button::secondary("b", label("B")).into_any_element(),
            ]
        });
        assert_eq!(drawn, 0, "a disabled button has no color");
    }

    /// Held down, a button keeps its size and fades.
    #[gpui::test]
    fn pressed_uses_opacity_not_scale(cx: &mut TestAppContext) {
        let cx = probe::window(cx, |_, _| {
            Button::primary("save", label("Save"))
                .on_click(|_, _, _| {})
                .into_any_element()
        });
        // The pointer is over the button while it is held, so the surface
        // is the hover color by then.
        let copper = [
            LIGHT.button_primary_bg.hsla,
            LIGHT.button_primary_hover.hsla,
        ];
        let surface = |cx: &mut VisualTestContext| {
            cx.update(|window, _| {
                window
                    .painted_quads()
                    .into_iter()
                    .find(|quad| {
                        quad.background
                            .as_solid()
                            .is_some_and(|color| copper.contains(&Hsla { a: 1.0, ..color }))
                    })
                    .map(|quad| (quad.bounds, quad.background.as_solid().unwrap().a))
                    .expect("the copper surface")
            })
        };
        let (bounds, alpha) = surface(cx);
        assert_eq!(alpha, 1.0);
        let at = point(px(10.), px(10.));
        cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let (pressed_bounds, pressed_alpha) = surface(cx);
        assert_eq!(
            pressed_bounds, bounds,
            "the same size and place while pressed"
        );
        assert!(
            (pressed_alpha - PRESSED_OPACITY).abs() < 1e-6,
            "{pressed_alpha}"
        );
        cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    }

    /// Tab shows the ring around the button, 1 px away and 2 px wide; a
    /// click on a focused button does not.
    #[gpui::test]
    fn the_focus_ring_shows_for_the_keyboard_only(cx: &mut TestAppContext) {
        let cx = probe::window(cx, |_, _| {
            div()
                .p(px(8.))
                .child(
                    div()
                        .debug_selector(|| "fp-test-button".into())
                        .child(Button::secondary("save", label("Save")).on_click(|_, _, _| {})),
                )
                .into_any_element()
        });
        assert_eq!(
            cx.debug_bounds("fp-focus-ring"),
            None,
            "nothing has focus yet"
        );
        // With nothing focused yet, a key press reaches no handler, so the
        // first tab stop is taken directly after it, as Tab would.
        cx.simulate_keystrokes("tab");
        cx.update(|window, cx| {
            window.focus_next(cx);
            assert!(window.last_input_was_keyboard());
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let button = cx.debug_bounds("fp-test-button").expect("the button");
        let ring = cx.debug_bounds("fp-focus-ring").expect("a ring after Tab");
        assert_eq!(ring, button.dilate(px(3.)));

        let middle = button.center();
        cx.simulate_click(middle, Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(
            cx.debug_bounds("fp-focus-ring"),
            None,
            "a click shows no ring"
        );
    }

    #[gpui::test]
    fn sizes_are_the_design_heights(cx: &mut TestAppContext) {
        let _lock = crate::ui::ds::tests::lock_mode();
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::ui::ds::apply(
                cx,
                crate::ui::ds::Appearance {
                    choice: crate::app::settings::ThemeChoice::Light,
                    prefs: crate::ui::ds::A11yPrefs::default(),
                    text_size: crate::ui::ds::tokens::TEXT_SIZE_STANDARD,
                },
            );
        });
        for size in ButtonSize::ALL {
            let cx = probe::window(cx, move |_, _| {
                div()
                    .debug_selector(|| "fp-test-button".into())
                    .child(Button::secondary("b", label("B")).size(size))
                    .into_any_element()
            });
            let bounds = cx.debug_bounds("fp-test-button").expect("the button");
            assert_eq!(bounds.size.height, px(size.height()), "{size:?}");
        }
    }
}

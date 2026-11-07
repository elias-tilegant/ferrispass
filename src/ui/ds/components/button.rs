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
//! shrinking: GPUI cannot scale an element. Focus from the keyboard shows the
//! ring of every component outside the button, 2 px in `focus_ring` with
//! 1 px of space; it takes no room, and a click does not show it.

use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AnyElement, App, ClickEvent, ElementId, Hsla, InteractiveElement as _, IntoElement, Keystroke,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _, Styled,
    Window, accesskit, div, prelude::FluentBuilder as _, px,
};
use gpui_component::Icon;
use gpui_component::spinner::Spinner;

use self::default_role::DefaultRole;
use super::AccessibilityId;
use super::focus_ring::{Outline, keyboard_focus_ring};
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

pub(super) type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

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
    default: DefaultRole,
    disabled: bool,
    loading: bool,
    description: Option<SharedString>,
    toggled: Option<bool>,
    expanded: Option<bool>,
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
            default: DefaultRole::default(),
            disabled: false,
            loading: false,
            description: None,
            toggled: None,
            expanded: None,
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

    /// Marks the button Return presses: its role is `DefaultButton`, and
    /// unless the button has a shortcut of its own it shows ⏎ and hands it
    /// to `aria_keyshortcuts`. It does not bind Return; like the keys of
    /// [`Button::shortcut`], the binding lives with the screen. The macOS
    /// adapter of AccessKit in `Cargo.lock` (0.26.3) hands the role to
    /// VoiceOver as a plain button and does not pass `aria_keyshortcuts`
    /// on, so VoiceOver reads the default button like any other today.
    ///
    /// A danger button can never be it: Return never triggers something
    /// that cannot be undone. Asking for it fails in a debug build and
    /// leaves the button as it was in a release build.
    pub fn default_button(self) -> Self {
        debug_check::danger_is_never_the_default(self.kind);
        self.make_default()
    }

    /// A disabled button has no handler and no color, and its description
    /// says it is not available.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// While loading, the label stays, a click does nothing, the pointer
    /// gets no hand, no hover surface and no fade, and after
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

        // Only a button that takes a click shows the pointing hand, its
        // hover surface and the fade while held; a loading one keeps its
        // colors but answers the pointer no more than a disabled one.
        let (surface, hover, text) = if self.disabled {
            let (surface, text) = self.kind.disabled_colors(colors);
            (surface, None, text)
        } else {
            let (surface, hover, text) = self.kind.colors(colors);
            (surface, self.takes_clicks().then_some(hover), text)
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
        let ring = keyboard_focus_ring(
            !self.disabled && focus.is_focused(window),
            if capsule {
                Outline::Capsule
            } else {
                Outline::Rounded
            },
            window,
            cx,
        );

        let content: AnyElement = match self.content {
            Content::Label(label) => label.into_any_element(),
            Content::Icon { icon, .. } => (*icon)
                .size(u(ICON_SIZE))
                .text_color(text)
                .into_any_element(),
        };

        gpui_base::Button::new(self.id)
            .role(if self.default.is_set() {
                Role::DefaultButton
            } else {
                Role::Button
            })
            .accessibility_label(name)
            .disabled(self.disabled)
            .track_focus(&focus)
            .when_some(self.accessibility_id, |this, id| id.apply(this))
            .when_some(description, |this, text| this.aria_description(text))
            .when_some(self.toggled, |this, on| {
                this.aria_toggled(if on {
                    accesskit::Toggled::True
                } else {
                    accesskit::Toggled::False
                })
            })
            .when_some(self.expanded, |this, open| this.aria_expanded(open))
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
            .when(spinner, |this| {
                this.child(
                    div()
                        .debug_selector(|| "fp-spinner".into())
                        .child(Spinner::new().color(text)),
                )
            })
            .child(content)
            .children(keycaps)
            .children(ring)
    }
}

/// The default role, behind the check that keeps it off a danger button.
///
/// The flag of [`DefaultRole`] is private to this module, and
/// [`Button::make_default`] below is the only code that builds it, together
/// with the ⏎ keys, whose key string only this module can name. Code
/// elsewhere in `button.rs` cannot build the flag or name the key string,
/// but it can copy both from a button that passed the check, or change the
/// kind of a danger button around a call to `make_default`, and either
/// compiles. `a_danger_button_never_becomes_the_default_button` builds
/// `Button::danger(..).default_button()` in every build and reads the node
/// it draws, so such code on that path fails it; in a builder method the
/// test does not call, such as `shortcut` or `disabled`, it is past every
/// test. `the_default_role_is_set_behind_its_guard_only` reads this
/// module's source so that it stays as it is.
mod default_role {
    use super::{Button, ButtonKind, parse_keys};

    /// The key string of the default button.
    const RETURN: &str = "enter";

    /// Whether a button is the one Return presses.
    #[derive(Clone, Copy, Debug, Default)]
    pub(super) struct DefaultRole(bool);

    impl DefaultRole {
        pub(super) fn is_set(self) -> bool {
            self.0
        }
    }

    impl Button {
        /// What [`Button::default_button`] does after its debug check.
        pub(super) fn make_default(mut self) -> Self {
            if self.kind == ButtonKind::Danger {
                return self;
            }
            self.default = DefaultRole(true);
            if self.keys.is_empty() {
                self.keys = parse_keys(RETURN);
            }
            self
        }
    }
}

/// The debug build's check in [`Button::default_button`].
///
/// A test can turn it off for its thread, so that it runs
/// `default_button` on a danger button the way a release build does,
/// where the check does not exist.
mod debug_check {
    use super::ButtonKind;

    #[cfg(test)]
    thread_local! {
        static OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// Panics in a debug build when `kind` is `Danger`.
    pub(super) fn danger_is_never_the_default(kind: ButtonKind) {
        #[cfg(test)]
        if OFF.get() {
            return;
        }
        debug_assert!(
            kind != ButtonKind::Danger,
            "a danger button is never the default button"
        );
    }

    /// Runs `build` with the check off on this thread.
    #[cfg(test)]
    pub(super) fn as_in_a_release_build<R>(build: impl FnOnce() -> R) -> R {
        OFF.set(true);
        let result = build();
        OFF.set(false);
        result
    }
}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        self.into_base(window, cx)
    }
}

/// What VoiceOver's press does: the same as a click from the keyboard.
pub(super) fn voiceover_press(
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
/// is not loading. The time comes from GPUI's clock, which is the system
/// clock in the app and a clock the tests move.
fn spinner_due(id: &ElementId, window: &mut Window, cx: &mut App) -> bool {
    let now = cx.background_executor().now();
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

    /// For a button that switches something on and off, such as showing a
    /// password: whether it is on, which VoiceOver reads with the name.
    pub fn toggled(mut self, on: bool) -> Self {
        self.0.toggled = Some(on);
        self
    }

    /// For a button that opens a menu: whether the menu is open.
    pub fn expanded(mut self, open: bool) -> Self {
        self.0.expanded = Some(open);
        self
    }

    pub fn accessibility_id(mut self, id: AccessibilityId) -> Self {
        self.0.accessibility_id = Some(id);
        self
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

    use gpui::{
        Bounds, CursorStyle, Modifiers, MouseButton, Pixels, TestAppContext, VisualTestContext,
        point,
    };
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
            "it shows ⏎ and hands it to aria_keyshortcuts"
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

    /// In a release build `default_button` leaves a danger button as it
    /// was. In a debug build its check panics first, so the test turns the
    /// check off for its thread and runs `default_button` itself, as a
    /// release build does, then reads the node the button draws.
    #[gpui::test]
    fn a_danger_button_never_becomes_the_default_button(cx: &mut TestAppContext) {
        let (role, node) = node_of(cx, || {
            debug_check::as_in_a_release_build(|| {
                Button::danger("delete", label("Delete")).default_button()
            })
        });
        assert_eq!(role, Some(Role::Button), "a danger button stays a button");
        assert_eq!(node.keyboard_shortcut(), None, "and shows no ⏎");
        for kind in ButtonKind::ALL
            .into_iter()
            .filter(|kind| *kind != ButtonKind::Danger)
        {
            let (role, _) = node_of(cx, move || {
                Button::new("b", kind, label("B")).default_button()
            });
            assert_eq!(role, Some(Role::DefaultButton), "{kind:?}");
        }
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

    /// A loading button shows its label alone at first, so a quick action
    /// does not flash a spinner, and the spinner joins the label once the
    /// delay is over. The button reads GPUI's clock, which the test moves.
    #[gpui::test]
    fn a_loading_button_shows_its_spinner_after_the_delay(cx: &mut TestAppContext) {
        let cx = probe::window(cx, |_, _| {
            Button::primary("save", label("Save"))
                .loading(true)
                .into_any_element()
        });
        let spinner = |cx: &mut VisualTestContext| {
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.debug_bounds("fp-spinner")
        };
        assert_eq!(spinner(cx), None, "no spinner at first");
        cx.executor()
            .advance_clock(SPINNER_DELAY - Duration::from_millis(1));
        assert_eq!(spinner(cx), None, "no spinner before the delay is over");
        cx.executor().advance_clock(Duration::from_millis(1));
        assert!(spinner(cx).is_some(), "the spinner once the delay is over");
    }

    /// The copper surface a window painted, with full opacity, and the
    /// opacity it was painted with.
    fn copper_paint(cx: &mut VisualTestContext) -> (Hsla, f32) {
        let copper = [
            LIGHT.button_primary_bg.hsla,
            LIGHT.button_primary_hover.hsla,
        ];
        cx.update(|window, _| {
            window
                .painted_quads()
                .into_iter()
                .filter_map(|quad| quad.background.as_solid())
                .find(|color| copper.contains(&Hsla { a: 1.0, ..*color }))
                .map(|color| (Hsla { a: 1.0, ..color }, color.a))
                .expect("the copper surface")
        })
    }

    /// A loading button takes no click, so it answers the pointer no more
    /// than a disabled one: no pointing hand, no hover surface, no fade
    /// while held. A button that takes clicks shows all three.
    #[gpui::test]
    fn a_loading_button_does_not_answer_the_pointer(cx: &mut TestAppContext) {
        let cursors = probe::in_frame(cx, |window, cx| {
            [("ready", false), ("busy", true)].map(|(id, loading)| {
                let mut base = Button::primary(id, label("Save"))
                    .loading(loading)
                    .on_click(|_, _, _| {})
                    .into_base(window, cx);
                Styled::style(&mut base).mouse_cursor
            })
        });
        assert_eq!(
            cursors,
            [Some(CursorStyle::PointingHand), None],
            "the pointing hand, ready and loading"
        );

        for loading in [false, true] {
            let cx = probe::window(cx, move |_, _| {
                Button::primary("save", label("Save"))
                    .loading(loading)
                    .on_click(|_, _, _| {})
                    .into_any_element()
            });
            let at = point(px(10.), px(10.));
            cx.simulate_mouse_move(at, None, Modifiers::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let (hovered, _) = copper_paint(cx);
            cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
            cx.update(|window, cx| window.draw(cx).clear(cx));
            let (_, held) = copper_paint(cx);
            cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
            if loading {
                assert_eq!(
                    hovered, LIGHT.button_primary_bg.hsla,
                    "loading: no hover surface"
                );
                assert_eq!(held, 1.0, "loading: no fade while held");
            } else {
                assert_eq!(
                    hovered, LIGHT.button_primary_hover.hsla,
                    "ready: the hover surface"
                );
                assert!(
                    (held - PRESSED_OPACITY).abs() < 1e-6,
                    "ready: the fade while held, {held}"
                );
            }
        }
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
    /// button a window draws, so a view with a second one fails here. The
    /// view holds one primary button, a button of every other kind, a
    /// selected row with its ghost verb, the panel footer with its menu
    /// button and a secret field with its icon buttons.
    #[gpui::test]
    #[cfg(debug_assertions)]
    fn one_copper_per_view(cx: &mut TestAppContext) {
        use super::super::{PanelFooter, Row, RowParts, SecretField, Shortcut, Tile, VaultBadge};

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
                Row::new(
                    "row",
                    0,
                    1,
                    RowParts::new(label("Web login"), label("Salesforce")),
                    Tile::new(IconName::Globe),
                    VaultBadge::Personal,
                )
                .selected(true)
                .verb(Button::ghost("open-row", label("Open website")))
                .into_any_element(),
                PanelFooter::new("footer", |_, _, _| {})
                    .primary(Shortcut::from_keystroke("enter", label("Open website")))
                    .more(Shortcut::from_keystroke("cmd-c", label("Password")))
                    .into_any_element(),
                SecretField::new("password", label("Password"))
                    .value(zeroize::Zeroizing::new("sp-test".to_string()))
                    .on_copy(|_, _, _| {})
                    .on_toggle_reveal(|_, _, _| {})
                    .into_any_element(),
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

    /// Tab shows the ring around the button, 1 px away and 2 px wide, and
    /// takes it to the next button; a click on a focused button does not
    /// show it.
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
                .child(Button::secondary("cancel", label("Cancel")).on_click(|_, _, _| {}))
                .into_any_element()
        });
        probe::assert_keyboard_focus_ring(cx, "fp-test-button");
    }

    /// The default role is built in one place: `make_default` in the module
    /// `default_role`, after the check for a danger button. The compiler
    /// keeps every other line of `button.rs` from building it as long as
    /// the role's flag and the ⏎ key string are private to that module, but
    /// not from copying both from another button; this reads the source so
    /// that they stay private, and so that the module itself builds and
    /// writes the flag nowhere else.
    #[test]
    fn the_default_role_is_set_behind_its_guard_only() {
        let code: String = probe::code_only(include_str!("button.rs"))
            .split_whitespace()
            .collect();
        let start = code
            .find("moddefault_role{")
            .expect("the module default_role");
        let module = probe::braced(&code[start..]);
        assert!(
            module.contains("pub(super)structDefaultRole(bool);"),
            "DefaultRole is declared in default_role, with a private flag: {module}"
        );
        assert!(
            module.contains("constRETURN:&str="),
            "the ⏎ key string is private to default_role: {module}"
        );
        assert!(
            module.contains(
                "ifself.kind==ButtonKind::Danger{returnself;}\
                 self.default=DefaultRole(true);\
                 ifself.keys.is_empty(){self.keys=parse_keys(RETURN);}"
            ),
            "make_default sets the role and the ⏎ keys after the check for a danger button: {module}"
        );
        // The declaration and the one construction in `make_default`.
        assert_eq!(
            code.matches("DefaultRole(").count(),
            2,
            "DefaultRole is built in make_default only"
        );
        assert_eq!(
            module.matches("Self(").count() + module.matches("{0:").count(),
            0,
            "default_role builds DefaultRole nowhere else, as Self(..) or with braces: {module}"
        );
        assert_eq!(
            module.matches(".0").count(),
            1,
            "default_role reads the flag in is_set and writes it nowhere: {module}"
        );
        assert_eq!(
            code.matches("RETURN").count(),
            2,
            "the ⏎ key string is used in make_default only"
        );
    }

    /// A loading button asks for the next frame for as long as its spinner
    /// is not due, so the spinner shows on time even when nothing else
    /// draws the window again. In this test nothing does: the window is
    /// drawn only for the frames the button asks for, which the test
    /// delivers as the display would.
    #[gpui::test]
    fn a_loading_button_asks_for_frames_until_its_spinner_shows(cx: &mut TestAppContext) {
        let next_frame = |cx: &mut VisualTestContext| {
            let asked = cx.update(|window, cx| window.simulate_next_frame(cx));
            cx.run_until_parked();
            asked
        };
        let idle = probe::window(cx, |_, _| {
            Button::primary("save", label("Save")).into_any_element()
        });
        assert_eq!(
            next_frame(idle),
            0,
            "a button that is not loading asks for none"
        );

        let cx = probe::window(cx, |_, _| {
            Button::primary("save", label("Save"))
                .loading(true)
                .into_any_element()
        });
        assert_eq!(cx.debug_bounds("fp-spinner"), None, "no spinner at first");
        cx.executor()
            .advance_clock(SPINNER_DELAY - Duration::from_millis(1));
        assert_eq!(
            next_frame(cx),
            1,
            "the button asks for a frame while its spinner is not due"
        );
        assert_eq!(
            cx.debug_bounds("fp-spinner"),
            None,
            "no spinner before the delay is over"
        );
        cx.executor().advance_clock(Duration::from_millis(1));
        assert_eq!(next_frame(cx), 1, "it asked again in that frame");
        assert!(
            cx.debug_bounds("fp-spinner").is_some(),
            "the frame it asked for shows the spinner"
        );
    }

    /// What the window painted in the place of its keycap: the colors of
    /// its fills and of its edges. GPUI paints a surface and its edge as
    /// quads of their own.
    fn keycap_paint(cx: &mut VisualTestContext) -> (Vec<Hsla>, Vec<Hsla>) {
        let keycap = cx.debug_bounds("fp-keycap").expect("the keycap");
        // Painting snaps to device pixels, so a quad is within one pixel of
        // the bounds of its element.
        let near = |a: Bounds<Pixels>, b: Bounds<Pixels>| {
            [
                (a.left(), b.left()),
                (a.top(), b.top()),
                (a.right(), b.right()),
                (a.bottom(), b.bottom()),
            ]
            .into_iter()
            .all(|(x, y)| (x - y).abs() <= px(1.))
        };
        cx.update(|window, _| {
            let scale = window.scale_factor();
            let quads: Vec<_> = window
                .painted_quads()
                .into_iter()
                .filter(|quad| near(quad.bounds.map(|value| px(value.0 / scale)), keycap))
                .collect();
            let fills = quads
                .iter()
                .filter(|quad| !quad.background.is_transparent())
                .filter_map(|quad| quad.background.as_solid())
                .collect();
            let edges = quads
                .iter()
                .map(|quad| quad.border_color)
                .filter(|color| !color.is_transparent())
                .collect();
            (fills, edges)
        })
    }

    /// On a copper button a keycap has no surface of its own, only an edge
    /// in `keycap_on_accent_border`, so the copper shows through. On every
    /// other button it keeps its surface and its edge, and so it does on a
    /// disabled copper button, which has no copper.
    #[gpui::test]
    fn a_keycap_on_copper_has_no_surface_of_its_own(cx: &mut TestAppContext) {
        let enabled = ButtonKind::ALL.map(|kind| (kind, false));
        let disabled_copper = [(ButtonKind::Primary, true), (ButtonKind::Large, true)];
        for (kind, disabled) in enabled.into_iter().chain(disabled_copper) {
            let case = if disabled {
                format!("disabled {kind:?}")
            } else {
                format!("{kind:?}")
            };
            let cx = probe::window(cx, move |_, _| {
                Button::new("copy", kind, label("Copy password"))
                    .shortcut("cmd-shift-c")
                    .disabled(disabled)
                    .into_any_element()
            });
            let colors = cx.update(|_, cx| cx.ds().colors);
            let (fills, edges) = keycap_paint(cx);
            let (fill, edge) = match (kind, disabled) {
                (ButtonKind::Primary | ButtonKind::Large, false) => {
                    (None, colors.keycap_on_accent_border.hsla)
                }
                _ => (Some(colors.keycap_bg.hsla), colors.keycap_border.hsla),
            };
            assert_eq!(
                fills,
                fill.into_iter().collect::<Vec<_>>(),
                "{case}: the keycap's fill"
            );
            assert!(!edges.is_empty(), "{case}: the keycap has an edge");
            assert!(
                edges.iter().all(|color| *color == edge),
                "{case}: the keycap's edge is {edge:?}: {edges:?}"
            );
        }
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

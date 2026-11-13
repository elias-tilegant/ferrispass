//! The gallery's view: the bar with the mode, the text size and the language
//! at the top, the list of pages on the left, the page on the right.
//!
//! The bar and the list are buttons of the design system as well: the one
//! that is on is a secondary button, the others are quiet. Around them the
//! view draws frames and captions, nothing else.
//!
//! The mode, the text size and the language belong to the gallery. It puts
//! them into effect through `ds::apply` with an `Appearance` of its own,
//! increased contrast included, and through `rust_i18n::set_locale`, so it
//! can show all four modes and both languages whatever macOS is set to, and
//! it changes nothing there.

use ferrispass::app::assets;
use ferrispass::app::settings::{TextSize, ThemeChoice};
use ferrispass::i18n::{self, Language};
use ferrispass::ui::ds::components::{Button, ButtonKind, ButtonSize};
use ferrispass::ui::ds::tokens::dimension;
use ferrispass::ui::ds::typography::{TextStyle, TextStyled as _, u};
use ferrispass::ui::ds::{self, A11yPrefs, ActiveTokens as _, Appearance, Mode};
use gpui::{
    AnyElement, App, ClickEvent, Context, FocusHandle, InteractiveElement as _, IntoElement,
    KeyBinding, ParentElement as _, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, actions, div, px,
};

use crate::pages::{self, Page, Samples};

/// The key context of the view.
pub const CONTEXT: &str = "Gallery";

actions!(
    gallery,
    [
        PreviousPage,
        NextPage,
        LightMode,
        DarkMode,
        LightMoreContrast,
        DarkMoreContrast,
        LargerText,
        SmallerText,
        SwitchLanguage,
    ]
);

/// What the app does at start before its first window opens, with the
/// gallery's keys in place of the app's: the bundled fonts, the Kit, the
/// text keys and the design system.
pub fn init(cx: &mut App) {
    let fonts = assets::font_bytes();
    if !fonts.is_empty()
        && let Err(error) = cx.text_system().add_fonts(fonts)
    {
        eprintln!("design gallery: could not load the bundled fonts: {error:#}");
    }
    gpui_component::init(cx);
    i18n::init();
    ds::init(ThemeChoice::Light, TextSize::Standard, cx);
    bind_keys(cx);
}

/// ↑ and ↓ for the page, 1 to 4 for the mode, + and - for the text size,
/// L for the language. They are off while a text field has focus, where the
/// same keys type.
pub fn bind_keys(cx: &mut App) {
    let context = Some("Gallery && !Input");
    cx.bind_keys([
        KeyBinding::new("up", PreviousPage, context),
        KeyBinding::new("down", NextPage, context),
        KeyBinding::new("1", LightMode, context),
        KeyBinding::new("2", DarkMode, context),
        KeyBinding::new("3", LightMoreContrast, context),
        KeyBinding::new("4", DarkMoreContrast, context),
        KeyBinding::new("+", LargerText, context),
        KeyBinding::new("=", LargerText, context),
        KeyBinding::new("-", SmallerText, context),
        KeyBinding::new("l", SwitchLanguage, context),
    ]);
}

/// The mode, the text size and the language the gallery shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    pub mode: Mode,
    pub text_size: TextSize,
    pub language: Language,
}

impl Look {
    /// What the gallery opens with.
    pub const FIRST: Look = Look {
        mode: Mode::Light,
        text_size: TextSize::Standard,
        language: Language::English,
    };

    /// Puts the look into effect: the mode and the text size through
    /// `ds::apply`, with contrast from the mode rather than from macOS and
    /// motion and VoiceOver off, and the language through rust-i18n.
    fn apply(self, cx: &mut App) {
        let (choice, increase_contrast) = match self.mode {
            Mode::Light => (ThemeChoice::Light, false),
            Mode::Dark => (ThemeChoice::Dark, false),
            Mode::LightHc => (ThemeChoice::Light, true),
            Mode::DarkHc => (ThemeChoice::Dark, true),
        };
        ds::apply(
            cx,
            Appearance {
                choice,
                prefs: A11yPrefs {
                    increase_contrast,
                    reduce_motion: false,
                    voiceover: false,
                },
                text_size: ds::text_size_points(self.text_size),
            },
        );
        rust_i18n::set_locale(self.language.code());
    }
}

/// The modes in the order of their keys, with their captions.
const MODES: [(Mode, &str, &str); 4] = [
    (Mode::Light, "light", "1"),
    (Mode::Dark, "dark", "2"),
    (Mode::LightHc, "light, more contrast", "3"),
    (Mode::DarkHc, "dark, more contrast", "4"),
];

/// The text sizes, with their captions and keys.
const TEXT_SIZES: [(TextSize, &str, &str); 2] = [
    (TextSize::Standard, "13", "-"),
    (TextSize::Large, "15", "+"),
];

/// The gallery window's root view.
pub struct Gallery {
    focus: FocusHandle,
    page: Page,
    look: Look,
    samples: Samples,
}

impl Gallery {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Look::FIRST.apply(cx);
        Self {
            focus,
            page: Page::ALL[0],
            look: Look::FIRST,
            samples: Samples::new(window, cx),
        }
    }

    pub fn show(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        cx.notify();
    }

    pub fn set_look(&mut self, look: Look, window: &mut Window, cx: &mut Context<Self>) {
        self.look = look;
        look.apply(cx);
        self.samples.follow_language(window, cx);
        // Every view draws again, in the new tokens and the new language.
        window.refresh();
    }

    /// The page `delta` places further down the list, stopping at both ends.
    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let at = Page::ALL
            .iter()
            .position(|page| *page == self.page)
            .unwrap_or_default();
        let to = at
            .saturating_add_signed(delta)
            .min(Page::ALL.len().saturating_sub(1));
        self.show(Page::ALL[to], cx);
    }

    fn set_mode(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        self.set_look(Look { mode, ..self.look }, window, cx);
    }

    fn set_text_size(&mut self, text_size: TextSize, window: &mut Window, cx: &mut Context<Self>) {
        self.set_look(
            Look {
                text_size,
                ..self.look
            },
            window,
            cx,
        );
    }

    fn switch_language(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let language = match self.look.language {
            Language::English => Language::German,
            Language::German => Language::English,
        };
        self.set_look(
            Look {
                language,
                ..self.look
            },
            window,
            cx,
        );
    }

    /// The bar: four modes, two text sizes, two languages.
    fn bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let look = self.look;
        let modes = MODES.map(|(mode, caption, keys)| {
            choice(("mode", mode as usize), caption, look.mode == mode)
                .shortcut(keys)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_mode(mode, window, cx)
                }))
        });
        let sizes = TEXT_SIZES.map(|(size, caption, keys)| {
            choice(("size", size as usize), caption, look.text_size == size)
                .shortcut(keys)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.set_text_size(size, window, cx)
                }))
        });
        let languages = Language::ALL.map(|language| {
            choice(
                ("language", language as usize),
                language.code(),
                look.language == language,
            )
            .shortcut("l")
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                if this.look.language != language {
                    this.switch_language(window, cx);
                }
            }))
        });
        let colors = cx.ds().colors;
        div()
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap(u(dimension::SPACE_16))
            .px(u(dimension::SPACE_16))
            .py(u(dimension::SPACE_8))
            .border_b_1()
            .border_color(colors.border_separator.hsla)
            .child(group("mode", modes))
            .child(group("text size", sizes))
            .child(group("language", languages))
    }

    /// One button per page, the one shown on.
    fn page_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.ds().colors;
        let shown = self.page;
        let buttons = Page::ALL.iter().enumerate().map(|(index, page)| {
            let page = *page;
            choice(("page", index), page.name(), page == shown)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.show(page, cx)))
        });
        div()
            .id("pages")
            .flex()
            .flex_col()
            .flex_none()
            .gap(u(dimension::SPACE_2))
            .w(u(dimension::WIDTH_SIDEBAR))
            .p(u(dimension::SPACE_8))
            .overflow_y_scroll()
            .bg(colors.bg_sidebar.hsla)
            .children(buttons)
    }
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.ds().colors;
        let page = self.page;
        let index = Page::ALL
            .iter()
            .position(|shown| *shown == page)
            .unwrap_or_default();
        let body: AnyElement = pages::page(page, &self.samples, cx);
        div()
            .id("gallery")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &PreviousPage, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &NextPage, _, cx| this.step(1, cx)))
            .on_action(
                cx.listener(|this, _: &LightMode, window, cx| {
                    this.set_mode(Mode::Light, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &DarkMode, window, cx| this.set_mode(Mode::Dark, window, cx)),
            )
            .on_action(cx.listener(|this, _: &LightMoreContrast, window, cx| {
                this.set_mode(Mode::LightHc, window, cx)
            }))
            .on_action(cx.listener(|this, _: &DarkMoreContrast, window, cx| {
                this.set_mode(Mode::DarkHc, window, cx)
            }))
            .on_action(cx.listener(|this, _: &LargerText, window, cx| {
                this.set_text_size(TextSize::Large, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SmallerText, window, cx| {
                this.set_text_size(TextSize::Standard, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &SwitchLanguage, window, cx| {
                    this.switch_language(window, cx)
                }),
            )
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.bg_window.hsla)
            .text_color(colors.text_primary.hsla)
            .text_style(TextStyle::Body)
            .child(self.bar(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.))
                    .child(self.page_list(cx))
                    .child(
                        // Each page keeps a scroll position of its own.
                        div()
                            .id(("page-body", index))
                            .flex_1()
                            .min_w(px(0.))
                            .overflow_y_scroll()
                            .p(u(dimension::SPACE_24))
                            .child(body),
                    ),
            )
    }
}

/// A button of the bar or the page list: secondary while it is on, quiet
/// otherwise.
fn choice(id: impl Into<gpui::ElementId>, caption: impl Into<SharedString>, on: bool) -> Button {
    let kind = if on {
        ButtonKind::Secondary
    } else {
        ButtonKind::Quiet
    };
    Button::new(id, kind, caption).size(ButtonSize::S24)
}

/// Buttons of the bar under their caption.
fn group<const N: usize>(caption: &'static str, buttons: [Button; N]) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(u(dimension::SPACE_4))
        .child(pages::caption(caption))
        .children(buttons)
}

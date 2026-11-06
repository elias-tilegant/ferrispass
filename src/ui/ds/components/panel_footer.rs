//! The panel footer: what Return does now, and the shortcuts next to it.
//!
//! The first line, in `Headline`, says what Return does for the selected
//! entry, with its target and its user. The second, in `Callout`, holds up
//! to three more shortcut pairs and, when there is one, a short note, such
//! as how old the data is while offline. A third line can say what to
//! connect to first. The button at the end opens the menu with locking,
//! opening FerrisPass and the settings.
//!
//! The footer takes no focus and is no node: VoiceOver hears its text
//! through the description of the element that has focus, which the panel
//! sets from [`PanelFooter::description_text`] on the search field and on
//! the selected row. The one node in it is the button that opens the menu,
//! and nothing in it is a live region.
//!
//! The tone colors the first line: ochre with a glyph while Return waits for
//! a production system to be chosen, blue while a decision is open, and
//! ochre once that decision is a day old. The caller says which.

use std::rc::Rc;

use gpui::{
    App, ClickEvent, Div, ElementId, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, SharedString, Styled as _, Window, div, px,
};
use gpui_component::{Icon, IconName};

use super::button::ClickHandler;
use super::{IconButton, Shortcut, counted};
use crate::tr;
use crate::ui::ds::ActiveTokens as _;
use crate::ui::ds::tokens::{ColorSet, dimension};
use crate::ui::ds::typography::{TextStyle, TextStyled as _, u};

/// The footer shows at most this many shortcut pairs after the first line.
pub const MAX_MORE: usize = 3;

/// The boxes of the pairs on the second line, first to third: the debug
/// selector of each, which is also the name it counts its children under
/// in the tests.
const MORE_PLACES: [&str; MAX_MORE] = ["fp-footer-more-0", "fp-footer-more-1", "fp-footer-more-2"];

/// Size of the glyph on the first line, in points.
const MAIN_GLYPH: f32 = 16.0;

/// Size of the glyph before the line about what to connect to first.
const VIA_GLYPH: f32 = 12.0;

/// Between the pairs of the second line in the description.
const LIST_SEPARATOR: &str = ", ";

/// Between the lines in the description, unless a line ends in a stop of
/// its own.
const SENTENCE_SEPARATOR: &str = ". ";

/// What the footer says besides its shortcuts, and in which color.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum FooterTone {
    #[default]
    Normal,
    /// The search is empty and the first match is a production system, so
    /// Return waits until one is chosen. The first line, which says so, is
    /// ochre with a glyph.
    ProductionSelectFirst,
    /// A decision is open, and the first line says which: blue, and ochre
    /// once it is stale, a day after it came up.
    Decision { stale: bool },
    /// Offline: the note says how old the data is, in ochre.
    Offline(SharedString),
    /// Matches that look the same: the note says how many.
    Tie(SharedString),
    /// An update is ready: the note says so, quietly.
    UpdateReady(SharedString),
}

impl FooterTone {
    /// The color of the first line.
    fn main_color(&self, colors: &ColorSet) -> Hsla {
        match self {
            Self::ProductionSelectFirst | Self::Decision { stale: true } => {
                colors.status_attention_text.hsla
            }
            Self::Decision { stale: false } => colors.status_decision_text.hsla,
            Self::Normal | Self::Offline(_) | Self::Tie(_) | Self::UpdateReady(_) => {
                colors.text_primary.hsla
            }
        }
    }

    /// The glyph before the first line, while Return waits.
    fn main_glyph(&self) -> Option<IconName> {
        (*self == Self::ProductionSelectFirst).then_some(IconName::TriangleAlert)
    }

    /// The note at the end of the second line.
    fn note(&self) -> Option<&SharedString> {
        match self {
            Self::Offline(note) | Self::Tie(note) | Self::UpdateReady(note) => Some(note),
            Self::Normal | Self::ProductionSelectFirst | Self::Decision { .. } => None,
        }
    }

    /// The color of the note.
    fn note_color(&self, colors: &ColorSet) -> Hsla {
        match self {
            Self::Offline(_) => colors.status_attention_text.hsla,
            Self::UpdateReady(_) => colors.text_tertiary.hsla,
            _ => colors.text_secondary.hsla,
        }
    }
}

/// The footer of the quick panel.
#[derive(IntoElement)]
pub struct PanelFooter {
    id: ElementId,
    primary: Option<Shortcut>,
    more: Vec<Shortcut>,
    connect_first: Option<SharedString>,
    tone: FooterTone,
    on_more: ClickHandler,
    menu_open: bool,
}

impl PanelFooter {
    /// A footer whose menu button runs `on_more`, which opens the menu.
    pub fn new(
        id: impl Into<ElementId>,
        on_more: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            primary: None,
            more: Vec::new(),
            connect_first: None,
            tone: FooterTone::Normal,
            on_more: Rc::new(on_more),
            menu_open: false,
        }
    }

    /// The first line: ⏎ and what it does, such as "Sign in to NHP, client
    /// 100 as N.WEISS". A pair without keys shows its words alone, as for a
    /// decision.
    pub fn primary(mut self, shortcut: Shortcut) -> Self {
        self.primary = Some(shortcut);
        self
    }

    /// One more shortcut pair on the second line. The footer shows at most
    /// three: a fourth fails in a debug build and is left out in a release
    /// build.
    pub fn more(mut self, shortcut: Shortcut) -> Self {
        debug_check::at_most_three_more(self.more.len());
        if self.more.len() < MAX_MORE {
            self.more.push(shortcut);
        }
        self
    }

    /// The third line: what to connect to first, such as "Connect first:
    /// Nordhafen GP (VPN)".
    pub fn connect_first(mut self, text: impl Into<SharedString>) -> Self {
        self.connect_first = Some(text.into());
        self
    }

    pub fn tone(mut self, tone: FooterTone) -> Self {
        self.tone = tone;
        self
    }

    /// Whether the menu the button opens is open.
    pub fn menu_open(mut self, open: bool) -> Self {
        self.menu_open = open;
        self
    }

    /// Everything the footer says, as sentences: the first line, the pairs
    /// of the second line and its note, and the line about what to connect
    /// to first, each pair with its keys. The panel puts it into the
    /// description of the search field and of the selected row, since the
    /// footer itself is no node.
    pub fn description_text(&self) -> SharedString {
        let mut sentences: Vec<String> = Vec::new();
        sentences.extend(self.primary.as_ref().map(spoken));
        let pairs: Vec<String> = self.more.iter().map(spoken).collect();
        sentences.push(pairs.join(LIST_SEPARATOR));
        sentences.extend(self.tone.note().map(ToString::to_string));
        sentences.extend(self.connect_first.as_ref().map(ToString::to_string));
        let mut text = String::new();
        for sentence in sentences.iter().map(|sentence| sentence.trim()) {
            if sentence.is_empty() {
                continue;
            }
            if !text.is_empty() {
                let stopped = text.ends_with(['.', '!', '?', ':']);
                text.push_str(if stopped { " " } else { SENTENCE_SEPARATOR });
            }
            text.push_str(sentence);
        }
        text.into()
    }
}

/// A shortcut pair as the description says it: its keys, then its words.
fn spoken(shortcut: &Shortcut) -> String {
    let keys = shortcut.keys_text();
    if keys.is_empty() {
        shortcut.label().to_string()
    } else {
        format!("{keys} {}", shortcut.label())
    }
}

/// The debug build's check in [`PanelFooter::more`], which a test can turn
/// off for its thread, so that it runs `more` as a release build does.
mod debug_check {
    use super::MAX_MORE;

    #[cfg(test)]
    thread_local! {
        static OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// Panics in a debug build when the footer has its three pairs already.
    pub(super) fn at_most_three_more(shown: usize) {
        #[cfg(test)]
        if OFF.get() {
            return;
        }
        debug_assert!(
            shown < MAX_MORE,
            "the footer shows at most three more shortcuts"
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

/// Every element the footer builds, before `assemble` puts them in place.
/// None of them but the menu button has an element id, so none of them but
/// the button writes a node, and none of them takes focus. Its boxes are
/// `counted`, so the tests find a child put into one of them in `build`
/// that `parts()` does not read.
struct Built {
    footer: Div,
    lines: Div,
    main: Div,
    glyph: Option<Icon>,
    primary: Option<Shortcut>,
    second: Div,
    more: Vec<(Div, Shortcut)>,
    note: Option<(Div, SharedString)>,
    via: Option<(Div, Icon, SharedString)>,
    menu: IconButton,
}

impl PanelFooter {
    fn build(self, cx: &mut App) -> Built {
        let colors = cx.ds().colors;
        let main_color = self.tone.main_color(colors);
        let footer = counted(div(), "fp-footer")
            .debug_selector(|| "fp-footer".into())
            .flex()
            .items_start()
            .gap(u(dimension::SPACE_8))
            .w_full()
            .pt(u(dimension::SPACE_8))
            .border_t_1()
            .border_color(colors.border_separator.hsla);
        let lines = counted(div(), "fp-footer-lines")
            .flex()
            .flex_col()
            .gap(u(dimension::SPACE_6))
            .flex_1()
            .min_w(px(0.));
        let main = counted(div(), "fp-footer-main")
            .debug_selector(|| "fp-footer-main".into())
            .flex()
            .items_center()
            .gap(u(dimension::SPACE_8));
        let glyph = self
            .tone
            .main_glyph()
            .map(|glyph| Icon::new(glyph).size(u(MAIN_GLYPH)).text_color(main_color));
        let primary = self.primary.map(|primary| {
            primary
                .text_style(TextStyle::Headline)
                .text_color(main_color)
        });
        let second = counted(div(), "fp-footer-more")
            .debug_selector(|| "fp-footer-more".into())
            .flex()
            .flex_wrap()
            .items_center()
            .gap(u(dimension::SPACE_16));
        let more = self
            .more
            .into_iter()
            .zip(MORE_PLACES)
            .map(|(pair, name)| {
                let place = counted(div(), name)
                    .debug_selector(move || name.into())
                    .flex_none();
                (place, pair)
            })
            .collect();
        let note = self.tone.note().map(|note| {
            let place = counted(div(), "fp-footer-note")
                .debug_selector(|| "fp-footer-note".into())
                .text_style(TextStyle::Callout)
                .text_color(self.tone.note_color(colors));
            (place, note.clone())
        });
        let via = self.connect_first.map(|text| {
            let place = counted(div(), "fp-footer-via")
                .debug_selector(|| "fp-footer-via".into())
                .flex()
                .items_center()
                .gap(u(dimension::SPACE_6))
                .text_style(TextStyle::Callout)
                .text_color(colors.text_secondary.hsla);
            let glyph = Icon::new(IconName::Network)
                .size(u(VIA_GLYPH))
                .text_color(colors.text_secondary.hsla);
            (place, glyph, text)
        });
        let handler = self.on_more.clone();
        let menu = IconButton::new(
            (self.id, "more"),
            IconName::Ellipsis,
            tr!("a11y.more_actions"),
        )
        .expanded(self.menu_open)
        .on_click(move |event, window, cx| handler(event, window, cx));
        Built {
            footer,
            lines,
            main,
            glyph,
            primary,
            second,
            more,
            note,
            via,
            menu,
        }
    }

    /// Every element `render` builds from the footer `make` returns, each
    /// read as the tests read it. The footer itself is read off the element
    /// `render` returns, so whatever is set on it on the way counts too;
    /// `make` builds it once for that and once for the elements inside.
    #[cfg(test)]
    pub(super) fn parts(
        make: impl Fn() -> Self,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<super::probe::ComponentPart> {
        use super::probe::part;
        let footer = part("footer", RenderOnce::render(make(), window, cx));
        // Every field by name, so an element added to `Built` does not
        // compile until it is listed here. The footer is read above.
        let Built {
            footer: _,
            lines,
            main,
            glyph,
            primary,
            second,
            more,
            note,
            via,
            menu,
        } = make().build(cx);
        let mut parts = vec![
            footer,
            part("lines", lines),
            part("main", main),
            part("second", second),
        ];
        parts.extend(glyph.map(|glyph| part("glyph", RenderOnce::render(glyph, window, cx))));
        parts.extend(primary.map(|pair| part("primary", RenderOnce::render(pair, window, cx))));
        for (place, pair) in more {
            parts.push(part("more place", place));
            parts.push(part("more", RenderOnce::render(pair, window, cx)));
        }
        if let Some((place, note)) = note {
            parts.push(part("note place", place));
            parts.push(part("note", note));
        }
        if let Some((place, glyph, text)) = via {
            parts.push(part("via", place));
            parts.push(part("via glyph", RenderOnce::render(glyph, window, cx)));
            parts.push(part("via text", text));
        }
        let menu = menu.into_base(window, cx);
        parts.push(part("menu", RenderOnce::render(menu, window, cx)));
        parts
    }
}

/// Puts what the footer builds in place: the first line with its glyph,
/// the second with its pairs and its note, the third, and the menu button
/// at the end.
fn assemble(built: Built) -> Div {
    let main = built.main.children(built.glyph).children(built.primary);
    let second = built
        .second
        .children(
            built
                .more
                .into_iter()
                .map(|(place, pair)| place.child(pair)),
        )
        .children(built.note.map(|(place, note)| place.child(note)));
    let via = built
        .via
        .map(|(place, glyph, text)| place.child(glyph).child(text));
    built
        .footer
        .child(built.lines.child(main).child(second).children(via))
        .child(built.menu)
}

impl RenderOnce for PanelFooter {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        assemble(self.build(cx))
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Role, Styled, TestAppContext, accesskit};

    use super::super::probe;
    use super::*;
    use crate::ui::ds::tokens::{LIGHT, Mode};

    fn text(value: &'static str) -> SharedString {
        SharedString::new_static(value)
    }

    fn pair(keys: &str, word: &'static str) -> Shortcut {
        Shortcut::from_keystroke(keys, text(word))
    }

    /// The footer of state A of the panel, with an SAP system selected.
    fn sap_footer() -> PanelFooter {
        PanelFooter::new("footer", |_, _, _| {})
            .primary(pair("enter", "Sign in to NHP, client 100 as N.WEISS"))
            .more(pair("cmd-c", "Password"))
            .more(pair("shift-enter", "Type into SAP GUI"))
            .more(pair("right", "More"))
            .connect_first(text("Connect first: Nordhafen GP (VPN)"))
    }

    fn parts_of(
        cx: &mut TestAppContext,
        build: impl Fn() -> PanelFooter + 'static,
    ) -> Vec<probe::ComponentPart> {
        probe::in_frame(cx, move |window, cx| PanelFooter::parts(&build, window, cx))
    }

    /// The color of the first line, as the footer styles its pair.
    fn main_color(cx: &mut TestAppContext, tone: FooterTone) -> Option<Hsla> {
        probe::in_frame(cx, move |_, cx| {
            let mut primary = PanelFooter::new("footer", |_, _, _| {})
                .primary(pair("enter", "Sign in"))
                .tone(tone.clone())
                .build(cx)
                .primary
                .expect("the first line");
            Styled::text_style(&mut primary).color
        })
    }

    /// The footer takes no focus and writes no node of its own: of every
    /// part `parts()` lists, the footer itself read as `render` returns it,
    /// only the menu button has an element id and a role, it is a button,
    /// and no part is a live region or a role that announces itself.
    /// `every_rendered_child_is_a_part` holds what `render` draws to that
    /// list. In a window that holds the footer alone, Tab finds one stop,
    /// the button.
    #[gpui::test]
    fn the_footer_is_not_focusable_and_writes_no_live_node(cx: &mut TestAppContext) {
        let tones = [
            FooterTone::Normal,
            FooterTone::ProductionSelectFirst,
            FooterTone::Decision { stale: true },
            FooterTone::Offline(text("As of 13:40")),
            FooterTone::Tie(text("2 identical matches")),
            FooterTone::UpdateReady(text("Update ready")),
        ];
        for tone in tones {
            let shown = tone.clone();
            let parts = parts_of(cx, move || sap_footer().tone(shown.clone()));
            assert!(parts.len() > 10, "{tone:?}: every part is read");
            for part in &parts {
                let announces = matches!(
                    part.role,
                    Some(Role::Alert | Role::Status | Role::Log | Role::Marquee | Role::Timer)
                );
                assert!(!announces, "{tone:?}: the {} is {:?}", part.name, part.role);
                assert_eq!(
                    part.node.live(),
                    None,
                    "{tone:?}: the {} is live",
                    part.name
                );
                if part.name == "menu" {
                    assert_eq!(part.role, Some(Role::Button));
                    assert!(part.id.is_some());
                } else {
                    assert_eq!(part.id, None, "{tone:?}: the {} has an id", part.name);
                }
            }
        }

        let cx = probe::window(cx, |_, _| sap_footer().into_any_element());
        cx.update(|window, cx| window.focus_next(cx));
        let first = cx.update(|window, cx| window.focused(cx));
        assert!(first.is_some(), "the menu button takes focus");
        cx.update(|window, cx| window.focus_next(cx));
        let second = cx.update(|window, cx| window.focused(cx));
        assert_eq!(first, second, "one tab stop in the footer, the button");
    }

    /// The menu button is named "More actions" and says whether its menu
    /// is open.
    #[gpui::test]
    fn the_more_button_is_named_and_expanded_state_is_set(cx: &mut TestAppContext) {
        for open in [false, true] {
            let parts = parts_of(cx, move || sap_footer().menu_open(open));
            let menu = parts
                .iter()
                .find(|part| part.name == "menu")
                .expect("the menu button");
            // The English of `a11y.more_actions`, written out.
            assert_eq!(menu.node.label(), Some("More actions"));
            assert_eq!(menu.node.is_expanded(), Some(open), "open: {open}");
            assert!(menu.node.supports_action(accesskit::Action::Click));
        }
    }

    /// While Return waits for a production system to be chosen, the first
    /// line is ochre, `status_attention_text`, never carmine, with a glyph
    /// before it. In the other tones it has no glyph.
    #[gpui::test]
    fn a_production_footer_is_ocher_with_its_glyph(cx: &mut TestAppContext) {
        for mode in Mode::ALL {
            let colors = mode.colors();
            let ochre = FooterTone::ProductionSelectFirst.main_color(colors);
            assert_eq!(ochre, colors.status_attention_text.hsla, "{mode:?}");
            assert_ne!(ochre, colors.status_danger_text.hsla, "{mode:?}");
        }
        assert_eq!(
            main_color(cx, FooterTone::ProductionSelectFirst),
            Some(LIGHT.status_attention_text.hsla)
        );
        assert_eq!(
            main_color(cx, FooterTone::Normal),
            Some(LIGHT.text_primary.hsla)
        );
        let build = || {
            PanelFooter::new("footer", |_, _, _| {})
                .primary(pair(
                    "",
                    "Production system: select it first (↓ or ⌘1), then ⏎",
                ))
                .tone(FooterTone::ProductionSelectFirst)
        };
        let parts = parts_of(cx, build);
        assert!(parts.iter().any(|part| part.name == "glyph"), "a glyph");
        let parts = parts_of(cx, sap_footer);
        assert!(!parts.iter().any(|part| part.name == "glyph"), "no glyph");
    }

    /// An open decision is blue on the first line, and ochre once the
    /// caller says it is stale, a day after it came up.
    #[gpui::test]
    fn a_stale_decision_turns_ocher_after_a_day(cx: &mut TestAppContext) {
        assert_eq!(
            main_color(cx, FooterTone::Decision { stale: false }),
            Some(LIGHT.status_decision_text.hsla)
        );
        assert_eq!(
            main_color(cx, FooterTone::Decision { stale: true }),
            Some(LIGHT.status_attention_text.hsla)
        );
    }

    /// Every word and every key the footer shows is in its description:
    /// each pair with its keys, the note, and the line about what to
    /// connect to first.
    #[test]
    fn every_footer_text_lands_in_the_description() {
        let footer = sap_footer().tone(FooterTone::Offline(text("As of 13:40")));
        let description = footer.description_text();
        assert_eq!(
            description,
            "⏎ Sign in to NHP, client 100 as N.WEISS. ⌘C Password, ⇧⏎ Type into SAP GUI, \
             → More. As of 13:40. Connect first: Nordhafen GP (VPN)"
        );
        let shown = footer.primary.iter().chain(&footer.more);
        for pair in shown {
            assert!(
                description.contains(pair.label().as_ref()),
                "{}",
                pair.label()
            );
            assert!(
                description.contains(pair.keys_text().as_ref()),
                "{}",
                pair.label()
            );
        }
        let waiting = PanelFooter::new("footer", |_, _, _| {})
            .primary(pair(
                "",
                "Production system: select it first (↓ or ⌘1), then ⏎",
            ))
            .tone(FooterTone::ProductionSelectFirst);
        assert_eq!(
            waiting.description_text(),
            "Production system: select it first (↓ or ⌘1), then ⏎"
        );
        let tie = PanelFooter::new("footer", |_, _, _| {})
            .primary(pair("enter", "Open website"))
            .tone(FooterTone::Tie(text("2 identical matches")));
        assert_eq!(
            tie.description_text(),
            "⏎ Open website. 2 identical matches"
        );
    }

    /// The footer shows at most three more pairs. A fourth fails in a debug
    /// build; in a release build, run here with the debug check off, the
    /// footer shows the first three, and its description says three.
    #[gpui::test]
    fn the_footer_shows_at_most_three_more_shortcuts(cx: &mut TestAppContext) {
        let four = || {
            debug_check::as_in_a_release_build(|| {
                sap_footer().more(pair("cmd-shift-c", "User name"))
            })
        };
        let description = four().description_text();
        assert!(description.contains("→ More"), "{description}");
        assert!(!description.contains("User name"), "{description}");
        let cx = probe::window(cx, move |_, _| four().into_any_element());
        for place in ["fp-footer-more-0", "fp-footer-more-1", "fp-footer-more-2"] {
            assert!(cx.debug_bounds(place).is_some(), "{place}");
        }
        assert_eq!(cx.debug_bounds("fp-footer-more-3"), None, "no fourth pair");
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "the footer shows at most three more shortcuts")]
    fn a_fourth_more_shortcut_fails_in_a_debug_build() {
        let _ = sap_footer().more(pair("cmd-shift-c", "User name"));
    }
}

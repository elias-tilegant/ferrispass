//! The list row: one entry, the same in the quick panel, the command
//! palette and the main window's list.
//!
//! A row shows the type tile, the title with at most one chip, a second
//! line with the customer, the client and the user, and the vault badge at
//! its end. In the panel it also shows the keys that select it, ⌘1 to ⌘9,
//! and a ghost button with its verb while it is selected or under the
//! pointer.
//!
//! For VoiceOver a row is one node, an option in a list box, with its place
//! in the list, whether it is selected, and a name that puts what the row
//! shows into words: [`RowParts::accessible_name`]. The tile, the chip, the
//! badge and the texts write no node of their own; the ghost button is a
//! button inside the option.
//!
//! The row draws the selection it is given and never changes it. The
//! pointer over a row changes its color and shows its verb, nothing else.
//! The row does not report itself as the focused element of the tree: how
//! the list lets VoiceOver hear the selected row while the search field
//! keeps focus is decided with the list.

use gpui::{
    App, Div, ElementId, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, Role, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _, px,
};

use super::focus_ring::{Outline, keyboard_focus_ring};
use super::keycap::{keys_text, parse_keys};
use super::{AccessibilityId, Button, Chip, Tile, VaultBadge};
use crate::ui::ds::ActiveTokens as _;
use crate::ui::ds::tokens::{ColorSet, dimension};
use crate::ui::ds::typography::{TextStyle, TextStyled as _, u};

/// Between the parts of a row's name. A comma reads the same in every
/// language the app speaks.
const NAME_SEPARATOR: &str = ", ";

/// Between the parts of the second line.
const LINE_SEPARATOR: &str = " · ";

/// What a row says about its entry, each part in the words VoiceOver reads:
/// from a text key through `tr!`, or from the entry itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowParts {
    kind_word: SharedString,
    title: SharedString,
    role_word: Option<SharedString>,
    customer: Option<SharedString>,
    client: Option<SharedString>,
    user: Option<SharedString>,
    vault: Option<SharedString>,
}

impl RowParts {
    /// `kind_word` is the entry's kind as a word, such as
    /// `tr!("glossary.type.sap_system")`, and `title` is the entry's title,
    /// which the row shows.
    pub fn new(kind_word: impl Into<SharedString>, title: impl Into<SharedString>) -> Self {
        Self {
            kind_word: kind_word.into(),
            title: title.into(),
            ..Self::default()
        }
    }

    /// The system's role as a word, such as "Production system". The row
    /// takes the word as it comes and changes nothing about it, so the
    /// caller passes the word, not the letters of a chip such as "PRD".
    pub fn role_word(mut self, word: impl Into<SharedString>) -> Self {
        self.role_word = Some(word.into());
        self
    }

    pub fn customer(mut self, customer: impl Into<SharedString>) -> Self {
        self.customer = Some(customer.into());
        self
    }

    /// The client as it is read and shown, such as "Client 100".
    pub fn client(mut self, client: impl Into<SharedString>) -> Self {
        self.client = Some(client.into());
        self
    }

    pub fn user(mut self, user: impl Into<SharedString>) -> Self {
        self.user = Some(user.into());
        self
    }

    /// The vault's name, such as "Team Beratung".
    pub fn vault(mut self, vault: impl Into<SharedString>) -> Self {
        self.vault = Some(vault.into());
        self
    }

    /// The name VoiceOver reads for the entry: the kind and the title, then
    /// the role, the customer, the client, the user and the vault, each
    /// after a comma, as in "SAP system NHP 100, Production system,
    /// Nordhafen AG, Client 100, N.WEISS, Team Beratung". A part that is
    /// missing or empty is left out, without a placeholder and without an
    /// empty comma.
    ///
    /// The row's node takes this name, and any other element that stands
    /// for an entry can take it too.
    pub fn accessible_name(&self) -> SharedString {
        let head = words([&self.kind_word, &self.title]).join(" ");
        let rest = [
            &self.role_word,
            &self.customer,
            &self.client,
            &self.user,
            &self.vault,
        ];
        let mut parts = words(rest.into_iter().flatten());
        if !head.is_empty() {
            parts.insert(0, head);
        }
        parts.join(NAME_SEPARATOR).into()
    }

    /// The second line, the customer, the client and the user between
    /// middle dots; a row of one line has room for the customer and the
    /// user. `None` when there is nothing to show.
    fn second_line(&self, density: RowDensity) -> Option<SharedString> {
        let parts = if density.two_lines() {
            vec![&self.customer, &self.client, &self.user]
        } else {
            vec![&self.customer, &self.user]
        };
        let line = words(parts.into_iter().flatten()).join(LINE_SEPARATOR);
        (!line.is_empty()).then(|| line.into())
    }
}

/// The parts that hold a word, trimmed.
fn words<'a>(parts: impl IntoIterator<Item = &'a SharedString>) -> Vec<String> {
    parts
        .into_iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

/// How tall a row is, and whether its second line has a line of its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowDensity {
    /// 44 points, two lines, in the main window's list.
    #[default]
    Comfortable44,
    /// 28 points, one line, in the main window's list.
    Compact28,
    /// 44 points, two lines, in the quick panel.
    Panel44,
    /// 32 points, one line, in the quick panel.
    Panel32,
}

impl RowDensity {
    pub const ALL: [RowDensity; 4] = [
        Self::Comfortable44,
        Self::Compact28,
        Self::Panel44,
        Self::Panel32,
    ];

    /// The height in points at text size 13.
    pub fn height(self) -> f32 {
        match self {
            Self::Comfortable44 => dimension::HEIGHT_ROW,
            Self::Compact28 => dimension::HEIGHT_ROW_COMPACT,
            Self::Panel44 => dimension::HEIGHT_ROW_PANEL,
            Self::Panel32 => dimension::HEIGHT_ROW_PANEL_COMPACT,
        }
    }

    fn two_lines(self) -> bool {
        matches!(self, Self::Comfortable44 | Self::Panel44)
    }
}

/// One entry in a list.
#[derive(IntoElement)]
pub struct Row {
    id: ElementId,
    index: usize,
    count: usize,
    parts: RowParts,
    tile: Tile,
    chip: Option<Chip>,
    badge: VaultBadge,
    selected: bool,
    focused: bool,
    shortcut_index: Option<u8>,
    description: Option<SharedString>,
    density: RowDensity,
    discreet: bool,
    verb: Option<Button>,
    accessibility_id: Option<AccessibilityId>,
}

impl Row {
    /// The row at `index`, counted from 0, in a list of `count` rows. The
    /// tile and the badge are the ones of the entry's kind and vault; they
    /// write no node, since `parts` puts what they show into the row's
    /// name.
    pub fn new(
        id: impl Into<ElementId>,
        index: usize,
        count: usize,
        parts: RowParts,
        tile: Tile,
        badge: VaultBadge,
    ) -> Self {
        debug_assert!(index < count, "row {index} in a list of {count}");
        Self {
            id: id.into(),
            index,
            count,
            parts,
            tile,
            chip: None,
            badge,
            selected: false,
            focused: true,
            shortcut_index: None,
            description: None,
            density: RowDensity::default(),
            discreet: false,
            verb: None,
            accessibility_id: None,
        }
    }

    /// The chip after the title, at most one per row.
    pub fn chip(mut self, chip: Chip) -> Self {
        self.chip = Some(chip);
        self
    }

    /// Whether this is the selected row. The list sets it; nothing in the
    /// row changes it.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Whether the list has focus, such as while the panel's search field
    /// holds it, which is the default. A selected row without it takes
    /// `row_selected_unfocused_bg`, and shows no focus ring.
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// The number that selects the row with ⌘, 1 to 9. The row shows the
    /// keys and puts them into `aria_keyshortcuts`; the panel binds them.
    pub fn shortcut_index(mut self, number: u8) -> Self {
        debug_assert!((1..=9).contains(&number), "⌘{number} selects no row");
        self.shortcut_index = Some(number);
        self
    }

    /// What VoiceOver reads after the name, such as the panel footer's
    /// sentence on the selected row.
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn density(mut self, density: RowDensity) -> Self {
        self.density = density;
        self
    }

    /// A discreet row has no second line, so the customer and the user stay
    /// off the screen.
    pub fn discreet(mut self, discreet: bool) -> Self {
        self.discreet = discreet;
        self
    }

    /// A ghost button with the row's verb, shown while the row is selected
    /// or under the pointer.
    pub fn verb(mut self, verb: Button) -> Self {
        self.verb = Some(verb);
        self
    }

    pub fn accessibility_id(mut self, id: AccessibilityId) -> Self {
        self.accessibility_id = Some(id);
        self
    }
}

/// The title, in `Headline` on the selected row and in `RowTitle` on the
/// others, cut off at its end where it runs out of room.
fn title(text: SharedString, selected: bool, colors: &ColorSet) -> Div {
    div()
        .debug_selector(|| "fp-row-title".into())
        .min_w(px(0.))
        .truncate()
        .text_style(if selected {
            TextStyle::Headline
        } else {
            TextStyle::RowTitle
        })
        .text_color(colors.text_primary.hsla)
        .child(text)
}

/// The second line, in `Callout`: `text_primary` on the selected row and
/// `text_secondary` on the others, cut off in the middle.
fn second_line(text: SharedString, selected: bool, colors: &ColorSet) -> Div {
    let color = if selected {
        colors.text_primary
    } else {
        colors.text_secondary
    };
    div()
        .debug_selector(|| "fp-row-second-line".into())
        .min_w(px(0.))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis_middle()
        .text_style(TextStyle::Callout)
        .text_color(color.hsla)
        .child(text)
}

/// The surface of a row: none, `row_selected_bg` when selected while the
/// list has focus, `row_selected_unfocused_bg` when selected without it.
fn surface(selected: bool, focused: bool, colors: &ColorSet) -> Option<Hsla> {
    if !selected {
        return None;
    }
    let surface = if focused {
        colors.row_selected_bg
    } else {
        colors.row_selected_unfocused_bg
    };
    Some(surface.hsla)
}

/// The keys that select a row, as a hint in `Caption` and `text_tertiary`.
fn keys_hint(keys: SharedString, colors: &ColorSet) -> Div {
    div()
        .debug_selector(|| "fp-row-keys".into())
        .flex_none()
        .text_style(TextStyle::Caption)
        .text_color(colors.text_tertiary.hsla)
        .child(keys)
}

/// What a row draws inside its node, each part as the row builds it: the
/// tile, the title, the chip, the second line, the vault badge, the keys
/// that select the row, and the verb while it shows. None of them but the
/// verb has an element id or a role, so GPUI writes no node for them and
/// the row's name says what they show; the verb is a button inside the
/// option. The title, the second line and the keys are plain `Div`s: GPUI
/// gives a role only to an element with an id, a `Stateful<Div>`, so a
/// node on one of them does not compile here.
struct Inside {
    tile: Tile,
    title: Div,
    chip: Option<Chip>,
    second_line: Option<Div>,
    badge: VaultBadge,
    keys: Option<Div>,
    verb: Option<Button>,
}

impl Row {
    /// The keys that select the row, ⌘1 to ⌘9, as a Mac prints them.
    fn keys(&self) -> Option<SharedString> {
        self.shortcut_index
            .filter(|number| (1..=9).contains(number))
            .map(|number| keys_text(&parse_keys(&format!("cmd-{number}"))))
    }

    /// What the row draws inside its node. The verb shows while the row is
    /// selected or `pointer_over` it.
    fn inside(self, pointer_over: bool, colors: &ColorSet) -> Inside {
        let selected = self.selected;
        let keys = self.keys().map(|keys| keys_hint(keys, colors));
        let second = (!self.discreet)
            .then(|| self.parts.second_line(self.density))
            .flatten()
            .map(|text| second_line(text, selected, colors));
        Inside {
            tile: self.tile.selected(selected),
            title: title(self.parts.title, selected, colors),
            chip: self.chip,
            second_line: second,
            badge: self.badge,
            keys,
            verb: self.verb.filter(|_| selected || pointer_over),
        }
    }
}

/// Puts what a row draws inside its node in its place: the tile, then the
/// title with the chip and the second line, on two lines or on one, then
/// the badge, the keys and the verb.
fn lay_out(row: Stateful<Div>, inside: Inside, density: RowDensity) -> Stateful<Div> {
    let title_line = div()
        .flex()
        .items_center()
        .gap(u(dimension::SPACE_6))
        .min_w(px(0.))
        .child(inside.title);
    let text = if density.two_lines() {
        div()
            .flex()
            .flex_col()
            .justify_center()
            .flex_1()
            .min_w(px(0.))
            .child(title_line.children(inside.chip))
            .children(inside.second_line)
    } else {
        div()
            .flex()
            .items_center()
            .gap(u(dimension::SPACE_8))
            .flex_1()
            .min_w(px(0.))
            .child(title_line)
            .children(inside.second_line)
            .children(inside.chip)
    };
    row.child(inside.tile)
        .child(text)
        .child(inside.badge)
        .children(inside.keys)
        .children(inside.verb.map(|verb| {
            div()
                .debug_selector(|| "fp-row-verb".into())
                .flex_none()
                .child(verb)
        }))
}

impl RenderOnce for Row {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.ds().colors;
        // Whether the pointer is over the row. It shows the verb and
        // nothing else; the selection stays the one the list gave.
        let hovered = window.use_keyed_state((self.id.clone(), "hovered"), cx, |_, _| false);
        let pointer_over = *hovered.read(cx);
        let selected = self.selected;
        let ring = keyboard_focus_ring(selected && self.focused, Outline::Inset, window, cx);

        let row = div()
            .id(self.id.clone())
            .role(Role::ListBoxOption)
            .aria_label(self.parts.accessible_name())
            .aria_selected(selected)
            .aria_position_in_set(self.index + 1)
            .aria_size_of_set(self.count)
            .when_some(self.description.clone(), |this, text| {
                this.aria_description(text)
            })
            .when_some(self.keys(), |this, keys| this.aria_keyshortcuts(keys))
            .on_hover(move |over, _, cx| {
                hovered.update(cx, |state, cx| {
                    if *state != *over {
                        *state = *over;
                        cx.notify();
                    }
                });
            })
            .debug_selector(|| "fp-row".into())
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(u(dimension::SPACE_8))
            .w_full()
            .h(u(self.density.height()))
            .px(u(dimension::SPACE_8))
            .rounded(px(dimension::RADIUS_MD))
            .map(|this| match surface(selected, self.focused, colors) {
                Some(surface) => this.bg(surface),
                None => {
                    let hover = colors.bg_hover.hsla;
                    this.hover(move |style| style.bg(hover))
                }
            });
        let row = match &self.accessibility_id {
            Some(id) => id.apply(row),
            None => row,
        };
        let density = self.density;
        lay_out(row, self.inside(pointer_over, colors), density).children(ring)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{
        AbsoluteLength, Bounds, Modifiers, Pixels, Styled, TestAppContext, VisualTestContext,
        accesskit, point,
    };
    use gpui_component::IconName;

    use super::super::{ChipTone, probe};
    use super::*;

    fn text(value: &'static str) -> SharedString {
        SharedString::new_static(value)
    }

    /// The example of the naming rule, in English.
    fn nhp() -> RowParts {
        RowParts::new(text("SAP system"), text("NHP 100"))
            .role_word(text("Production system"))
            .customer(text("Nordhafen AG"))
            .client(text("Client 100"))
            .user(text("N.WEISS"))
            .vault(text("Team Beratung"))
    }

    const NHP_NAME: &str =
        "SAP system NHP 100, Production system, Nordhafen AG, Client 100, N.WEISS, Team Beratung";

    fn row(index: usize, count: usize) -> Row {
        Row::new(
            ("row", index),
            index,
            count,
            nhp(),
            Tile::new(IconName::Globe),
            VaultBadge::team(text("B"), crate::ui::ds::tokens::LIGHT.team_teal.hsla),
        )
        .chip(Chip::new(ChipTone::Production, text("PRD")))
    }

    /// The node the row writes, and its element id.
    fn row_node(
        cx: &mut TestAppContext,
        build: impl Fn() -> Row + 'static,
    ) -> (Option<Role>, accesskit::Node, Option<ElementId>) {
        probe::in_frame(cx, move |window, cx| {
            let element = RenderOnce::render(build(), window, cx).into_element();
            let (role, node) = probe::node_of(&element);
            (role, node, probe::element_id(element))
        })
    }

    fn near(a: Bounds<Pixels>, b: Bounds<Pixels>) -> bool {
        [
            (a.left(), b.left()),
            (a.top(), b.top()),
            (a.right(), b.right()),
            (a.bottom(), b.bottom()),
        ]
        .into_iter()
        .all(|(x, y)| (x - y).abs() <= px(1.))
    }

    /// The surface painted in the bounds of the element marked `selector`.
    fn surface_of(cx: &mut VisualTestContext, selector: &'static str) -> Option<Hsla> {
        let bounds = cx.debug_bounds(selector).expect(selector);
        cx.update(|window, _| {
            let scale = window.scale_factor();
            window
                .painted_quads()
                .into_iter()
                .filter(|quad| near(quad.bounds.map(|value| px(value.0 / scale)), bounds))
                .filter_map(|quad| quad.background.as_solid())
                .find(|color| !color.is_transparent())
        })
    }

    /// A window that draws `build`'s row and keeps the node of every row it
    /// drew, read from the element it drew.
    fn drawn_rows(
        cx: &mut TestAppContext,
        build: impl Fn() -> Row + 'static,
    ) -> (&mut VisualTestContext, Rc<RefCell<Vec<accesskit::Node>>>) {
        let nodes = Rc::new(RefCell::new(Vec::new()));
        let seen = nodes.clone();
        let cx = probe::window(cx, move |window, cx| {
            let element = RenderOnce::render(build(), window, cx).into_element();
            seen.borrow_mut().push(probe::node_of(&element).1);
            element.into_any_element()
        });
        (cx, nodes)
    }

    fn redraw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    /// Below the rows, which the windows draw at their top. A test window
    /// starts with the pointer at its corner, over the first row.
    fn below_the_rows() -> gpui::Point<Pixels> {
        point(px(1.), px(400.))
    }

    /// Moves the pointer off the rows and draws the window again.
    fn pointer_away(cx: &mut VisualTestContext) {
        cx.simulate_mouse_move(below_the_rows(), None, Modifiers::default());
        redraw(cx);
    }

    /// A row is an option in a list box: its node has the role, the name
    /// from its parts, its place in the list counted from 1, the size of
    /// the list, whether it is selected, and the ⌘ number that selects it.
    /// The selected row says it is selected while its list has no focus
    /// too.
    #[gpui::test]
    fn a_row_is_a_list_box_option_with_its_place_in_the_set(cx: &mut TestAppContext) {
        let (role, node, id) = row_node(cx, || row(2, 7).selected(true).shortcut_index(3));
        assert_eq!(role, Some(Role::ListBoxOption));
        assert_eq!(
            id,
            Some(ElementId::from(("row", 2usize))),
            "an id, so it reaches the tree"
        );
        assert_eq!(node.label(), Some(NHP_NAME));
        assert_eq!(node.position_in_set(), Some(3), "counted from 1");
        assert_eq!(node.size_of_set(), Some(7));
        assert_eq!(node.is_selected(), Some(true));
        assert_eq!(node.keyboard_shortcut(), Some("⌘3"));

        let (_, node, _) = row_node(cx, || row(2, 7).selected(true).focused(false));
        assert_eq!(
            node.is_selected(),
            Some(true),
            "selected while the list has no focus"
        );

        let (_, node, _) = row_node(cx, || row(0, 1));
        assert_eq!(node.position_in_set(), Some(1));
        assert_eq!(node.size_of_set(), Some(1));
        assert_eq!(node.is_selected(), Some(false), "not selected says so");
        assert_eq!(node.keyboard_shortcut(), None, "no keys without a number");

        let (_, node, _) = row_node(cx, || row(0, 1).focused(false));
        assert_eq!(node.is_selected(), Some(false));
    }

    /// The naming rule: the kind and the title, then the role, the
    /// customer, the client, the user and the vault, after commas. A part
    /// that is missing or holds no word is left out without a trace.
    #[test]
    fn accessible_name_follows_the_naming_pattern() {
        let sap = || RowParts::new(text("SAP-System"), text("NHP 100"));
        for (parts, name) in [
            (
                sap()
                    .role_word(text("Produktivsystem"))
                    .customer(text("Nordhafen AG"))
                    .client(text("Mandant 100"))
                    .user(text("N.WEISS"))
                    .vault(text("Team Beratung")),
                "SAP-System NHP 100, Produktivsystem, Nordhafen AG, Mandant 100, N.WEISS, Team Beratung",
            ),
            (nhp(), NHP_NAME),
            (
                sap()
                    .customer(text("Nordhafen AG"))
                    .client(text("Mandant 100"))
                    .user(text("N.WEISS"))
                    .vault(text("Team Beratung")),
                "SAP-System NHP 100, Nordhafen AG, Mandant 100, N.WEISS, Team Beratung",
            ),
            (
                RowParts::new(text("Web login"), text("Salesforce"))
                    .user(text("petra.engel@example.com"))
                    .vault(text("Team Vertrieb")),
                "Web login Salesforce, petra.engel@example.com, Team Vertrieb",
            ),
            (
                RowParts::new(text("Note"), text("Door codes")),
                "Note Door codes",
            ),
            // Empty and blank parts are missing parts.
            (
                sap()
                    .role_word(text(""))
                    .customer(text("  "))
                    .client(text("Mandant 100"))
                    .user(text(""))
                    .vault(text(" Team Beratung ")),
                "SAP-System NHP 100, Mandant 100, Team Beratung",
            ),
            (
                RowParts::new(text("Note"), text("")).vault(text("Personal")),
                "Note, Personal",
            ),
            (
                RowParts::new(text(""), text("NHP 100")).user(text("N.WEISS")),
                "NHP 100, N.WEISS",
            ),
            (RowParts::new(text(""), text("")), ""),
        ] {
            assert_eq!(parts.accessible_name(), name);
        }
    }

    /// The row puts the role word into the name as the caller gives it and
    /// adds nothing of its own: not the letters on its chip, whatever the
    /// chip says.
    #[gpui::test]
    fn accessible_name_takes_the_role_word_as_given(cx: &mut TestAppContext) {
        for word in ["Production system", "Produktivsystem", "Production"] {
            let parts = RowParts::new(text("SAP system"), text("NHP 100")).role_word(word);
            assert_eq!(
                parts.accessible_name().to_string(),
                format!("SAP system NHP 100, {word}")
            );
        }
        let (_, node, _) = row_node(cx, || row(0, 3).selected(true));
        assert_eq!(node.label(), Some(NHP_NAME), "the name is the parts' name");
        for (part, said) in [
            ("label", node.label()),
            ("value", node.value()),
            ("description", node.description()),
        ] {
            assert!(
                !said.is_some_and(|said| said.contains("PRD")),
                "the {part} spells the chip's letters: {said:?}"
            );
        }
    }

    /// The footer's sentence reaches VoiceOver as the row's description;
    /// without one the row has none.
    #[gpui::test]
    fn a_row_description_carries_the_footer_text(cx: &mut TestAppContext) {
        const FOOTER: &str = "Return: sign in to NHP, client 100, as N.WEISS. Command C: password.";
        let (_, node, _) = row_node(cx, || row(0, 2).selected(true).description(text(FOOTER)));
        assert_eq!(node.description(), Some(FOOTER));
        let (_, node, _) = row_node(cx, || row(1, 2));
        assert_eq!(node.description(), None);
    }

    /// The pointer over a row gives it `bg_hover` and shows its verb, and
    /// leaves the selection as it was: the node still says not selected,
    /// and neither the row nor its tile takes a color of the selection.
    /// When the pointer leaves, so do the color and the verb.
    #[gpui::test]
    fn hover_changes_the_color_not_the_selection(cx: &mut TestAppContext) {
        let (cx, nodes) = drawn_rows(cx, || {
            row(0, 2).verb(Button::ghost("sign-in", text("Sign in")))
        });
        let colors = cx.update(|_, cx| cx.ds().colors);
        pointer_away(cx);
        assert_eq!(surface_of(cx, "fp-row"), None, "no surface yet");
        assert_eq!(cx.debug_bounds("fp-row-verb"), None, "no verb yet");

        let over = cx.debug_bounds("fp-row").expect("the row").center();
        cx.simulate_mouse_move(over, None, Modifiers::default());
        redraw(cx);
        assert_eq!(
            surface_of(cx, "fp-row"),
            Some(colors.bg_hover.hsla),
            "the hover color"
        );
        assert!(cx.debug_bounds("fp-row-verb").is_some(), "the verb shows");
        assert_eq!(surface_of(cx, "fp-tile"), Some(colors.tile_bg.hsla));
        let node = nodes.borrow().last().cloned().expect("a drawn row");
        assert_eq!(node.is_selected(), Some(false), "hover selects nothing");

        pointer_away(cx);
        assert_eq!(surface_of(cx, "fp-row"), None, "the color goes");
        assert_eq!(cx.debug_bounds("fp-row-verb"), None, "and the verb");
        let node = nodes.borrow().last().cloned().expect("a drawn row");
        assert_eq!(node.is_selected(), Some(false));
    }

    /// The selected row sets its title in `Headline` and its second line in
    /// `text_primary`, the others in `RowTitle` and `text_secondary`. It
    /// takes `row_selected_bg`, or `row_selected_unfocused_bg` while the
    /// list has no focus, its tile `tile_selected_bg`, and it shows its
    /// verb without the pointer.
    #[gpui::test]
    fn a_selected_row_title_is_headline_the_others_row_title(cx: &mut TestAppContext) {
        let colors = &crate::ui::ds::tokens::LIGHT;
        for (selected, style, line) in [
            (true, TextStyle::Headline, colors.text_primary.hsla),
            (false, TextStyle::RowTitle, colors.text_secondary.hsla),
        ] {
            let mut heading = title(text("NHP 100"), selected, colors);
            let heading = Styled::text_style(&mut heading).clone();
            assert_eq!(
                heading.font_weight,
                Some(style.weight()),
                "selected: {selected}"
            );
            assert_eq!(
                heading.font_size,
                Some(AbsoluteLength::Rems(u(style.size()))),
                "selected: {selected}"
            );
            let mut second = second_line(text("Nordhafen AG"), selected, colors);
            let second = Styled::text_style(&mut second).clone();
            assert_eq!(second.color, Some(line), "selected: {selected}");
        }

        for (focused, expected) in [
            (true, colors.row_selected_bg.hsla),
            (false, colors.row_selected_unfocused_bg.hsla),
        ] {
            let cx = probe::window(cx, move |_, _| {
                row(0, 2)
                    .selected(true)
                    .focused(focused)
                    .verb(Button::ghost("sign-in", text("Sign in")))
                    .into_any_element()
            });
            assert_eq!(
                surface_of(cx, "fp-row"),
                Some(expected),
                "focused: {focused}"
            );
            assert_eq!(
                surface_of(cx, "fp-tile"),
                Some(colors.tile_selected_bg.hsla),
                "focused: {focused}"
            );
            assert!(cx.debug_bounds("fp-row-verb").is_some(), "the verb shows");
        }
        let cx = probe::window(cx, |_, _| row(1, 2).into_any_element());
        pointer_away(cx);
        assert_eq!(surface_of(cx, "fp-row"), None);
        assert_eq!(surface_of(cx, "fp-tile"), Some(colors.tile_bg.hsla));
    }

    /// A discreet row shows no second line, in every density: the customer
    /// and the user stay off the screen. A row with nothing for its second
    /// line has none either.
    #[gpui::test]
    fn a_discreet_row_has_no_second_line(cx: &mut TestAppContext) {
        for density in RowDensity::ALL {
            for discreet in [false, true] {
                let cx = probe::window(cx, move |_, _| {
                    row(0, 1)
                        .density(density)
                        .discreet(discreet)
                        .into_any_element()
                });
                assert_eq!(
                    cx.debug_bounds("fp-row-second-line").is_some(),
                    !discreet,
                    "{density:?}, discreet: {discreet}"
                );
                assert!(cx.debug_bounds("fp-row-title").is_some(), "{density:?}");
            }
        }
        let cx = probe::window(cx, |_, _| {
            Row::new(
                "note",
                0,
                1,
                RowParts::new(text("Note"), text("Door codes")).vault(text("Personal")),
                Tile::new(IconName::Globe),
                VaultBadge::Personal,
            )
            .into_any_element()
        });
        assert_eq!(cx.debug_bounds("fp-row-second-line"), None);
        assert_eq!(
            RowParts::new(text("Note"), text("Door codes")).second_line(RowDensity::Panel44),
            None
        );
        assert_eq!(
            nhp().second_line(RowDensity::Panel44).as_deref(),
            Some("Nordhafen AG · Client 100 · N.WEISS")
        );
        assert_eq!(
            nhp().second_line(RowDensity::Panel32).as_deref(),
            Some("Nordhafen AG · N.WEISS"),
            "one line has room for the customer and the user"
        );
    }

    /// Rows are 44 points high in the main window's list and 28 in its
    /// compact form, 44 or 32 in the panel, at text size 13.
    #[gpui::test]
    fn rows_are_44_28_44_or_32_high(cx: &mut TestAppContext) {
        for (density, points) in [
            (RowDensity::Comfortable44, 44.),
            (RowDensity::Compact28, 28.),
            (RowDensity::Panel44, 44.),
            (RowDensity::Panel32, 32.),
        ] {
            assert_eq!(density.height(), points, "{density:?}");
            let cx = probe::window(cx, move |_, _| {
                row(0, 1).density(density).into_any_element()
            });
            let bounds = cx.debug_bounds("fp-row").expect("the row");
            // Layout snaps to device pixels.
            let height = cx.update(|window, _| u(points).to_pixels(window.rem_size()));
            assert!(
                (bounds.size.height - height).abs() <= px(1.),
                "{density:?}: {:?}, not {height:?}",
                bounds.size.height
            );
        }
    }

    /// The id the caller gives lands in the node as its author id, the
    /// identifier UI tests read; without one the row has none.
    #[gpui::test]
    fn a_row_applies_the_id_it_was_given(cx: &mut TestAppContext) {
        let (_, node, _) = row_node(cx, || {
            row(3, 5).accessibility_id(AccessibilityId::new("quick", "row", 3))
        });
        assert_eq!(node.author_id(), Some("fp.quick.row.3"));
        let (_, node, _) = row_node(cx, || row(3, 5));
        assert_eq!(node.author_id(), None);
    }

    /// While the keyboard is in use, the selected row of a list with focus
    /// shows the focus ring inside its edge, where a list that scrolls
    /// cannot cut it off. A row that is not selected, or whose list has no
    /// focus, shows none, and neither does the selected row once the
    /// pointer is used.
    #[gpui::test]
    fn a_selected_row_shows_the_ring_inside_while_the_keyboard_is_in_use(cx: &mut TestAppContext) {
        for (selected, focused) in [(true, true), (false, true), (true, false)] {
            let cx = probe::window(cx, move |_, _| {
                row(0, 1)
                    .selected(selected)
                    .focused(focused)
                    .into_any_element()
            });
            assert_eq!(cx.debug_bounds("fp-focus-ring"), None, "no input yet");
            cx.simulate_keystrokes("down");
            redraw(cx);
            let ring = cx.debug_bounds("fp-focus-ring");
            if selected && focused {
                let row = cx.debug_bounds("fp-row").expect("the row");
                assert_eq!(ring, Some(row), "the ring lies inside the row's edge");
                cx.simulate_click(below_the_rows(), Modifiers::default());
                redraw(cx);
                assert_eq!(
                    cx.debug_bounds("fp-focus-ring"),
                    None,
                    "no ring after a click"
                );
            } else {
                assert_eq!(ring, None, "selected: {selected}, focused: {focused}");
            }
        }
    }
}

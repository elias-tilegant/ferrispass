//! The pages of the gallery: one for each component of the design system,
//! with every state it offers side by side.
//!
//! A page calls the components and draws nothing of its own but frames and
//! captions. A caption names a component or a state, in English, as the
//! code names it. What the components show comes from the text keys where
//! the app has one, so it follows the language; the rest is made-up sample
//! data. The one secret-looking value is [`SECRET`], and the codes of the
//! one-time code ring are [`CODE`] and [`NEXT_CODE`]: none of them opens
//! anything.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use ferrispass::app::registry::{self, CommandId};
use ferrispass::ui::ds::ActiveTokens as _;
use ferrispass::ui::ds::components::{
    Button, ButtonKind, ButtonSize, Chip, ChipTone, CopyState, FooterTone, IconButton, OtpRing,
    OtpSize, PanelFooter, ResultList, Row, RowDensity, RowParts, SearchGroup, SecretField,
    Shortcut, Switch, TextField, Tile, VaultBadge, initial_for,
};
use ferrispass::ui::ds::icons::FpIcon;
use ferrispass::ui::ds::tokens::dimension;
use ferrispass::ui::ds::typography::{TextStyle, TextStyled as _, u};
use ferrispass::{tp, tr};
use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Div, ElementId, Entity, InteractiveElement as _,
    IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _, Task, Window, div,
};
use gpui_component::input::InputState;
use zeroize::Zeroizing;

/// The gallery's test secret. It looks like a password and is none.
pub const SECRET: &str = "gallery-9Qz!2026";

/// The one-time code the ring shows, and the one after it. Neither is a
/// code of any account.
pub const CODE: &str = "123456";
pub const NEXT_CODE: &str = "654321";

/// The vault the sample entries are in.
const VAULT: &str = "Team Beratung";

/// How long a copy flashes the value, and how long the tick stays after it,
/// in the secret field that answers clicks.
const FLASH: Duration = Duration::from_millis(200);
const TICK: Duration = Duration::from_millis(1500);

/// One page of the gallery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Button,
    IconButton,
    Shortcut,
    TextField,
    Switch,
    Tile,
    Chip,
    VaultBadge,
    Row,
    ResultList,
    SecretField,
    OneTimeCode,
    PanelFooter,
}

impl Page {
    /// The pages in the order of the list.
    pub const ALL: &'static [Page] = &[
        Page::Button,
        Page::IconButton,
        Page::Shortcut,
        Page::TextField,
        Page::Switch,
        Page::Tile,
        Page::Chip,
        Page::VaultBadge,
        Page::Row,
        Page::ResultList,
        Page::SecretField,
        Page::OneTimeCode,
        Page::PanelFooter,
    ];

    /// The page's name in the list.
    pub fn name(self) -> &'static str {
        match self {
            Page::Button => "button",
            Page::IconButton => "icon button",
            Page::Shortcut => "shortcut pair",
            Page::TextField => "text field",
            Page::Switch => "switch",
            Page::Tile => "type tile",
            Page::Chip => "chip",
            Page::VaultBadge => "vault badge",
            Page::Row => "row",
            Page::ResultList => "result list",
            Page::SecretField => "secret field",
            Page::OneTimeCode => "one-time code ring",
            Page::PanelFooter => "panel footer",
        }
    }

    /// The modules under `src/ui/ds/components/` whose components the page
    /// shows.
    pub fn modules(self) -> &'static [&'static str] {
        match self {
            Page::Button | Page::IconButton => &["button"],
            Page::Shortcut => &["shortcut"],
            Page::TextField => &["text_field"],
            Page::Switch => &["switch"],
            Page::Tile => &["tile"],
            Page::Chip => &["chip"],
            Page::VaultBadge => &["vault_badge"],
            Page::Row => &["row"],
            Page::ResultList => &["result_list", "search_group"],
            Page::SecretField => &["secret_field"],
            Page::OneTimeCode => &["otp"],
            Page::PanelFooter => &["panel_footer"],
        }
    }

    /// The debug selector of the page's outer element, by which a test
    /// finds the page in a drawn frame.
    pub fn selector(self) -> &'static str {
        match self {
            Page::Button => "gallery-button",
            Page::IconButton => "gallery-icon-button",
            Page::Shortcut => "gallery-shortcut",
            Page::TextField => "gallery-text-field",
            Page::Switch => "gallery-switch",
            Page::Tile => "gallery-tile",
            Page::Chip => "gallery-chip",
            Page::VaultBadge => "gallery-vault-badge",
            Page::Row => "gallery-row",
            Page::ResultList => "gallery-result-list",
            Page::SecretField => "gallery-secret-field",
            Page::OneTimeCode => "gallery-one-time-code",
            Page::PanelFooter => "gallery-panel-footer",
        }
    }
}

/// What the pages keep between frames: the text of the text fields, which
/// the Kit's inputs hold, and the states the components that answer clicks
/// are in.
pub struct Samples {
    empty: Entity<InputState>,
    placeholder: Entity<InputState>,
    filled: Entity<InputState>,
    wrong: Entity<InputState>,
    disabled: Entity<InputState>,
    search: Entity<InputState>,
    search_empty: Entity<InputState>,
    switch_on: Rc<Cell<bool>>,
    copied: Rc<Cell<CopyState>>,
    copy_timer: Rc<RefCell<Option<Task<()>>>>,
    revealed: Rc<Cell<bool>>,
}

impl Samples {
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        let samples = Self {
            empty: cx.new(|cx| InputState::new(window, cx)),
            placeholder: cx.new(|cx| InputState::new(window, cx)),
            filled: cx.new(|cx| InputState::new(window, cx).default_value("N.WEISS")),
            wrong: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .default_value(SECRET)
            }),
            disabled: cx.new(|cx| InputState::new(window, cx).default_value("100")),
            search: cx.new(|cx| InputState::new(window, cx).default_value("nhp")),
            search_empty: cx.new(|cx| InputState::new(window, cx).default_value("xyz")),
            switch_on: Rc::new(Cell::new(true)),
            copied: Rc::new(Cell::new(CopyState::Idle)),
            copy_timer: Rc::new(RefCell::new(None)),
            revealed: Rc::new(Cell::new(false)),
        };
        samples.follow_language(window, cx);
        samples
    }

    /// Puts the placeholder, which the input keeps as it was given, into
    /// the current language.
    pub fn follow_language(&self, window: &mut Window, cx: &mut App) {
        self.placeholder.update(cx, |state, cx| {
            state.set_placeholder(tr!("quick.search.placeholder"), window, cx)
        });
    }
}

/// `page` with its heading.
pub fn page(page: Page, samples: &Samples, cx: &App) -> AnyElement {
    let body = match page {
        Page::Button => buttons(),
        Page::IconButton => icon_buttons(),
        Page::Shortcut => shortcuts(),
        Page::TextField => text_fields(samples),
        Page::Switch => switches(samples),
        Page::Tile => tiles(cx),
        Page::Chip => chips(),
        Page::VaultBadge => vault_badges(cx),
        Page::Row => rows(cx),
        Page::ResultList => result_lists(samples),
        Page::SecretField => secret_fields(samples),
        Page::OneTimeCode => one_time_codes(),
        Page::PanelFooter => panel_footers(),
    };
    let sources = page
        .modules()
        .iter()
        .map(|module| caption(format!("src/ui/ds/components/{module}.rs")));
    div()
        .debug_selector(|| page.selector().to_string())
        .flex()
        .flex_col()
        .gap(u(dimension::SPACE_24))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(u(dimension::SPACE_4))
                .child(div().text_style(TextStyle::Display).child(page.name()))
                .children(sources),
        )
        .child(body)
        .into_any_element()
}

/// A caption: the name of a component or a state.
pub fn caption(text: impl Into<SharedString>) -> Caption {
    Caption(text.into())
}

/// The text of a caption, in `Caption` and `text_secondary`.
#[derive(IntoElement)]
pub struct Caption(SharedString);

impl RenderOnce for Caption {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        div()
            .text_style(TextStyle::Caption)
            .text_color(cx.ds().colors.text_secondary.hsla)
            .child(self.0)
    }
}

// ------------------------------------------------------------- frames --

/// A component above its caption.
fn cell(name: impl Into<SharedString>, element: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(u(dimension::SPACE_4))
        .child(element)
        .child(caption(name))
}

/// Cells side by side under a heading, wrapping where the page ends.
fn section(heading: impl Into<SharedString>, cells: impl IntoIterator<Item = Div>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(u(dimension::SPACE_8))
        .child(div().text_style(TextStyle::Headline).child(heading.into()))
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_start()
                .gap(u(dimension::SPACE_16))
                .children(cells),
        )
}

/// Sections one below the other.
fn sections(sections: impl IntoIterator<Item = Div>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(u(dimension::SPACE_24))
        .children(sections)
}

/// A component in a box `width` points wide, for components that take the
/// width they are given.
fn wide(width: f32, element: impl IntoElement) -> Div {
    div().w(u(width)).child(element)
}

/// A component in a box `width` by `height` points, for components that
/// fill the box they are in.
fn boxed(width: f32, height: f32, element: impl IntoElement) -> Div {
    div().w(u(width)).h(u(height)).child(element)
}

/// A wide component below its caption.
fn titled(name: impl Into<SharedString>, element: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(u(dimension::SPACE_4))
        .child(caption(name))
        .child(element)
}

/// An element id from a name.
fn id(name: impl std::fmt::Display) -> ElementId {
    ElementId::Name(name.to_string().into())
}

/// A name from a variant, as the code writes it, in small letters.
fn named(variant: impl std::fmt::Debug) -> String {
    format!("{variant:?}").to_lowercase()
}

// ------------------------------------------------------------ buttons --

/// What a button of each kind says.
fn button_label(kind: ButtonKind) -> SharedString {
    match kind {
        ButtonKind::Primary => tr!("action.sap_login.label"),
        ButtonKind::Secondary => tr!("common.cancel"),
        ButtonKind::Quiet => tr!("common.later"),
        ButtonKind::Ghost => tr!("action.copy_password.label"),
        ButtonKind::Danger => tr!("action.quit.label"),
        ButtonKind::Large => tr!("quick.locked.unlock_touch_id"),
    }
}

/// Every kind in every size, then disabled, loading, with a shortcut and as
/// the default button, which a danger button never is.
fn buttons() -> Div {
    sections(ButtonKind::ALL.map(|kind| {
        let kind_name = named(kind);
        let button = |state: &str| {
            Button::new(
                id(format_args!("button-{kind_name}-{state}")),
                kind,
                button_label(kind),
            )
        };
        let sizes = ButtonSize::ALL.map(|size| {
            let size_name = format!("{size:?}");
            cell(size_name.clone(), button(&size_name).size(size))
        });
        let default = if kind == ButtonKind::Danger {
            div().child(caption("never the default button"))
        } else {
            cell("default button", button("default").default_button())
        };
        section(
            kind_name.clone(),
            sizes.into_iter().chain([
                cell("disabled", button("disabled").disabled(true)),
                cell("loading", button("loading").loading(true)),
                cell("shortcut", button("shortcut").shortcut("cmd-c")),
                cell(
                    "shortcut, disabled",
                    button("shortcut-disabled").shortcut("cmd-c").disabled(true),
                ),
                default,
            ]),
        )
    }))
}

/// The icon button as it is, disabled, as a switch and as a menu button.
fn icon_buttons() -> Div {
    section(
        "icon button",
        [
            cell(
                "normal",
                IconButton::new("icon-normal", FpIcon::Copy, tr!("a11y.copy_password")),
            ),
            cell(
                "disabled",
                IconButton::new("icon-disabled", FpIcon::Copy, tr!("a11y.copy_password"))
                    .disabled(true),
            ),
            cell(
                "toggled off",
                IconButton::new("icon-off", FpIcon::Eye, tr!("a11y.reveal_password"))
                    .toggled(false),
            ),
            cell(
                "toggled on",
                IconButton::new("icon-on", FpIcon::EyeOff, tr!("a11y.reveal_password"))
                    .toggled(true),
            ),
            cell(
                "menu closed",
                IconButton::new("icon-closed", FpIcon::More, tr!("a11y.more_actions"))
                    .expanded(false),
            ),
            cell(
                "menu open",
                IconButton::new("icon-open", FpIcon::More, tr!("a11y.more_actions")).expanded(true),
            ),
        ],
    )
}

// ----------------------------------------------------------- shortcuts --

/// The pair of every command in the command table, keys and word from the
/// table, and one styled as the footer styles its first line.
fn shortcuts() -> Div {
    sections([
        section(
            "from the command table",
            registry::commands()
                .iter()
                .map(|command| cell(command.id.as_str(), Shortcut::for_command(command.id))),
        ),
        section(
            "styled by the caller",
            [cell(
                "headline",
                Shortcut::for_command(CommandId::ENTRY_PRIMARY).text_style(TextStyle::Headline),
            )],
        ),
    ])
}

// --------------------------------------------------- fields and switches --

/// Empty, with a placeholder, with text, with an error, disabled. The Kit
/// input keeps the text, so each field can be typed into.
fn text_fields(samples: &Samples) -> Div {
    section(
        "text field",
        [
            cell(
                "empty",
                wide(
                    dimension::WIDTH_LIST,
                    TextField::new("field-empty", &samples.empty, tr!("quick.preview.user")),
                ),
            ),
            cell(
                "empty, placeholder",
                wide(
                    dimension::WIDTH_LIST,
                    TextField::new(
                        "field-placeholder",
                        &samples.placeholder,
                        tr!("quick.header.searching_vault", vault = VAULT),
                    ),
                ),
            ),
            cell(
                "text",
                wide(
                    dimension::WIDTH_LIST,
                    TextField::new("field-text", &samples.filled, tr!("quick.preview.user")),
                ),
            ),
            cell(
                "error",
                wide(
                    dimension::WIDTH_LIST,
                    TextField::new(
                        "field-error",
                        &samples.wrong,
                        tr!("glossary.password.master"),
                    )
                    .error(tr!("msg.wrong_master_password")),
                ),
            ),
            cell(
                "disabled",
                wide(
                    dimension::WIDTH_LIST,
                    TextField::new(
                        "field-disabled",
                        &samples.disabled,
                        tr!("glossary.sap_client"),
                    )
                    .disabled(true),
                ),
            ),
        ],
    )
}

/// Off, on, disabled both ways, with a note, and one that answers clicks.
fn switches(samples: &Samples) -> Div {
    let label = tr!("action.quick_access.label");
    let switch_on = samples.switch_on.clone();
    section(
        "switch",
        [
            cell("off", Switch::new("switch-off", label.clone())),
            cell("on", Switch::new("switch-on", label.clone()).checked(true)),
            cell(
                "disabled, off",
                Switch::new("switch-disabled-off", label.clone()).disabled(true),
            ),
            cell(
                "disabled, on",
                Switch::new("switch-disabled-on", label.clone())
                    .checked(true)
                    .disabled(true),
            ),
            cell(
                "with a note",
                wide(
                    dimension::WIDTH_LIST,
                    Switch::new("switch-note", label.clone())
                        .checked(true)
                        .note(tr!("msg.concurrent_change")),
                ),
            ),
            cell(
                "answers clicks",
                Switch::new("switch-live", label)
                    .checked(samples.switch_on.get())
                    .on_change(move |on, window, _| {
                        switch_on.set(*on);
                        window.refresh();
                    }),
            ),
        ],
    )
}

// --------------------------------------------- tiles, chips and badges --

/// The kinds of entry, with their glyph, their word and a sample title.
fn kinds() -> [(FpIcon, SharedString, &'static str); 5] {
    [
        (FpIcon::TypeSap, tr!("glossary.type.sap_system"), "NHP 100"),
        (
            FpIcon::TypeRemote,
            tr!("glossary.type.remote_access"),
            "Nordhafen GP",
        ),
        (
            FpIcon::TypeWeb,
            tr!("glossary.type.web_login"),
            "portal.nordhafen.example",
        ),
        (
            FpIcon::TypeServer,
            tr!("glossary.type.server"),
            "nhp-app-01",
        ),
        (
            FpIcon::TypeNote,
            tr!("glossary.type.secure_note"),
            "Recovery codes",
        ),
    ]
}

/// The tile of every kind on its own, and the selected tile, which only a
/// selected row draws.
fn tiles(cx: &App) -> Div {
    let kinds = kinds();
    let count = kinds.len();
    sections([
        section(
            "on its own",
            kinds.clone().map(|(icon, word, _)| {
                cell(
                    word.clone(),
                    Tile::standalone(id(format_args!("tile-{icon:?}")), icon, word),
                )
            }),
        ),
        section(
            "selected, in a selected row",
            kinds
                .into_iter()
                .enumerate()
                .map(|(index, (icon, word, title))| {
                    wide(
                        dimension::WIDTH_LIST,
                        Row::new(
                            ("tile-row", index),
                            index,
                            count,
                            RowParts::new(word, title),
                            Tile::new(icon),
                            team_badge(cx),
                        )
                        .density(RowDensity::Compact28)
                        .selected(true),
                    )
                }),
        ),
    ])
}

/// What each tone of chip says, and its name for VoiceOver.
fn chip_samples() -> [(ChipTone, SharedString, SharedString, FpIcon); 6] {
    [
        (
            ChipTone::Neutral,
            "QAS".into(),
            tr!("glossary.role.quality_system"),
            FpIcon::Tag,
        ),
        (
            ChipTone::Production,
            "PRD".into(),
            tr!("glossary.role.production_system"),
            FpIcon::Security,
        ),
        (
            ChipTone::Attention,
            tr!("glossary.one_time_code"),
            tr!("glossary.one_time_code"),
            FpIcon::Clock,
        ),
        (
            ChipTone::Decision,
            tr!("glossary.earlier_versions"),
            tr!("glossary.earlier_versions"),
            FpIcon::StatusDecision,
        ),
        (
            ChipTone::Danger,
            tr!("glossary.trash"),
            tr!("glossary.trash"),
            FpIcon::StatusDanger,
        ),
        (
            ChipTone::Patina,
            tr!("quick.footer.copied.password"),
            tr!("quick.footer.copied.password"),
            FpIcon::StatusOk,
        ),
    ]
}

/// Every tone, without and with a glyph.
fn chips() -> Div {
    sections(chip_samples().map(|(tone, text, name, glyph)| {
        let tone_name = named(tone);
        section(
            tone_name.clone(),
            [
                cell(
                    "text",
                    Chip::standalone(
                        id(format_args!("chip-{tone_name}")),
                        tone,
                        text.clone(),
                        name,
                    ),
                ),
                cell("text and glyph", Chip::new(tone, text).glyph(glyph)),
            ],
        )
    }))
}

/// A team's badge in the first team color.
fn team_badge(cx: &App) -> VaultBadge {
    VaultBadge::team(initial_for(VAULT, &[]), cx.ds().colors.team_moss.hsla)
}

/// The personal vault, a team vault in each team color, and any other
/// vault. Two teams whose words start alike take two letters.
fn vault_badges(cx: &App) -> Div {
    let colors = cx.ds().colors;
    let teams = [
        (VAULT, "moss", colors.team_moss.hsla),
        ("Team Vertrieb", "plum", colors.team_plum.hsla),
        ("Team Verwaltung", "slate", colors.team_slate.hsla),
        ("Team Technik", "teal", colors.team_teal.hsla),
    ];
    let names: Vec<String> = teams.iter().map(|(name, _, _)| name.to_string()).collect();
    let team_cells = teams.map(|(name, color_name, color)| {
        let others: Vec<String> = names
            .iter()
            .filter(|other| *other != name)
            .cloned()
            .collect();
        cell(
            format!("team, {color_name}, {name}"),
            VaultBadge::team(initial_for(name, &others), color)
                .standalone(id(format_args!("badge-{name}")), name),
        )
    });
    section(
        "vault badge",
        [cell(
            "personal",
            VaultBadge::Personal.standalone("badge-personal", tr!("glossary.vault.personal")),
        )]
        .into_iter()
        .chain(team_cells)
        .chain([cell(
            "other, kunden-alt",
            VaultBadge::other(initial_for("kunden-alt", &[]))
                .standalone("badge-other", tr!("glossary.vault.other_vault")),
        )]),
    )
}

// --------------------------------------------------------------- rows --

/// The sample SAP system, as a row names it.
fn sap_parts() -> RowParts {
    RowParts::new(tr!("glossary.type.sap_system"), "NHP 100")
        .role_word(tr!("glossary.role.production_system"))
        .customer("Nordhafen AG")
        .client(format!("{} 100", tr!("glossary.sap_client")))
        .user("N.WEISS")
        .vault(VAULT)
}

/// The states of a row, each with what makes it so.
const ROW_STATES: [&str; 5] = [
    "normal, verb under the pointer",
    "selected",
    "selected, list without focus",
    "discreet",
    "selected, with its verb",
];

/// The row of the sample SAP system at `index` of the states, in `density`.
fn state_row(density: RowDensity, index: usize, cx: &App) -> Row {
    let density_name = named(density);
    let verb = || {
        Button::ghost(
            id(format_args!("row-verb-{density_name}-{index}")),
            tr!("action.sap_login.label"),
        )
    };
    let row = Row::new(
        id(format_args!("row-{density_name}-{index}")),
        index,
        ROW_STATES.len(),
        sap_parts(),
        Tile::new(FpIcon::TypeSap),
        team_badge(cx),
    )
    .chip(Chip::new(ChipTone::Production, "PRD"))
    .density(density)
    .shortcut_index(u8::try_from(index + 1).unwrap_or(9));
    match index {
        0 => row.verb(verb()),
        1 => row.selected(true),
        2 => row.selected(true).focused(false),
        3 => row.discreet(true),
        _ => row.selected(true).verb(verb()),
    }
}

/// Every density, each in every state.
fn rows(cx: &App) -> Div {
    sections(RowDensity::ALL.map(|density| {
        div()
            .flex()
            .flex_col()
            .gap(u(dimension::SPACE_12))
            .child(div().text_style(TextStyle::Headline).child(named(density)))
            .children(ROW_STATES.iter().enumerate().map(|(index, state)| {
                titled(
                    *state,
                    wide(dimension::WIDTH_PANEL, state_row(density, index, cx)),
                )
            }))
    }))
}

// -------------------------------------------------------- result list --

/// Five sample entries of different kinds, the first a production system.
fn result_row(list: &'static str, index: usize, count: usize, cx: &App) -> Row {
    let (parts, icon, chip) = match index % 5 {
        0 => (
            sap_parts(),
            FpIcon::TypeSap,
            Some(("PRD", ChipTone::Production)),
        ),
        1 => (
            RowParts::new(tr!("glossary.type.sap_system"), "NHQ 200")
                .role_word(tr!("glossary.role.quality_system"))
                .customer("Nordhafen AG")
                .client(format!("{} 200", tr!("glossary.sap_client")))
                .user("N.WEISS")
                .vault(VAULT),
            FpIcon::TypeSap,
            Some(("QAS", ChipTone::Neutral)),
        ),
        2 => (
            RowParts::new(tr!("glossary.type.remote_access"), "Nordhafen GP")
                .customer("Nordhafen AG")
                .user("n.weiss")
                .vault(VAULT),
            FpIcon::TypeRemote,
            None,
        ),
        3 => (
            RowParts::new(tr!("glossary.type.web_login"), "portal.nordhafen.example")
                .customer("Nordhafen AG")
                .user("n.weiss@nordhafen.example")
                .vault(VAULT),
            FpIcon::TypeWeb,
            None,
        ),
        _ => (
            RowParts::new(tr!("glossary.type.server"), "nhp-app-01")
                .customer("Nordhafen AG")
                .user("root")
                .vault(VAULT),
            FpIcon::TypeServer,
            None,
        ),
    };
    let row = Row::new(
        (list, index),
        index,
        count,
        parts,
        Tile::new(icon),
        team_badge(cx),
    )
    .density(RowDensity::Panel44)
    .shortcut_index(u8::try_from(index + 1).unwrap_or(9));
    match chip {
        Some((letters, tone)) => row.chip(Chip::new(tone, letters)),
        None => row,
    }
}

/// The list in the search group with five rows and the second selected, the
/// group with an empty list and the sentence in its place, and the list on
/// its own with nothing selected.
fn result_lists(samples: &Samples) -> Div {
    const ROWS: usize = 5;
    let searching = tr!("quick.header.searching_vault", vault = VAULT);
    let five = ResultList::new("results", tr!("a11y.results"), ROWS, |index, _, cx| {
        result_row("results-row", index, ROWS, cx)
    })
    .selected(Some(1));
    let none = ResultList::new("empty-results", tr!("a11y.results"), 0, |index, _, cx| {
        result_row("empty-row", index, 1, cx)
    });
    let plain = ResultList::new(
        "plain-results",
        tr!("a11y.results"),
        ROWS,
        |index, _, cx| result_row("plain-row", index, ROWS, cx),
    );
    let field_and_rows = 64. + ROWS as f32 * dimension::HEIGHT_ROW_PANEL;
    sections([
        section(
            "search group, five rows, the second selected",
            [boxed(
                dimension::WIDTH_PANEL,
                field_and_rows,
                SearchGroup::new(
                    "results-group",
                    tr!("a11y.search"),
                    TextField::new("results-search", &samples.search, searching.clone()),
                    five,
                ),
            )],
        ),
        section(
            "search group, no match",
            [boxed(
                dimension::WIDTH_PANEL,
                96.,
                SearchGroup::new(
                    "empty-group",
                    tr!("a11y.search"),
                    TextField::new("empty-search", &samples.search_empty, searching),
                    none,
                )
                .description(tr!("quick.no_match.text", vault = VAULT)),
            )],
        ),
        section(
            "list on its own, nothing selected",
            [boxed(
                dimension::WIDTH_PANEL,
                ROWS as f32 * dimension::HEIGHT_ROW_PANEL,
                plain,
            )],
        ),
    ])
}

// ------------------------------------------------------- secret field --

fn secret() -> Zeroizing<String> {
    Zeroizing::new(SECRET.to_string())
}

/// A field with `make` applied, named after the preview's password.
fn secret_field(name: &str, make: impl FnOnce(SecretField) -> SecretField) -> Div {
    cell(
        name.to_string(),
        wide(
            dimension::WIDTH_LIST,
            make(SecretField::new(
                id(format_args!("secret-{name}")),
                tr!("quick.preview.password"),
            )),
        ),
    )
}

/// What the copy button of the field that answers clicks does: it flashes
/// the value, shows the tick for a while and goes back. It copies nothing.
fn flash_then_tick(
    copied: Rc<Cell<CopyState>>,
    timer: Rc<RefCell<Option<Task<()>>>>,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    move |_, window, cx| {
        copied.set(CopyState::Flash);
        window.refresh();
        let copied = copied.clone();
        // A new copy starts over: dropping the last task stops it.
        *timer.borrow_mut() = Some(window.spawn(cx, async move |cx| {
            cx.background_executor().timer(FLASH).await;
            copied.set(CopyState::Check);
            cx.update(|window, _| window.refresh()).ok();
            cx.background_executor().timer(TICK).await;
            copied.set(CopyState::Idle);
            cx.update(|window, _| window.refresh()).ok();
        }));
    }
}

/// Hidden with and without its buttons, right after a copy and after the
/// flash, revealed, without a value, and one that answers clicks.
fn secret_fields(samples: &Samples) -> Div {
    let nothing = |_: &ClickEvent, _: &mut Window, _: &mut App| {};
    let revealed = samples.revealed.clone();
    section(
        "secret field",
        [
            secret_field("hidden", |field| field.value(secret())),
            secret_field("hidden, copy button", |field| {
                field.value(secret()).on_copy(nothing)
            }),
            secret_field("hidden, copy and reveal buttons", |field| {
                field
                    .value(secret())
                    .on_copy(nothing)
                    .on_toggle_reveal(nothing)
            }),
            secret_field("flash", |field| {
                field
                    .value(secret())
                    .copied(CopyState::Flash)
                    .on_copy(nothing)
            }),
            secret_field("tick", |field| {
                field
                    .value(secret())
                    .copied(CopyState::Check)
                    .on_copy(nothing)
            }),
            secret_field("revealed", |field| {
                field
                    .value(secret())
                    .revealed(true)
                    .on_copy(nothing)
                    .on_toggle_reveal(nothing)
            }),
            secret_field("revealed, no buttons", |field| {
                field.value(secret()).revealed(true)
            }),
            secret_field("no value", |field| field),
            secret_field("answers clicks", |field| {
                field
                    .value(secret())
                    .revealed(samples.revealed.get())
                    .copied(samples.copied.get())
                    .on_copy(flash_then_tick(
                        samples.copied.clone(),
                        samples.copy_timer.clone(),
                    ))
                    .on_toggle_reveal(move |_, window, _| {
                        revealed.set(!revealed.get());
                        window.refresh();
                    })
            }),
        ],
    )
}

// --------------------------------------------------- one-time code ring --

fn code() -> Zeroizing<String> {
    Zeroizing::new(CODE.to_string())
}

/// Both sizes: masked, shown, in the last seconds with the next code, masked
/// in the last seconds, and without a code.
fn one_time_codes() -> Div {
    let period = Duration::from_secs(30);
    let plenty = Duration::from_secs(25);
    let last = Duration::from_secs(4);
    sections([OtpSize::Row16, OtpSize::Detail18].map(|size| {
        let size_name = named(size);
        let ring = |state: &str, remaining: Duration| {
            OtpRing::new(
                id(format_args!("otp-{size_name}-{state}")),
                remaining,
                period,
            )
            .size(size)
        };
        section(
            size_name.clone(),
            [
                cell("masked, 25 s", ring("masked", plenty).code(code())),
                cell(
                    "shown, 25 s",
                    ring("shown", plenty).code(code()).masked(false),
                ),
                cell(
                    "shown, 4 s, next code",
                    ring("next", last)
                        .code(code())
                        .next_code(Zeroizing::new(NEXT_CODE.to_string()))
                        .masked(false),
                ),
                cell("masked, 4 s", ring("masked-last", last).code(code())),
                cell("no code, 25 s", ring("none", plenty)),
            ],
        )
    }))
}

// -------------------------------------------------------- panel footer --

/// The three shortcuts after the first line.
fn more_shortcuts(footer: PanelFooter) -> PanelFooter {
    footer
        .more(Shortcut::for_command(CommandId::ENTRY_COPY_PASSWORD))
        .more(Shortcut::for_command(CommandId::ENTRY_COPY_USERNAME))
        .more(Shortcut::for_command(CommandId::ENTRY_COPY_CODE))
}

/// The first line for an SAP system: Return and what it does.
fn sign_in() -> Shortcut {
    Shortcut::from_keystroke(
        "enter",
        tr!(
            "quick.footer.sap_login",
            sid = "NHP",
            client = "100",
            user = "N.WEISS"
        ),
    )
}

/// A footer whose menu button does nothing.
fn footer(name: &str) -> PanelFooter {
    PanelFooter::new(id(format_args!("footer-{name}")), |_, _, _| {})
}

/// What Return does while a production system waits to be chosen.
fn select_first() -> Shortcut {
    Shortcut::from_keystroke("", tr!("quick.footer.prd_select_first"))
}

/// The decision the first line names.
fn decision() -> Shortcut {
    Shortcut::from_keystroke("", tr!("msg.decision_needed", vault = VAULT))
}

/// A tone of the footer: its name, the tone, and what its first line says.
type ToneSample = (&'static str, FooterTone, fn() -> Shortcut);

/// Every tone with three more shortcuts and without them, then with the
/// line about what to connect to first, with its menu open, and with
/// nothing but the menu button.
fn panel_footers() -> Div {
    let tones: [ToneSample; 7] = [
        ("normal", FooterTone::Normal, sign_in),
        (
            "production, select it first",
            FooterTone::ProductionSelectFirst,
            select_first,
        ),
        ("decision", FooterTone::Decision { stale: false }, decision),
        (
            "decision, a day old",
            FooterTone::Decision { stale: true },
            decision,
        ),
        (
            "offline",
            FooterTone::Offline("As of 13:40".into()),
            sign_in,
        ),
        (
            "matches that look the same",
            FooterTone::Tie(tp!("quick.results.same_matches", 2)),
            sign_in,
        ),
        (
            "update ready",
            FooterTone::UpdateReady("Update ready".into()),
            sign_in,
        ),
    ];
    let toned = tones.into_iter().flat_map(|(name, tone, primary)| {
        let three = more_shortcuts(
            footer(&format!("{name}-three"))
                .primary(primary())
                .tone(tone.clone()),
        );
        let none = footer(&format!("{name}-none"))
            .primary(primary())
            .tone(tone);
        [
            titled(
                format!("{name}, three more shortcuts"),
                wide(dimension::WIDTH_PANEL, three),
            ),
            titled(
                format!("{name}, no more shortcuts"),
                wide(dimension::WIDTH_PANEL, none),
            ),
        ]
    });
    let others = [
        titled(
            "connect first",
            wide(
                dimension::WIDTH_PANEL,
                more_shortcuts(footer("connect").primary(sign_in()))
                    .connect_first("Connect first: Nordhafen GP (VPN)"),
            ),
        ),
        titled(
            "menu open",
            wide(
                dimension::WIDTH_PANEL,
                more_shortcuts(footer("menu").primary(sign_in())).menu_open(true),
            ),
        ),
        titled(
            "menu button only",
            wide(dimension::WIDTH_PANEL, footer("empty")),
        ),
    ];
    sections([
        div()
            .flex()
            .flex_col()
            .gap(u(dimension::SPACE_12))
            .child(div().text_style(TextStyle::Headline).child("tones"))
            .children(toned),
        div()
            .flex()
            .flex_col()
            .gap(u(dimension::SPACE_12))
            .child(div().text_style(TextStyle::Headline).child("parts"))
            .children(others),
    ])
}

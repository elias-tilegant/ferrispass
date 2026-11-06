//! The chip: a short mark next to a row's title, such as "PRD" for a
//! production system, "QAS" or "expires".
//!
//! In a row the chip writes no node: it has no id and no role, and the
//! row's name says the same thing in words, "Production system" for "PRD".
//! On its own it is a label with that spelled-out name, through
//! [`Chip::standalone`]. A chip shows the text its caller gives it: a word
//! from a text key, or a fixed mark such as "PRD", which the caller passes
//! in as well.

use gpui::{
    App, ElementId, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce,
    Role, SharedString, Styled as _, Window, div, px,
};
use gpui_component::Icon;

use super::Standalone;
use crate::ui::ds::ActiveTokens as _;
use crate::ui::ds::tokens::{ColorSet, dimension};
use crate::ui::ds::typography::{TextStyle, TextStyled as _, u};

/// Size of the glyph in a chip, in points.
const GLYPH_SIZE: f32 = 12.0;

/// What a chip says and in which color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChipTone {
    /// QAS, DEV, 2FA, new.
    Neutral,
    /// A production system: ocher, never carmine, which is for what cannot
    /// be undone.
    Production,
    /// Something that needs looking at soon, such as a code that expires.
    Attention,
    /// A decision that is open.
    Decision,
    Danger,
    /// Something that went well: up to date, saved, cleared.
    Patina,
}

impl ChipTone {
    pub const ALL: [ChipTone; 6] = [
        Self::Neutral,
        Self::Production,
        Self::Attention,
        Self::Decision,
        Self::Danger,
        Self::Patina,
    ];

    /// The surface, and the color of the text and the glyph.
    fn colors(self, colors: &ColorSet) -> (Hsla, Hsla) {
        let (surface, text) = match self {
            Self::Neutral => (colors.chip_neutral_bg, colors.chip_neutral_text),
            Self::Production => (colors.prd_chip_bg, colors.prd_chip_text),
            Self::Attention => (colors.status_attention_soft, colors.status_attention_text),
            Self::Decision => (colors.status_decision_soft, colors.status_decision_text),
            Self::Danger => (colors.status_danger_soft, colors.status_danger_text),
            Self::Patina => (colors.status_ok_soft, colors.status_ok_text),
        };
        (surface.hsla, text.hsla)
    }
}

/// A chip, 18 high with a radius of 4, its text in `Caption`, with an
/// optional glyph of 12 before it.
#[derive(IntoElement)]
pub struct Chip {
    tone: ChipTone,
    text: SharedString,
    glyph: Option<Icon>,
}

impl Chip {
    /// A chip for a row, which writes no node. Panics on an empty text, in
    /// every build.
    pub fn new(tone: ChipTone, text: impl Into<SharedString>) -> Self {
        let text = text.into();
        assert!(!text.is_empty(), "a chip needs its text");
        Self {
            tone,
            text,
            glyph: None,
        }
    }

    /// A glyph before the text, such as the clock of a code that expires.
    pub fn glyph(mut self, icon: impl Into<Icon>) -> Self {
        self.glyph = Some(icon.into());
        self
    }

    /// A chip on its own: a label named `name`, the chip's meaning spelled
    /// out ("Production system" for "PRD"), from a text key. Panics on an
    /// empty text or name.
    pub fn standalone(
        id: impl Into<ElementId>,
        tone: ChipTone,
        text: impl Into<SharedString>,
        name: impl Into<SharedString>,
    ) -> Standalone {
        Standalone::new(
            id.into(),
            Role::Label,
            name.into(),
            Self::new(tone, text).into_any_element(),
        )
    }
}

impl RenderOnce for Chip {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let (surface, text) = self.tone.colors(cx.ds().colors);
        div()
            .debug_selector(|| "fp-chip".into())
            .flex()
            .flex_none()
            .items_center()
            .gap(u(dimension::SPACE_4))
            .h(u(dimension::HEIGHT_CHIP))
            .px(u(dimension::SPACE_6))
            .rounded(px(dimension::RADIUS_XS))
            .bg(surface)
            .text_style(TextStyle::Caption)
            .text_color(text)
            .children(
                self.glyph
                    .map(|glyph| glyph.size(u(GLYPH_SIZE)).text_color(text)),
            )
            .child(self.text)
    }
}

#[cfg(test)]
mod tests {
    use gpui::{TestAppContext, VisualTestContext};
    use gpui_component::IconName;

    use super::super::probe;
    use super::*;
    use crate::ui::ds::tokens::Mode;

    fn text(value: &'static str) -> SharedString {
        SharedString::new_static(value)
    }

    /// The carmine colors, which are for errors and for what cannot be
    /// undone.
    fn carmine(colors: &ColorSet) -> [Hsla; 4] {
        [
            colors.status_danger_soft.hsla,
            colors.status_danger_text.hsla,
            colors.status_danger_fill.hsla,
            colors.button_danger_bg.hsla,
        ]
    }

    /// The surfaces a window painted in its last frame.
    fn surfaces(cx: &mut VisualTestContext) -> Vec<Hsla> {
        cx.update(|window, _| {
            window
                .painted_quads()
                .into_iter()
                .filter_map(|quad| quad.background.as_solid())
                .filter(|color| !color.is_transparent())
                .collect()
        })
    }

    /// A production chip is ocher in every mode, in `prd_chip_bg` with its
    /// text in `prd_chip_text`, and never carmine. The window shows the
    /// surface it paints.
    #[gpui::test]
    fn a_production_chip_is_ocher_never_carmine(cx: &mut TestAppContext) {
        for mode in Mode::ALL {
            let colors = mode.colors();
            let (surface, text) = ChipTone::Production.colors(colors);
            assert_eq!(surface, colors.prd_chip_bg.hsla, "{mode:?}: the surface");
            assert_eq!(text, colors.prd_chip_text.hsla, "{mode:?}: the text");
            for color in [surface, text] {
                assert!(
                    !carmine(colors).contains(&color),
                    "{mode:?}: {color:?} is carmine"
                );
            }
        }

        let cx = probe::window(cx, |_, _| {
            Chip::new(ChipTone::Production, text("PRD")).into_any_element()
        });
        let colors = cx.update(|_, cx| cx.ds().colors);
        let painted = surfaces(cx);
        assert!(painted.contains(&colors.prd_chip_bg.hsla), "{painted:?}");
        assert!(
            painted.iter().all(|color| !carmine(colors).contains(color)),
            "{painted:?}"
        );
        let chip = cx.debug_bounds("fp-chip").expect("the chip");
        // Layout snaps to device pixels, so the height is within one of 18
        // points.
        let height = cx.update(|window, _| u(dimension::HEIGHT_CHIP).to_pixels(window.rem_size()));
        assert!(
            (chip.size.height - height).abs() <= px(1.),
            "18 high: {:?}, not {height:?}",
            chip.size.height
        );
    }

    /// In a row the chip writes no node, with or without a glyph: the
    /// row's name says what it means.
    #[gpui::test]
    fn a_chip_in_a_row_writes_no_node(cx: &mut TestAppContext) {
        let found = probe::in_frame(cx, |window, cx| {
            ChipTone::ALL.map(|tone| {
                let chip = || Chip::new(tone, text("QAS")).glyph(IconName::Info);
                let role = probe::node(RenderOnce::render(chip(), window, cx)).0;
                (
                    tone,
                    role,
                    probe::element_id(RenderOnce::render(chip(), window, cx)),
                )
            })
        });
        for (tone, role, id) in found {
            assert_eq!(role, None, "{tone:?}: no role");
            assert_eq!(id, None, "{tone:?}: no id");
        }
    }

    /// On its own the chip is a label named with the meaning spelled out,
    /// "Production system", never the letters it shows.
    #[gpui::test]
    fn a_standalone_chip_is_a_label_with_the_spelled_out_name(cx: &mut TestAppContext) {
        let ((role, node), id) = probe::in_frame(cx, |window, cx| {
            let chip = || {
                Chip::standalone(
                    "role",
                    ChipTone::Production,
                    text("PRD"),
                    text("Production system"),
                )
            };
            let node = probe::node(RenderOnce::render(chip(), window, cx));
            (
                node,
                probe::element_id(RenderOnce::render(chip(), window, cx)),
            )
        });
        assert_eq!(role, Some(Role::Label));
        assert_eq!(node.label(), Some("Production system"));
        assert_eq!(id, Some(ElementId::from("role")));
        for (part, said) in [
            ("label", node.label()),
            ("value", node.value()),
            ("description", node.description()),
        ] {
            assert!(
                !said.is_some_and(|said| said.contains("PRD")),
                "the {part} spells the letters: {said:?}"
            );
        }
    }

    #[test]
    #[should_panic(expected = "a chip needs its text")]
    fn a_chip_needs_its_text_in_every_build() {
        let _ = Chip::new(ChipTone::Neutral, SharedString::default());
    }
}

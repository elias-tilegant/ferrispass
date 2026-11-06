//! The type tile: the glyph of an entry's kind on a small square, at the
//! start of a row and in the head of an entry's details.
//!
//! In a row the tile writes no node: it has no id and no role, and the
//! row's name starts with the kind, so VoiceOver hears it there. On its own
//! it is an image with the name of the kind, through [`Tile::standalone`].
//! The tile shows the glyph of the kind and nothing else: no letters taken
//! from the title and no website icons.
//!
//! Until the app's own glyphs arrive, the tile takes a GPUI Kit icon.

use gpui::{
    App, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, Role,
    SharedString, Styled as _, Window, div, px,
};
use gpui_component::Icon;

use super::Standalone;
use crate::ui::ds::ActiveTokens as _;
use crate::ui::ds::tokens::dimension;
use crate::ui::ds::typography::u;

/// Size of the glyph on the tile, in points.
const GLYPH_SIZE: f32 = 16.0;

/// The glyph of an entry's kind on a 28 by 28 square with a radius of 6.
#[derive(IntoElement)]
pub struct Tile {
    icon: Icon,
    selected: bool,
}

impl Tile {
    /// A tile for a row, which writes no node.
    pub fn new(icon: impl Into<Icon>) -> Self {
        Self {
            icon: icon.into(),
            selected: false,
        }
    }

    /// The tile of a selected row, in `tile_selected_bg` and
    /// `tile_selected_glyph`. The row sets it.
    pub(in crate::ui::ds) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// A tile on its own, such as in the head of an entry's details: an
    /// image named after the kind, `name`, from a text key such as
    /// `tr!("glossary.type.sap_system")`. Panics on an empty name.
    pub fn standalone(
        id: impl Into<ElementId>,
        icon: impl Into<Icon>,
        name: impl Into<SharedString>,
    ) -> Standalone {
        Standalone::new(
            id.into(),
            Role::Image,
            name.into(),
            Self::new(icon).into_any_element(),
        )
    }
}

impl RenderOnce for Tile {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.ds().colors;
        let (surface, glyph) = if self.selected {
            (colors.tile_selected_bg, colors.tile_selected_glyph)
        } else {
            (colors.tile_bg, colors.tile_glyph)
        };
        div()
            .debug_selector(|| "fp-tile".into())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(u(dimension::HEIGHT_TILE))
            .rounded(px(dimension::RADIUS_SM))
            .bg(surface.hsla)
            .child(self.icon.size(u(GLYPH_SIZE)).text_color(glyph.hsla))
    }
}

#[cfg(test)]
mod tests {
    use gpui::{TestAppContext, px};
    use gpui_component::IconName;

    use super::super::probe;
    use super::*;

    fn name(text: &'static str) -> SharedString {
        SharedString::new_static(text)
    }

    /// In a row the tile has no id and no role, so it writes no node: the
    /// row's name carries the kind. It is 28 by 28 points at text size 13
    /// and grows with the text size.
    #[gpui::test]
    fn a_tile_in_a_row_writes_no_node(cx: &mut TestAppContext) {
        let (role, id) = probe::in_frame(cx, |window, cx| {
            let role = probe::node(RenderOnce::render(Tile::new(IconName::Globe), window, cx)).0;
            let id = probe::element_id(RenderOnce::render(Tile::new(IconName::Globe), window, cx));
            (role, id)
        });
        assert_eq!(role, None, "no role");
        assert_eq!(id, None, "no id");

        let cx = probe::window(cx, |_, _| Tile::new(IconName::Globe).into_any_element());
        let bounds = cx.debug_bounds("fp-tile").expect("the tile");
        // Layout snaps to device pixels, so each side is within one of 28
        // points.
        let side = cx.update(|window, _| u(dimension::HEIGHT_TILE).to_pixels(window.rem_size()));
        for length in [bounds.size.width, bounds.size.height] {
            assert!((length - side).abs() <= px(1.), "{length:?}, not {side:?}");
        }
    }

    /// On its own, as in the head of an entry's details, the tile is an
    /// image named after the kind, with an id, so that it reaches the tree.
    #[gpui::test]
    fn a_standalone_tile_is_an_image_with_its_name(cx: &mut TestAppContext) {
        let ((role, node), id) = probe::in_frame(cx, |window, cx| {
            let tile = || Tile::standalone("kind", IconName::Globe, name("Web login"));
            let node = probe::node(RenderOnce::render(tile(), window, cx));
            (
                node,
                probe::element_id(RenderOnce::render(tile(), window, cx)),
            )
        });
        assert_eq!(role, Some(Role::Image));
        assert_eq!(node.label(), Some("Web login"));
        assert_eq!(id, Some(ElementId::from("kind")));

        let cx = probe::window(cx, |_, _| {
            Tile::standalone("kind", IconName::Globe, name("Web login")).into_any_element()
        });
        assert!(cx.debug_bounds("fp-tile").is_some(), "it draws the tile");
    }

    #[test]
    #[should_panic(expected = "a component on its own needs a name")]
    fn a_standalone_tile_needs_a_name_in_every_build() {
        let _ = Tile::standalone("kind", IconName::Globe, SharedString::default());
    }
}

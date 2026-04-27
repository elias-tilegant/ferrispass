use gpui::{
    AnyElement, IntoElement as _, ParentElement as _, Styled as _, div, px,
};
use gpui_component::{Icon, Sizable as _};

use crate::ui::icons::AppIcon;
use crate::ui::palette;

/// The "KeePass RS" mark — a deep-blue rounded square with an orange key glyph.
pub fn brand(size: f32) -> AnyElement {
    div()
        .size(px(size))
        .rounded(px((size / 4.5).max(6.0)))
        .bg(palette::BLUE)
        .flex()
        .items_center()
        .justify_center()
        .child(
            Icon::from(AppIcon::Key)
                .with_size(gpui_component::Size::Size(px((size * 0.55).round())))
                .text_color(palette::ORANGE),
        )
        .into_any_element()
}

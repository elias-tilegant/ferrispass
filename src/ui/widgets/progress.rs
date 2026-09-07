//! Progress that is honest about not knowing how far along it is.
//!
//! Four screens rendered a static bar filled to 40% while they waited. It
//! never moved, so it read as a stalled download rather than as work in
//! progress. There is no percentage to show for a device-code poll, an Argon2
//! derivation or a Graph round trip, so this says "working" and animates.

use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _};
use gpui_component::{Sizable as _, h_flex};

use crate::ui::palette;

/// A spinner and a line of text, for a wait with no known duration.
pub fn working(message: impl Into<String>) -> AnyElement {
    h_flex()
        .gap_2()
        .items_center()
        .child(
            gpui_component::spinner::Spinner::new()
                .with_size(gpui_component::Size::Size(gpui::px(14.)))
                .color(palette::blue()),
        )
        .child(
            gpui::div()
                .text_xs()
                .text_color(palette::text_muted())
                .child(message.into()),
        )
        .into_any_element()
}

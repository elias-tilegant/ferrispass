use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, div, px};

use crate::ui::palette;

pub(crate) fn switch_visual(on: bool) -> AnyElement {
    div()
        .relative()
        .w(px(32.))
        .h(px(18.))
        .rounded_full()
        .bg(if on {
            palette::blue()
        } else {
            palette::border_strong()
        })
        .child(
            div()
                .absolute()
                .top(px(2.))
                .left(px(if on { 16. } else { 2. }))
                .size(px(14.))
                .rounded_full()
                .bg(palette::panel()),
        )
        .into_any_element()
}

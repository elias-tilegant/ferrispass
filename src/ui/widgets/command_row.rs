use gpui::{
    AnyElement, App, ClickEvent, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{Sizable as _, h_flex, v_flex};

use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::interaction::Interaction as _;

#[derive(Clone, Copy)]
pub enum RowTone {
    Default,
    Primary,
}

pub fn command_row<F>(
    id: impl Into<ElementId>,
    icon: AppIcon,
    title: impl Into<SharedString>,
    meta: impl Into<SharedString>,
    tone: RowTone,
    trailing: Option<SharedString>,
    on_click: F,
) -> AnyElement
where
    F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let is_primary = matches!(tone, RowTone::Primary);
    let bg = if is_primary {
        palette::blue_soft()
    } else {
        palette::panel()
    };
    let border = if is_primary {
        palette::blue_border()
    } else {
        palette::border()
    };
    let icon_bg = if is_primary {
        palette::blue()
    } else {
        palette::sidebar()
    };
    let icon_fg = if is_primary {
        palette::panel()
    } else {
        palette::text_muted()
    };
    let title_color = if is_primary {
        palette::blue_hover()
    } else {
        palette::text()
    };

    // The `id`, hover and press feedback all live on this single styled row so
    // GPUI tracks the interactive state reliably — splitting `id`/`on_click`
    // onto an outer wrapper (the previous shape) left the hover on a non-stateful
    // child, which only repainted when something else nudged the tree.
    h_flex()
        .id(id.into())
        .h(px(58.))
        .gap_3()
        .items_center()
        .px_3()
        .rounded(px(7.))
        .bg(bg)
        .border_1()
        .border_color(border)
        .hover(|s| {
            if is_primary {
                s.border_color(palette::blue())
            } else {
                s.bg(palette::sidebar())
                    .border_color(palette::border_strong())
            }
        })
        .pressable()
        .on_click(on_click)
        .child(
            div()
                .size(px(30.))
                .flex_shrink_0()
                .rounded(px(6.))
                .bg(icon_bg)
                .text_color(icon_fg)
                .border_1()
                .border_color(if is_primary {
                    palette::blue()
                } else {
                    palette::border()
                })
                .flex()
                .items_center()
                .justify_center()
                .child(
                    gpui_component::Icon::from(icon)
                        .with_size(gpui_component::Size::Size(px(14.))),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w(px(0.))
                .gap_0p5()
                .child(
                    div()
                        .truncate()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(title_color)
                        .child(title.into()),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(palette::text_faint())
                        .child(meta.into()),
                ),
        )
        .when_some(trailing, |this, trailing| {
            this.child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(if is_primary {
                        palette::blue()
                    } else {
                        palette::text_faint()
                    })
                    .child(trailing),
            )
        })
        .into_any_element()
}

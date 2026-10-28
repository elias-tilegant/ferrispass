use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::{Sizable as _, h_flex, input::Input, v_flex};

use crate::app::Overlay;
use crate::ui::app_shell::AppShell;
use crate::ui::palette;
use crate::ui::widgets::atoms::label;
use crate::ui::widgets::interaction::Interaction as _;

pub fn render(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let underlay = crate::ui::screens::vault::render(shell, cx);
    let modal = modal_card(shell, cx);

    div()
        .size_full()
        .relative()
        .child(underlay)
        .child(
            div()
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .left_0()
                .bg(palette::transparent_overlay())
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .p(px(16.))
                .child(modal),
        )
        .into_any_element()
}

fn modal_card(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let overlay = shell.state().read(cx).overlay().clone();
    let name_input = shell.new_group_name_input().clone();

    // Resolve title + subtitle from the overlay variant. For AddGroup
    // we distinguish "top-level" (parent == root) from "subgroup so
    // the user always knows where the new group is going to land.
    let (title, subtitle) = match &overlay {
        Overlay::AddGroup { parent_group_id } => {
            let browser = shell.state().read(cx).vault_browser();
            let root_id = browser
                .as_ref()
                .map(|b| b.snapshot.root.id.clone())
                .unwrap_or_default();
            if parent_group_id == &root_id {
                ("New group".to_string(), String::new())
            } else {
                let parent_name = browser
                    .as_ref()
                    .and_then(|b| b.snapshot.find_group(parent_group_id))
                    .map(|g| g.name.clone())
                    .unwrap_or_default();
                ("New subgroup".to_string(), format!("in {parent_name}"))
            }
        }
        Overlay::RenameGroup { group_id } => {
            let current = shell
                .state()
                .read(cx)
                .vault_browser()
                .and_then(|b| b.snapshot.find_group(group_id).map(|g| g.name.clone()))
                .unwrap_or_default();
            ("Rename group".to_string(), current)
        }
        _ => ("Group".to_string(), String::new()),
    };

    let cancel_button = div()
        .id("group-cancel")
        .h(px(30.))
        .px(px(12.))
        .rounded(px(6.))
        .bg(palette::panel())
        .border_1()
        .border_color(palette::border_strong())
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(palette::text())
        .flex()
        .items_center()
        .justify_center()
        .child("Cancel")
        .hover_press(palette::border())
        .on_click(cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
            shell.state().clone().update(cx, |state, cx| {
                let _ = state.close_overlay(cx);
            });
        }));

    let save_button = div()
        .id("group-save")
        .h(px(30.))
        .px(px(14.))
        .rounded(px(6.))
        .bg(palette::blue())
        .border_1()
        .border_color(palette::blue_hover())
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(palette::panel())
        .flex()
        .items_center()
        .justify_center()
        .gap_1p5()
        .child(
            gpui_component::Icon::from(gpui_component::IconName::Check)
                .with_size(gpui_component::Size::Size(px(13.)))
                .text_color(palette::panel()),
        )
        .child("Save")
        .hover_press(palette::blue_hover())
        .on_click(
            cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
                shell.submit_group_form(window, cx);
            }),
        );

    v_flex()
        .w(px(380.))
        .bg(palette::panel())
        .border_1()
        .border_color(palette::border())
        .rounded(px(10.))
        .overflow_hidden()
        .child(
            h_flex()
                .gap_2p5()
                .items_center()
                .px_5()
                .py_4()
                .border_b_1()
                .border_color(palette::border())
                .child(
                    div()
                        .size(px(28.))
                        .rounded(px(6.))
                        .bg(palette::blue_soft())
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            gpui_component::Icon::from(gpui_component::IconName::Folder)
                                .with_size(gpui_component::Size::Size(px(14.)))
                                .text_color(palette::blue()),
                        ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette::text_muted())
                                .child(subtitle),
                        ),
                ),
        )
        .child(
            v_flex()
                .px_5()
                .py_4()
                .gap_2()
                .child(label("Name"))
                .child(Input::new(&name_input)),
        )
        .child(
            h_flex()
                .px_5()
                .py_3p5()
                .gap_2()
                .justify_end()
                .border_t_1()
                .border_color(palette::border())
                .child(cancel_button)
                .child(save_button),
        )
        .into_any_element()
}

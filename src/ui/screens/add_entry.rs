use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement as _,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::{Sizable as _, WindowExt as _, h_flex, input::Input, v_flex};

use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::atoms::label;
use crate::ui::widgets::password::generator_card;

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
                .bg(palette::TRANSPARENT_OVERLAY)
                .flex()
                .items_start()
                .justify_center()
                .pt(px(60.))
                .child(modal),
        )
        .into_any_element()
}

fn modal_card(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let title_input = shell.new_entry_title_input().clone();
    let username_input = shell.new_entry_username_input().clone();
    let password_input = shell.new_entry_password_input().clone();
    let url_input = shell.new_entry_url_input().clone();

    let cancel_button = div()
        .id("add-cancel")
        .h(px(30.))
        .px(px(12.))
        .rounded(px(6.))
        .bg(palette::PANEL)
        .border_1()
        .border_color(palette::BORDER_STRONG)
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(palette::TEXT)
        .flex()
        .items_center()
        .justify_center()
        .child("Cancel")
        .on_click(cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
            shell.state().clone().update(cx, |state, cx| {
                let _ = state.close_overlay(cx);
            });
        }));

    let save_button = div()
        .id("add-save")
        .h(px(30.))
        .px(px(14.))
        .rounded(px(6.))
        .bg(palette::BLUE)
        .border_1()
        .border_color(palette::BLUE_HOVER)
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(palette::PANEL)
        .flex()
        .items_center()
        .justify_center()
        .gap_1p5()
        .child(
            gpui_component::Icon::from(gpui_component::IconName::Check)
                .with_size(gpui_component::Size::Size(px(13.)))
                .text_color(palette::PANEL),
        )
        .child("Save entry")
        .on_click(cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
            window.push_notification("Saving entries isn't wired up yet — coming soon.", cx);
            shell.state().clone().update(cx, |state, cx| {
                let _ = state.close_overlay(cx);
            });
        }));

    v_flex()
        .w(px(540.))
        .bg(palette::PANEL)
        .border_1()
        .border_color(palette::BORDER)
        .rounded(px(10.))
        .overflow_hidden()
        .child(
            h_flex()
                .gap_2p5()
                .items_center()
                .px_5()
                .py_4()
                .border_b_1()
                .border_color(palette::BORDER)
                .child(
                    div()
                        .size(px(28.))
                        .rounded(px(6.))
                        .bg(palette::BLUE_SOFT)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            gpui_component::Icon::from(gpui_component::IconName::Plus)
                                .with_size(gpui_component::Size::Size(px(14.)))
                                .text_color(palette::BLUE),
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
                                .child("New entry"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette::TEXT_MUTED)
                                .child("in Work"),
                        ),
                ),
        )
        .child(
            v_flex()
                .gap_3p5()
                .px_5()
                .py_4()
                .child(
                    v_flex()
                        .gap_2()
                        .child(label("Title"))
                        .child(Input::new(&title_input)),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(label("Username"))
                        .child(Input::new(&username_input)),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(label("Password"))
                        .child(
                            h_flex()
                                .gap_1p5()
                                .child(div().flex_1().child(Input::new(&password_input)))
                                .child(generate_button()),
                        )
                        .child(generator_card(18, "Strong", 118)),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(label("URL"))
                        .child(Input::new(&url_input)),
                ),
        )
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .px_5()
                .py_3()
                .bg(palette::SIDEBAR)
                .border_t_1()
                .border_color(palette::BORDER)
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(palette::TEXT_MUTED)
                        .font_family("JetBrains Mono")
                        .child("Saves locally, syncs to OneDrive"),
                )
                .child(cancel_button)
                .child(save_button),
        )
        .into_any_element()
}

fn generate_button() -> AnyElement {
    h_flex()
        .h(px(32.))
        .px(px(12.))
        .gap_1p5()
        .items_center()
        .rounded(px(6.))
        .bg(palette::ORANGE)
        .border_1()
        .border_color(palette::ORANGE_DEEP)
        .text_color(palette::PANEL)
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .child(
            gpui_component::Icon::from(AppIcon::Refresh)
                .with_size(gpui_component::Size::Size(px(12.)))
                .text_color(palette::PANEL),
        )
        .child("Generate")
        .into_any_element()
}

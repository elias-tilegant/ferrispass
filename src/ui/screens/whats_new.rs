use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder, px,
};
use gpui_component::{h_flex, scroll::ScrollableElement as _, v_flex};

use crate::app::Overlay;
use crate::ui::app_shell::AppShell;
use crate::ui::palette;

pub fn render(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let info = match shell.state().read(cx).overlay() {
        Overlay::WhatsNew { info } => info.clone(),
        _ => return div().into_any_element(),
    };

    let notes = if info.notes.trim().is_empty() {
        vec![
            div()
                .text_sm()
                .text_color(palette::text_muted())
                .child("No release notes were included with this update.")
                .into_any_element(),
        ]
    } else {
        info.notes
            .lines()
            .map(|line| {
                if line.trim().is_empty() {
                    div().h(px(8.)).into_any_element()
                } else {
                    div()
                        .text_sm()
                        .line_height(px(20.))
                        .text_color(palette::text())
                        .child(line.to_string())
                        .into_any_element()
                }
            })
            .collect()
    };

    div()
        .size_full()
        .bg(palette::transparent_overlay())
        .flex()
        .items_center()
        .justify_center()
        .p_6()
        .child(
            v_flex()
                .w(px(560.))
                .max_h(px(620.))
                .rounded(px(10.))
                .bg(palette::panel())
                .border_1()
                .border_color(palette::border_strong())
                .shadow_lg()
                .overflow_hidden()
                .child(
                    h_flex()
                        .items_start()
                        .justify_between()
                        .gap_4()
                        .p_5()
                        .border_b_1()
                        .border_color(palette::border())
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .text_color(palette::text())
                                        .child(format!(
                                            "What's New in FerrisPass {}",
                                            info.version
                                        )),
                                )
                                .when_some(info.pub_date.clone(), |this, date| {
                                    this.child(
                                        div()
                                            .text_xs()
                                            .text_color(palette::text_muted())
                                            .child(date),
                                    )
                                }),
                        )
                        .child(close_button(cx)),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_h(px(0.))
                        .overflow_y_scrollbar()
                        .p_5()
                        .gap_1()
                        .children(notes),
                )
                .child(
                    h_flex()
                        .justify_end()
                        .p_4()
                        .border_t_1()
                        .border_color(palette::border())
                        .child(done_button(cx)),
                ),
        )
        .into_any_element()
}

fn close_button(cx: &mut Context<AppShell>) -> AnyElement {
    div()
        .id("whats-new-close")
        .h(px(28.))
        .px(px(10.))
        .rounded(px(5.))
        .border_1()
        .border_color(palette::border_strong())
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(palette::text())
        .flex()
        .items_center()
        .justify_center()
        .child("Close")
        .cursor_pointer()
        .hover(|s| s.opacity(0.85))
        .on_click(cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
            shell.state().clone().update(cx, |state, cx| {
                let _ = state.close_overlay(cx);
            });
        }))
        .into_any_element()
}

fn done_button(cx: &mut Context<AppShell>) -> AnyElement {
    div()
        .id("whats-new-done")
        .h(px(30.))
        .px(px(14.))
        .rounded(px(6.))
        .bg(palette::blue())
        .text_color(palette::panel())
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .flex()
        .items_center()
        .justify_center()
        .child("Done")
        .cursor_pointer()
        .hover(|s| s.opacity(0.88))
        .on_click(cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
            shell.state().clone().update(cx, |state, cx| {
                let _ = state.close_overlay(cx);
            });
        }))
        .into_any_element()
}

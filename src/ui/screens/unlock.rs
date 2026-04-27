use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement as _,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Sizable as _, h_flex,
    input::{Input, InputState},
    v_flex,
};

use crate::app::actions::{CancelUnlock, SubmitPassword};
use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::atoms::label;

pub fn render(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let prompt = match shell.state().read(cx).unlock_prompt() {
        Some(p) => p,
        None => return div().into_any_element(),
    };

    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(cx.theme().background)
        .child(
            v_flex()
                .w(px(420.))
                .p_10()
                .bg(palette::PANEL)
                .border_1()
                .border_color(palette::BORDER)
                .rounded(px(12.))
                .gap_5()
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(
                            div()
                                .size(px(40.))
                                .rounded(px(8.))
                                .bg(palette::BLUE_SOFT)
                                .border_1()
                                .border_color(palette::BLUE_BORDER)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    gpui_component::Icon::from(AppIcon::Lock)
                                        .with_size(gpui_component::Size::Size(px(18.)))
                                        .text_color(palette::BLUE),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_base()
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .child(prompt.file_name.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(palette::TEXT_MUTED)
                                        .font_family("JetBrains Mono")
                                        .child("OneDrive · synced 2 minutes ago"),
                                ),
                        ),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(label("Master password"))
                        .child(input_box(shell.password_input())),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            h_flex()
                                .gap_1()
                                .child(label("Key file"))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(palette::TEXT_FAINT)
                                        .child("(optional)"),
                                ),
                        )
                        .child(input_box(shell.keyfile_input())),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .text_xs()
                        .text_color(palette::TEXT_MUTED)
                        .child(
                            div()
                                .size(px(14.))
                                .rounded(px(3.))
                                .bg(palette::BLUE)
                                .text_color(palette::PANEL)
                                .text_xs()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child("✓"),
                        )
                        .child("Remember in keychain for 8 hours"),
                )
                .when_some(prompt.error.clone(), |this, error| {
                    this.child(div().text_sm().text_color(palette::RED).child(error))
                })
                .child(
                    div()
                        .id("unlock-submit")
                        .on_click(cx.listener(
                            |_: &mut AppShell, _: &ClickEvent, window, cx| {
                                window.dispatch_action(Box::new(SubmitPassword), cx);
                            },
                        ))
                        .child(primary_button("Unlock vault", AppIcon::Unlock)),
                )
                .child(
                    div()
                        .pt_4()
                        .border_t_1()
                        .border_color(palette::BORDER)
                        .child(
                            h_flex()
                                .items_center()
                                .justify_between()
                                .text_xs()
                                .text_color(palette::TEXT_FAINT)
                                .child("Touch ID available")
                                .child(
                                    div()
                                        .id("cancel-unlock")
                                        .text_color(palette::BLUE)
                                        .child("Cancel")
                                        .on_click(cx.listener(
                                            |_: &mut AppShell, _: &ClickEvent, window, cx| {
                                                window.dispatch_action(Box::new(CancelUnlock), cx);
                                            },
                                        )),
                                ),
                        ),
                ),
        )
        .into_any_element()
}

fn input_box(state: &gpui::Entity<InputState>) -> impl gpui::IntoElement {
    Input::new(state).cleanable(false)
}

fn primary_button(label: &'static str, icon: AppIcon) -> impl gpui::IntoElement {
    h_flex()
        .h(px(38.))
        .w_full()
        .px_4()
        .rounded(px(6.))
        .gap_2()
        .items_center()
        .justify_center()
        .bg(palette::BLUE)
        .border_1()
        .border_color(palette::BLUE_HOVER)
        .text_color(palette::PANEL)
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .child(
            gpui_component::Icon::from(icon)
                .with_size(gpui_component::Size::Size(px(14.)))
                .text_color(palette::PANEL),
        )
        .child(label)
}

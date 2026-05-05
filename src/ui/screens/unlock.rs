use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
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
    let state = shell.state().read(cx);
    let prompt = match state.unlock_prompt() {
        Some(p) => p,
        None => return div().into_any_element(),
    };
    let summary = state.summary();
    let subtitle = match (&summary.provider, &summary.synced_at) {
        (Some(provider), Some(synced)) => format!("{provider} · {synced}"),
        (Some(provider), None) => provider.clone(),
        _ => prompt.display_path.clone(),
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
                .bg(palette::panel())
                .border_1()
                .border_color(palette::border())
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
                                .bg(palette::blue_soft())
                                .border_1()
                                .border_color(palette::blue_border())
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    gpui_component::Icon::from(AppIcon::Lock)
                                        .with_size(gpui_component::Size::Size(px(18.)))
                                        .text_color(palette::blue()),
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
                                        .text_color(palette::text_muted())
                                        .font_family("JetBrains Mono")
                                        .child(subtitle),
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
                            h_flex().gap_1().child(label("Key file")).child(
                                div()
                                    .text_xs()
                                    .text_color(palette::text_faint())
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
                        .text_color(palette::text_muted())
                        .child(
                            div()
                                .size(px(14.))
                                .rounded(px(3.))
                                .bg(palette::blue())
                                .text_color(palette::panel())
                                .text_xs()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child("✓"),
                        )
                        .child("Remember in keychain for 8 hours"),
                )
                .when_some(prompt.error.clone(), |this, error| {
                    this.child(div().text_sm().text_color(palette::red()).child(error))
                })
                .child(
                    div()
                        .id("unlock-submit")
                        .on_click(cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                            window.dispatch_action(Box::new(SubmitPassword), cx);
                        }))
                        .child(primary_button("Unlock vault", AppIcon::Unlock)),
                )
                .child(
                    div()
                        .pt_4()
                        .border_t_1()
                        .border_color(palette::border())
                        .child(
                            h_flex()
                                .items_center()
                                .justify_between()
                                .text_xs()
                                .text_color(palette::text_faint())
                                .child("Touch ID available")
                                .child(
                                    div()
                                        .id("cancel-unlock")
                                        .text_color(palette::blue())
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
        .bg(palette::blue())
        .border_1()
        .border_color(palette::blue_hover())
        .text_color(palette::panel())
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .child(
            gpui_component::Icon::from(icon)
                .with_size(gpui_component::Size::Size(px(14.)))
                .text_color(palette::panel()),
        )
        .child(label)
}

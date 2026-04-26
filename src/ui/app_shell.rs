use crate::app::{AppState, VaultSummary};
use gpui::{
    AnyElement, Context, Entity, IntoElement as _, ParentElement as _, Render, Styled as _, Window,
    div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};

pub struct AppShell {
    state: Entity<AppState>,
}

impl AppShell {
    pub fn new(state: Entity<AppState>) -> Self {
        Self { state }
    }
}

impl Render for AppShell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let summary = self.state.read(cx).summary();

        h_flex()
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(render_sidebar(&summary, cx))
            .child(render_workspace(&summary, cx))
    }
}

fn render_sidebar(summary: &VaultSummary, cx: &mut Context<AppShell>) -> AnyElement {
    v_flex()
        .h_full()
        .w(px(286.))
        .flex_shrink_0()
        .bg(cx.theme().sidebar)
        .text_color(cx.theme().sidebar_foreground)
        .border_r_1()
        .border_color(cx.theme().sidebar_border)
        .child(
            v_flex()
                .gap_3()
                .p_4()
                .border_b_1()
                .border_color(cx.theme().sidebar_border)
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(div().text_lg().font_semibold().child("STC KeePass"))
                        .child(status_badge(&summary.status, cx)),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .overflow_hidden()
                        .child(summary.title.clone()),
                ),
        )
        .child(
            v_flex()
                .gap_1()
                .p_3()
                .child(nav_item("All entries", summary.is_open, cx))
                .child(nav_item("Groups", false, cx))
                .child(nav_item("Favorites", false, cx)),
        )
        .child(div().flex_1())
        .child(
            v_flex()
                .gap_3()
                .p_4()
                .border_t_1()
                .border_color(cx.theme().sidebar_border)
                .child(stat_row("Entries", summary.entries, cx))
                .child(stat_row("Groups", summary.groups, cx)),
        )
        .into_any_element()
}

fn render_workspace(summary: &VaultSummary, cx: &mut Context<AppShell>) -> AnyElement {
    let content = if summary.is_open {
        render_vault_overview(summary, cx)
    } else {
        render_empty_state(cx)
    };

    v_flex()
        .flex_1()
        .min_w(px(0.))
        .bg(cx.theme().background)
        .child(
            h_flex()
                .h(px(64.))
                .px_5()
                .border_b_1()
                .border_color(cx.theme().border)
                .justify_between()
                .child(
                    v_flex()
                        .gap_1()
                        .child(div().text_lg().font_semibold().child("Vault"))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(summary.status.clone()),
                        ),
                )
                .child(open_button("toolbar-open-vault")),
        )
        .child(content)
        .into_any_element()
}

fn render_empty_state(cx: &mut Context<AppShell>) -> AnyElement {
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_4()
        .p_6()
        .child(
            v_flex()
                .items_center()
                .gap_2()
                .child(div().text_2xl().font_semibold().child("No vault open"))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Ready"),
                ),
        )
        .child(open_button("empty-open-vault"))
        .into_any_element()
}

fn render_vault_overview(summary: &VaultSummary, cx: &mut Context<AppShell>) -> AnyElement {
    v_flex()
        .flex_1()
        .gap_4()
        .p_5()
        .child(
            div()
                .text_2xl()
                .font_semibold()
                .child(summary.title.clone()),
        )
        .child(
            h_flex()
                .gap_3()
                .child(metric_tile("Entries", summary.entries, cx))
                .child(metric_tile("Groups", summary.groups, cx)),
        )
        .into_any_element()
}

fn open_button(id: &'static str) -> Button {
    Button::new(id)
        .primary()
        .label("Open vault")
        .on_click(|_, _, _| {
            eprintln!("Open vault flow is not wired yet");
        })
}

fn nav_item(label: &'static str, selected: bool, cx: &mut Context<AppShell>) -> AnyElement {
    h_flex()
        .h(px(34.))
        .w_full()
        .px_3()
        .rounded(px(6.))
        .text_sm()
        .when(selected, |this| {
            this.bg(cx.theme().sidebar_accent)
                .text_color(cx.theme().sidebar_accent_foreground)
        })
        .when(!selected, |this| {
            this.text_color(cx.theme().sidebar_foreground.opacity(0.76))
        })
        .child(label)
        .into_any_element()
}

fn status_badge(status: &str, cx: &mut Context<AppShell>) -> AnyElement {
    div()
        .px_2()
        .py_1()
        .rounded(px(999.))
        .border_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(status.to_string())
        .into_any_element()
}

fn stat_row(label: &'static str, value: usize, cx: &mut Context<AppShell>) -> AnyElement {
    h_flex()
        .justify_between()
        .text_sm()
        .child(div().text_color(cx.theme().muted_foreground).child(label))
        .child(div().font_medium().child(value.to_string()))
        .into_any_element()
}

fn metric_tile(label: &'static str, value: usize, cx: &mut Context<AppShell>) -> AnyElement {
    v_flex()
        .w(px(156.))
        .gap_1()
        .rounded(px(8.))
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().secondary)
        .p_4()
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(div().text_2xl().font_semibold().child(value.to_string()))
        .into_any_element()
}

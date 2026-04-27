use crate::{
    app::{AppState, VaultBrowserModel},
    domain::{VaultEntry, VaultGroup},
    ui::AppShell,
};
use gpui::{
    AnyElement, ClickEvent, Context, Entity, IntoElement as _, ParentElement as _, Styled as _,
    div, prelude::FluentBuilder as _, px,
};
use gpui_component::{ActiveTheme as _, StyledExt as _, h_flex, list::ListItem, v_flex};

pub fn render_group_tree(
    model: &VaultBrowserModel,
    state: &Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    v_flex()
        .gap_1()
        .p_3()
        .child(
            div()
                .px_2()
                .pb_2()
                .text_xs()
                .font_medium()
                .text_color(cx.theme().muted_foreground)
                .child("Groups"),
        )
        .child(render_group_row(
            &model.root,
            &model.selected_group_id,
            state,
            0,
            cx,
        ))
        .into_any_element()
}

pub fn render_vault_browser(
    model: VaultBrowserModel,
    state: &Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    h_flex()
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .child(render_entry_list(&model, state, cx))
        .child(render_entry_detail(&model, cx))
        .into_any_element()
}

fn render_group_row(
    group: &VaultGroup,
    selected_group_id: &str,
    state: &Entity<AppState>,
    depth: usize,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let selected = group.id == selected_group_id;
    let group_id = group.id.clone();
    let state_for_click = state.clone();

    v_flex()
        .gap_1()
        .child(
            ListItem::new(format!("group-{}", group.id))
                .selected(selected)
                .h(px(30.))
                .w_full()
                .pl(px(10.) + px((depth as f32) * 14.))
                .pr_2()
                .rounded(px(6.))
                .gap_2()
                .text_sm()
                .when(selected, |this| {
                    this.bg(cx.theme().sidebar_accent)
                        .text_color(cx.theme().sidebar_accent_foreground)
                })
                .when(!selected, |this| {
                    this.text_color(cx.theme().sidebar_foreground.opacity(0.82))
                })
                .on_click(move |_: &ClickEvent, _, cx| {
                    state_for_click.update(cx, |state, cx| {
                        state.select_group(group_id.clone(), cx);
                    });
                })
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .child(group.name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(group.entries.len().to_string()),
                ),
        )
        .children(
            group
                .groups
                .iter()
                .map(|group| render_group_row(group, selected_group_id, state, depth + 1, cx)),
        )
        .into_any_element()
}

fn render_entry_list(
    model: &VaultBrowserModel,
    state: &Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    v_flex()
        .h_full()
        .w(px(360.))
        .flex_shrink_0()
        .border_r_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .child(
            v_flex()
                .gap_1()
                .p_4()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    h_flex()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .font_semibold()
                                .overflow_hidden()
                                .child(model.selected_group_name.clone()),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{} entries", model.entries.len())),
                        ),
                )
                .when(model.showing_search_results, |this| {
                    this.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("Search: {}", model.search_query)),
                    )
                }),
        )
        .child(
            v_flex()
                .flex_1()
                .min_h(px(0.))
                .gap_1()
                .p_2()
                .children(model.entries.iter().map(|entry| {
                    render_entry_row(
                        entry,
                        model.selected_entry_id.as_deref() == Some(entry.id.as_str()),
                        state,
                        cx,
                    )
                }))
                .when(model.entries.is_empty(), |this| {
                    this.child(
                        v_flex()
                            .flex_1()
                            .items_center()
                            .justify_center()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("No entries"),
                    )
                }),
        )
        .into_any_element()
}

fn render_entry_row(
    entry: &VaultEntry,
    selected: bool,
    state: &Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let entry_id = entry.id.clone();
    let state_for_click = state.clone();

    ListItem::new(format!("entry-{}", entry.id))
        .selected(selected)
        .w_full()
        .rounded(px(6.))
        .px_3()
        .py_2()
        .when(selected, |this| {
            this.bg(cx.theme().accent)
                .text_color(cx.theme().accent_foreground)
        })
        .when(!selected, |this| this.text_color(cx.theme().foreground))
        .on_click(move |_: &ClickEvent, _, cx| {
            state_for_click.update(cx, |state, cx| {
                state.select_entry(entry_id.clone(), cx);
            });
        })
        .child(
            v_flex()
                .gap_1()
                .min_w(px(0.))
                .child(
                    div()
                        .text_sm()
                        .font_medium()
                        .overflow_hidden()
                        .child(entry.title.clone()),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .text_xs()
                        .text_color(if selected {
                            cx.theme().accent_foreground.opacity(0.72)
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(if entry.username.is_empty() {
                            "No username".to_string()
                        } else {
                            entry.username.clone()
                        })
                        .when(!entry.url.is_empty(), |this| {
                            this.child(div().child("-")).child(entry.url.clone())
                        }),
                ),
        )
        .into_any_element()
}

fn render_entry_detail(model: &VaultBrowserModel, cx: &mut Context<AppShell>) -> AnyElement {
    v_flex()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .bg(cx.theme().background)
        .child(match &model.selected_entry {
            Some(entry) => render_selected_entry(entry, cx),
            None => render_no_entry(cx),
        })
        .into_any_element()
}

fn render_selected_entry(entry: &VaultEntry, cx: &mut Context<AppShell>) -> AnyElement {
    v_flex()
        .flex_1()
        .gap_5()
        .p_5()
        .child(
            v_flex()
                .gap_2()
                .child(div().text_2xl().font_semibold().child(entry.title.clone()))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(if entry.url.is_empty() {
                            "No URL".to_string()
                        } else {
                            entry.url.clone()
                        }),
                ),
        )
        .child(
            v_flex()
                .gap_3()
                .max_w(px(620.))
                .child(detail_row(
                    "Username",
                    display_or_empty(&entry.username),
                    cx,
                ))
                .child(detail_row("URL", display_or_empty(&entry.url), cx))
                .child(detail_row(
                    "Password",
                    if entry.has_password {
                        "Present".to_string()
                    } else {
                        "Not set".to_string()
                    },
                    cx,
                )),
        )
        .into_any_element()
}

fn render_no_entry(cx: &mut Context<AppShell>) -> AnyElement {
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_2()
        .text_color(cx.theme().muted_foreground)
        .child(div().text_lg().font_semibold().child("Select an entry"))
        .child(div().text_sm().child("Entry details will appear here."))
        .into_any_element()
}

fn detail_row(label: &'static str, value: String, cx: &mut Context<AppShell>) -> AnyElement {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_medium()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            div()
                .min_h(px(34.))
                .w_full()
                .rounded(px(6.))
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().secondary)
                .px_3()
                .py_2()
                .text_sm()
                .overflow_hidden()
                .child(value),
        )
        .into_any_element()
}

fn display_or_empty(value: &str) -> String {
    if value.is_empty() {
        "Not set".to_string()
    } else {
        value.to_string()
    }
}

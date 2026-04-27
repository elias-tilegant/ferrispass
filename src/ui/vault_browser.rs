use crate::{
    app::{AppState, CopyValueKind, VaultBrowserModel},
    domain::{VaultEntry, VaultGroup},
    ui::AppShell,
};
use gpui::{
    AnyElement, ClickEvent, ClipboardItem, Context, Entity, IntoElement as _, ParentElement as _,
    Styled as _, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, StyledExt as _, WindowExt as _,
    button::Button,
    h_flex,
    input::{Input, InputState},
    list::ListItem,
    v_flex,
};

pub fn render_group_tree(
    model: &VaultBrowserModel,
    state: &Entity<AppState>,
    search_input: &Entity<InputState>,
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
            search_input,
            0,
            cx,
        ))
        .into_any_element()
}

pub fn render_vault_browser(
    model: VaultBrowserModel,
    state: &Entity<AppState>,
    search_input: &Entity<InputState>,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    h_flex()
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .child(render_entry_list(&model, state, search_input, cx))
        .child(render_entry_detail(&model, state, cx))
        .into_any_element()
}

fn render_group_row(
    group: &VaultGroup,
    selected_group_id: &str,
    state: &Entity<AppState>,
    search_input: &Entity<InputState>,
    depth: usize,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let selected = group.id == selected_group_id;
    let group_id = group.id.clone();
    let state_for_click = state.clone();
    let search_input_for_click = search_input.clone();

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
                .on_click(move |_: &ClickEvent, window, cx| {
                    search_input_for_click.update(cx, |input, cx| {
                        input.set_value("", window, cx);
                    });
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
        .children(group.groups.iter().map(|group| {
            render_group_row(group, selected_group_id, state, search_input, depth + 1, cx)
        }))
        .into_any_element()
}

fn render_entry_list(
    model: &VaultBrowserModel,
    state: &Entity<AppState>,
    search_input: &Entity<InputState>,
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
                        .child(div().font_semibold().overflow_hidden().child(
                            if model.showing_search_results {
                                "Search results".to_string()
                            } else {
                                model.selected_group_name.clone()
                            },
                        ))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{} entries", model.entries.len())),
                        ),
                )
                .child(
                    Input::new(search_input)
                        .prefix(IconName::Search)
                        .cleanable(true),
                )
                .when(model.showing_search_results, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("Across vault for \"{}\"", model.search_query)),
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

fn render_entry_detail(
    model: &VaultBrowserModel,
    state: &Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    v_flex()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .bg(cx.theme().background)
        .child(match &model.selected_entry {
            Some(entry) => render_selected_entry(entry, state, cx),
            None => render_no_entry(cx),
        })
        .into_any_element()
}

fn render_selected_entry(
    entry: &VaultEntry,
    state: &Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> AnyElement {
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
            h_flex()
                .gap_2()
                .child(copy_button(
                    "copy-entry-username",
                    "Username",
                    CopyValueKind::Username,
                    entry.username.is_empty(),
                    state,
                ))
                .child(copy_button(
                    "copy-entry-url",
                    "URL",
                    CopyValueKind::Url,
                    entry.url.is_empty(),
                    state,
                ))
                .child(copy_button(
                    "copy-entry-password",
                    "Password",
                    CopyValueKind::Password,
                    !entry.has_password,
                    state,
                )),
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

fn copy_button(
    id: &'static str,
    label: &'static str,
    kind: CopyValueKind,
    disabled: bool,
    state: &Entity<AppState>,
) -> Button {
    let state_for_click = state.clone();

    Button::new(id)
        .outline()
        .small()
        .icon(IconName::Copy)
        .label(label)
        .tooltip(format!("Copy {}", label))
        .disabled(disabled)
        .on_click(move |_: &ClickEvent, window, cx| {
            if let Some(value) = state_for_click.read(cx).copy_selected_value(kind) {
                cx.write_to_clipboard(ClipboardItem::new_string(value));
                window.push_notification(format!("{} copied.", copy_value_label(kind)), cx);
            } else {
                window.push_notification(format!("No {} to copy.", copy_value_label(kind)), cx);
            }
        })
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

fn copy_value_label(kind: CopyValueKind) -> &'static str {
    match kind {
        CopyValueKind::Username => "Username",
        CopyValueKind::Url => "URL",
        CopyValueKind::Password => "Password",
    }
}

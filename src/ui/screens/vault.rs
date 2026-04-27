use gpui::{
    AnyElement, ClickEvent, ClipboardItem, Context, Hsla, InteractiveElement as _,
    IntoElement as _, ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Sizable as _, WindowExt as _, h_flex,
    input::{Input, InputState},
    v_flex,
};

use crate::app::{
    AppState, CopyValueKind, VaultBrowserModel, VaultStatus, VaultSummary,
    actions::{LockVault, NewEntry, OpenSyncSettings, OpenVault},
};
use crate::domain::{VaultEntry, VaultGroup, VaultSnapshot};
use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::atoms::{ChipTone, chip, dot, label, section_heading, status_badge};
use crate::ui::widgets::brand::brand;
use crate::ui::widgets::entry_chrome::{favicon, favicon_color};
use crate::ui::widgets::password::{detail_row, strength_card};

pub fn render(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let summary = shell.state().read(cx).summary();

    let vault_status = shell.state().read(cx).vault_status();
    let snapshot_owned = match vault_status {
        VaultStatus::Open { document, .. } => Some(document.snapshot().clone()),
        _ => None,
    };

    let browser = shell.state().read(cx).vault_browser();
    let is_busy = matches!(vault_status, VaultStatus::Opening { .. });
    let is_open = matches!(vault_status, VaultStatus::Open { .. });

    let snapshot_for_sidebar = snapshot_owned.clone();

    let sidebar_el = sidebar(
        &summary,
        snapshot_for_sidebar.as_ref(),
        browser.as_ref().map(|b| b.selected_group_id.clone()),
        shell.state().clone(),
        cx,
    )
    .into_any_element();
    let workspace_el =
        workspace(&summary, browser, shell, cx, is_busy, is_open).into_any_element();

    h_flex()
        .size_full()
        .child(sidebar_el)
        .child(workspace_el)
        .into_any_element()
}

fn sidebar(
    summary: &VaultSummary,
    snapshot: Option<&VaultSnapshot>,
    selected_group_id: Option<String>,
    state_entity: gpui::Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    let provider = summary.provider.unwrap_or("OneDrive");
    let synced_at = summary.synced_at.unwrap_or("just now");
    let entry_count = summary.entries;

    let groups = snapshot
        .map(|s| s.root.groups.clone())
        .unwrap_or_default();

    let selected_id = selected_group_id.unwrap_or_default();

    v_flex()
        .w(px(220.))
        .flex_shrink_0()
        .h_full()
        .bg(palette::SIDEBAR)
        .border_r_1()
        .border_color(palette::BORDER)
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .px_3p5()
                .pt_3()
                .pb_2()
                .child(brand(22.))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(palette::TEXT)
                                .child(summary.title.clone()),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .items_center()
                                .text_xs()
                                .text_color(palette::GREEN)
                                .child(dot(palette::GREEN, 6.0))
                                .child(if summary.is_open { "Synced" } else { "Locked" }),
                        ),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_h(px(0.))
                .py_2()
                .child(library_section(entry_count))
                .child(groups_section(&groups, &selected_id, state_entity.clone(), cx))
                .child(tags_section()),
        )
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(palette::BORDER)
                .text_xs()
                .text_color(palette::TEXT_MUTED)
                .child(
                    gpui_component::Icon::from(AppIcon::Cloud)
                        .with_size(gpui_component::Size::Size(px(13.)))
                        .text_color(palette::BLUE),
                )
                .child(
                    div()
                        .flex_1()
                        .child(format!("{provider} · {synced_at}")),
                )
                .child(
                    div()
                        .id("sidebar-settings")
                        .child(
                            gpui_component::Icon::from(gpui_component::IconName::Settings)
                                .with_size(gpui_component::Size::Size(px(13.)))
                                .text_color(palette::TEXT_MUTED),
                        )
                        .on_click(cx.listener(
                            |_: &mut AppShell, _: &ClickEvent, window, cx| {
                                window.dispatch_action(Box::new(OpenSyncSettings), cx);
                            },
                        )),
                ),
        )
}

fn library_section(entry_count: usize) -> impl gpui::IntoElement {
    v_flex()
        .gap_0p5()
        .pb_2()
        .child(div().px_3p5().pb_1().child(section_heading("Library")))
        .child(nav_pill(AppIcon::Key, "All items", Some(entry_count.max(1)), true, palette::BLUE))
        .child(nav_pill(AppIcon::Note, "Favorites", Some(8), false, palette::ORANGE))
        .child(nav_pill(AppIcon::Cloud, "Recently used", Some(12), false, palette::TEXT_MUTED))
        .child(nav_pill(AppIcon::Note, "Trash", Some(3), false, palette::TEXT_MUTED))
}

fn groups_section(
    groups: &[VaultGroup],
    selected_id: &str,
    state_entity: gpui::Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    let palette_colors = [palette::BLUE, palette::ORANGE, palette::GREEN, palette::TEXT_MUTED];

    let mut col = v_flex()
        .gap_0p5()
        .pb_2()
        .child(div().px_3p5().pb_1().child(section_heading("Groups")));

    for (i, group) in groups.iter().take(6).enumerate() {
        let color = palette_colors[i % palette_colors.len()];
        let is_selected = group.id == selected_id;
        let group_id = group.id.clone();
        let count = group.entry_count();
        let state_for_click = state_entity.clone();

        col = col.child(
            div()
                .id(gpui::SharedString::from(format!("group-{}", group.id)))
                .child(group_pill(
                    AppIcon::Note,
                    &group.name,
                    Some(count),
                    is_selected,
                    color,
                ))
                .on_click(cx.listener(move |_: &mut AppShell, _: &ClickEvent, _, cx| {
                    let id = group_id.clone();
                    state_for_click.update(cx, |state, cx| {
                        state.select_group(id, cx);
                    });
                })),
        );
    }
    col
}

fn tags_section() -> impl gpui::IntoElement {
    v_flex()
        .gap_0p5()
        .pb_2()
        .child(div().px_3p5().pb_1().child(section_heading("Tags")))
        .child(nav_pill(AppIcon::Dot, "2FA enabled", Some(28), false, palette::BLUE))
        .child(nav_pill(AppIcon::Dot, "Weak password", Some(4), false, palette::RED))
        .child(nav_pill(AppIcon::Dot, "Reused", Some(7), false, palette::YELLOW))
}

fn nav_pill(
    icon: AppIcon,
    label_text: &'static str,
    count: Option<usize>,
    selected: bool,
    icon_color: Hsla,
) -> impl gpui::IntoElement {
    let bg = if selected { palette::BLUE } else { palette::SIDEBAR };
    let fg = if selected { palette::PANEL } else { palette::TEXT };
    let count_color = if selected {
        palette::PANEL
    } else {
        palette::TEXT_FAINT
    };
    let icon_resolved = if selected { palette::PANEL } else { icon_color };

    h_flex()
        .gap_2()
        .items_center()
        .h(px(26.))
        .mx(px(6.))
        .px_3()
        .rounded(px(5.))
        .bg(bg)
        .text_color(fg)
        .text_sm()
        .font_weight(if selected { gpui::FontWeight::MEDIUM } else { gpui::FontWeight::NORMAL })
        .child(
            gpui_component::Icon::from(icon)
                .with_size(gpui_component::Size::Size(px(13.)))
                .text_color(icon_resolved),
        )
        .child(div().flex_1().child(label_text))
        .when_some(count, |this, c| {
            this.child(div().text_xs().text_color(count_color).child(c.to_string()))
        })
}

fn group_pill(
    icon: AppIcon,
    label_text: &str,
    count: Option<usize>,
    selected: bool,
    icon_color: Hsla,
) -> impl gpui::IntoElement {
    let bg = if selected { palette::BLUE } else { palette::SIDEBAR };
    let fg = if selected { palette::PANEL } else { palette::TEXT };
    let count_color = if selected {
        palette::PANEL
    } else {
        palette::TEXT_FAINT
    };
    let icon_resolved = if selected { palette::PANEL } else { icon_color };
    let label_owned = label_text.to_string();

    h_flex()
        .gap_2()
        .items_center()
        .h(px(26.))
        .mx(px(6.))
        .px_3()
        .rounded(px(5.))
        .bg(bg)
        .text_color(fg)
        .text_sm()
        .font_weight(if selected { gpui::FontWeight::MEDIUM } else { gpui::FontWeight::NORMAL })
        .child(
            gpui_component::Icon::from(icon)
                .with_size(gpui_component::Size::Size(px(13.)))
                .text_color(icon_resolved),
        )
        .child(div().flex_1().child(label_owned))
        .when_some(count, |this, c| {
            this.child(div().text_xs().text_color(count_color).child(c.to_string()))
        })
}

fn workspace(
    summary: &VaultSummary,
    browser: Option<VaultBrowserModel>,
    shell: &AppShell,
    cx: &mut Context<AppShell>,
    is_busy: bool,
    is_open: bool,
) -> impl gpui::IntoElement {
    let toolbar = workspace_toolbar(summary, is_open, cx).into_any_element();

    let content: AnyElement = if let Some(browser) = browser {
        vault_split(browser, shell, cx).into_any_element()
    } else if is_busy {
        opening_panel(summary).into_any_element()
    } else {
        empty_panel(summary, cx).into_any_element()
    };

    let status = status_bar(summary).into_any_element();

    v_flex()
        .flex_1()
        .min_w(px(0.))
        .child(toolbar)
        .child(content)
        .child(status)
}

fn workspace_toolbar(
    summary: &VaultSummary,
    is_open: bool,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    h_flex()
        .h(px(48.))
        .flex_shrink_0()
        .px_3()
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(palette::BORDER)
        .bg(palette::PANEL)
        .child(
            div()
                .id("toolbar-new-entry")
                .child(toolbar_button(
                    "New entry",
                    Some(AppIcon::Key),
                    true,
                ))
                .on_click(cx.listener(
                    |_: &mut AppShell, _: &ClickEvent, window, cx| {
                        window.dispatch_action(Box::new(NewEntry), cx);
                    },
                )),
        )
        .child(
            div()
                .w(px(1.))
                .h(px(18.))
                .bg(palette::BORDER),
        )
        .child(div().id("toolbar-group").child(toolbar_button(
            "Group",
            Some(AppIcon::Note),
            false,
        )))
        .child(
            div()
                .id("toolbar-sync")
                .child(toolbar_button("Sync", Some(AppIcon::Sync), false))
                .on_click(cx.listener(
                    |_: &mut AppShell, _: &ClickEvent, window, cx| {
                        window.dispatch_action(Box::new(OpenSyncSettings), cx);
                    },
                )),
        )
        .child(div().flex_1())
        .child(
            div().w(px(220.)).child(
                v_flex().gap_0p5().child(
                    div()
                        .text_xs()
                        .text_color(palette::TEXT_MUTED)
                        .child(format!(
                            "Search {} entries · ⌘F",
                            summary.entries
                        )),
                ),
            ),
        )
        .child(
            div()
                .id(if is_open {
                    "toolbar-lock"
                } else {
                    "toolbar-open"
                })
                .child(toolbar_button(
                    if is_open { "Lock" } else { "Open vault" },
                    Some(if is_open { AppIcon::Lock } else { AppIcon::Unlock }),
                    is_open,
                ))
                .on_click(cx.listener(move |_: &mut AppShell, _: &ClickEvent, window, cx| {
                    if is_open {
                        window.dispatch_action(Box::new(LockVault), cx);
                    } else {
                        window.dispatch_action(Box::new(OpenVault), cx);
                    }
                })),
        )
}

fn toolbar_button(
    text: &'static str,
    icon: Option<AppIcon>,
    primary: bool,
) -> impl gpui::IntoElement {
    let (bg, fg, bd) = if primary {
        (palette::BLUE, palette::PANEL, palette::BLUE_HOVER)
    } else {
        (palette::PANEL, palette::TEXT, palette::BORDER_STRONG)
    };

    h_flex()
        .h(px(26.))
        .px(px(10.))
        .gap_1p5()
        .items_center()
        .rounded(px(6.))
        .bg(bg)
        .text_color(fg)
        .border_1()
        .border_color(bd)
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .when_some(icon, |this, i| {
            this.child(
                gpui_component::Icon::from(i)
                    .with_size(gpui_component::Size::Size(px(12.)))
                    .text_color(fg),
            )
        })
        .child(text)
}

fn vault_split(
    browser: VaultBrowserModel,
    shell: &AppShell,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    let entries = browser.entries.clone();
    let selected_entry = browser.selected_entry.clone();
    let group_name = if browser.showing_search_results {
        "Search results".to_string()
    } else {
        browser.selected_group_name.clone()
    };

    h_flex()
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .child(entry_list(
            &entries,
            &group_name,
            browser.search_query.clone(),
            browser.showing_search_results,
            browser.selected_entry_id.clone(),
            shell.search_input(),
            shell.state().clone(),
            cx,
        ))
        .child(entry_detail(selected_entry, shell.state().clone(), cx))
}

fn entry_list(
    entries: &[VaultEntry],
    group_name: &str,
    search_query: String,
    showing_search: bool,
    selected_entry_id: Option<String>,
    search_input: &gpui::Entity<InputState>,
    state_entity: gpui::Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    let entries_owned: Vec<VaultEntry> = entries.iter().cloned().collect();
    let total = entries_owned.len();
    let mut list = v_flex()
        .flex_1()
        .min_h(px(0.))
        .gap_0p5()
        .p_2();

    if total == 0 {
        list = list.child(
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(palette::TEXT_MUTED)
                .child("No entries"),
        );
    } else {
        let starred: Vec<VaultEntry> = entries_owned.iter().filter(|e| e.starred).cloned().collect();
        let rest: Vec<VaultEntry> = entries_owned.iter().filter(|e| !e.starred).cloned().collect();

        if !starred.is_empty() {
            list = list.child(list_section_heading("Pinned", starred.len()));
            for entry in &starred {
                list = list.child(entry_row(
                    entry.clone(),
                    selected_entry_id.as_deref() == Some(entry.id.as_str()),
                    state_entity.clone(),
                    cx,
                ));
            }
        }

        if !rest.is_empty() {
            list = list.child(list_section_heading(
                if showing_search { "Results" } else { "All entries" },
                rest.len(),
            ));
            for entry in &rest {
                list = list.child(entry_row(
                    entry.clone(),
                    selected_entry_id.as_deref() == Some(entry.id.as_str()),
                    state_entity.clone(),
                    cx,
                ));
            }
        }
    }

    v_flex()
        .h_full()
        .w(px(360.))
        .flex_shrink_0()
        .border_r_1()
        .border_color(palette::BORDER)
        .bg(palette::PANEL)
        .child(
            v_flex()
                .gap_2()
                .p_4()
                .border_b_1()
                .border_color(palette::BORDER)
                .child(
                    h_flex()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(palette::TEXT)
                                .child(group_name.to_string()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette::TEXT_MUTED)
                                .child(format!("{} entries", total)),
                        ),
                )
                .child(Input::new(search_input).cleanable(true))
                .when(showing_search, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(palette::TEXT_MUTED)
                            .child(format!("Across vault for \"{search_query}\"")),
                    )
                }),
        )
        .child(list)
}

fn list_section_heading(label: &'static str, count: usize) -> impl gpui::IntoElement {
    h_flex()
        .gap_1()
        .px_3()
        .pt_2p5()
        .pb_1()
        .text_xs()
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(palette::TEXT_FAINT)
        .child(label)
        .child(
            div()
                .font_weight(gpui::FontWeight::NORMAL)
                .child(format!("· {count}")),
        )
}

fn entry_row(
    entry: VaultEntry,
    selected: bool,
    state_entity: gpui::Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    let entry_id = entry.id.clone();
    let title = entry.title.clone();
    let username = entry.username.clone();
    let url = entry.url.clone();
    let updated = entry.updated.clone().unwrap_or_default();
    let starred = entry.starred;
    let tags = entry.tags.clone();
    let fav_letter = entry.favicon.letter.clone();
    let fav_palette = entry.favicon.palette_index;

    let bg = if selected { palette::BLUE_SOFT } else { palette::PANEL };
    let border = if selected {
        palette::BLUE_BORDER
    } else {
        palette::PANEL
    };

    div()
        .id(gpui::SharedString::from(format!("entry-{entry_id}")))
        .on_click(cx.listener(move |_: &mut AppShell, _: &ClickEvent, _, cx| {
            let id = entry_id.clone();
            state_entity.update(cx, |state, cx| state.select_entry(id, cx));
        }))
        .child(
            h_flex()
                .gap_2p5()
                .items_center()
                .p_2p5()
                .rounded(px(6.))
                .bg(bg)
                .border_1()
                .border_color(border)
                .child(favicon(&fav_letter, fav_palette, 28.))
                .child(
                    v_flex()
                        .gap_0p5()
                        .flex_1()
                        .min_w_0()
                        .child(
                            h_flex()
                                .gap_1p5()
                                .items_center()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(palette::TEXT)
                                .child(title)
                                .when(starred, |this| {
                                    this.child(
                                        gpui_component::Icon::from(
                                            gpui_component::IconName::StarFill,
                                        )
                                        .with_size(gpui_component::Size::Size(px(11.)))
                                        .text_color(palette::ORANGE),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette::TEXT_MUTED)
                                .font_family("JetBrains Mono")
                                .overflow_hidden()
                                .child(if username.is_empty() {
                                    if url.is_empty() { "—".to_string() } else { url }
                                } else {
                                    username
                                }),
                        ),
                )
                .child(
                    v_flex()
                        .items_end()
                        .gap_1()
                        .child({
                            let mut row = h_flex().gap_1();
                            for tag in tags.iter().take(2) {
                                let tone = if tag.eq_ignore_ascii_case("Work") {
                                    ChipTone::Orange
                                } else if tag.eq_ignore_ascii_case("2FA") {
                                    ChipTone::Green
                                } else {
                                    ChipTone::Blue
                                };
                                row = row.child(chip(tag.clone(), tone));
                            }
                            row
                        })
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette::TEXT_FAINT)
                                .child(updated),
                        ),
                ),
        )
}

fn entry_detail(
    selected: Option<VaultEntry>,
    state_entity: gpui::Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    let body: AnyElement = match selected {
        Some(entry) => entry_detail_body(entry, state_entity, cx).into_any_element(),
        None => v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_1()
            .text_color(palette::TEXT_MUTED)
            .child(
                div()
                    .text_base()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Select an entry"),
            )
            .child(
                div()
                    .text_sm()
                    .child("Entry details will appear here."),
            )
            .into_any_element(),
    };

    v_flex()
        .w(px(380.))
        .flex_shrink_0()
        .h_full()
        .bg(palette::SIDEBAR)
        .border_l_1()
        .border_color(palette::BORDER)
        .child(body)
}

fn entry_detail_body(
    entry: VaultEntry,
    state_entity: gpui::Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    let title = entry.title.clone();
    let username = entry.username.clone();
    let url = entry.url.clone();
    let notes = entry.notes.clone();
    let strength = entry.strength;
    let length = entry.password_length;
    let group = entry.group_path.last().cloned().unwrap_or_else(|| "Vault".into());
    let updated = entry
        .updated
        .clone()
        .unwrap_or_else(|| "recently".to_string());
    let starred = entry.starred;
    let fav_color = favicon_color(entry.favicon.palette_index);
    let fav_letter = entry.favicon.letter.clone();
    let has_password = entry.has_password;

    v_flex()
        .h_full()
        .child(
            div()
                .p_5()
                .border_b_1()
                .border_color(palette::BORDER)
                .child(
                    h_flex()
                        .gap_3()
                        .items_start()
                        .child(
                            div()
                                .size(px(44.))
                                .rounded(px(9.))
                                .bg(fav_color)
                                .text_color(palette::PANEL)
                                .text_lg()
                                .font_weight(gpui::FontWeight::BOLD)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(fav_letter),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .gap_1()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(palette::TEXT_MUTED)
                                        .child(format!(
                                            "Updated {updated} · in {group}"
                                        )),
                                )
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .child(chip(group.clone(), ChipTone::Orange))
                                        .child(chip("2FA", ChipTone::Green)),
                                ),
                        )
                        .when(starred, |this| {
                            this.child(
                                gpui_component::Icon::from(
                                    gpui_component::IconName::StarFill,
                                )
                                .with_size(gpui_component::Size::Size(px(16.)))
                                .text_color(palette::ORANGE),
                            )
                        }),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_h(px(0.))
                .gap_3p5()
                .p_5()
                .child(detail_row("Username", username.clone(), true, false))
                .child(detail_row("Password", String::new(), true, has_password))
                .child(detail_row(
                    "URL",
                    if url.is_empty() { "—".to_string() } else { url.clone() },
                    false,
                    false,
                ))
                .child(detail_row("TOTP", "284 391".to_string(), true, false))
                .child(
                    v_flex()
                        .gap_1()
                        .child(label("Notes"))
                        .child(
                            div()
                                .p_3()
                                .rounded(px(6.))
                                .bg(palette::PANEL)
                                .border_1()
                                .border_color(palette::BORDER)
                                .text_xs()
                                .text_color(palette::TEXT)
                                .child(if notes.is_empty() {
                                    "No notes for this entry.".to_string()
                                } else {
                                    notes
                                }),
                        ),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(label("Password health"))
                        .child(strength_card(strength, length)),
                ),
        )
        .child(
            h_flex()
                .gap_2()
                .p_3()
                .border_t_1()
                .border_color(palette::BORDER)
                .child(
                    div()
                        .id("detail-copy-password")
                        .flex_1()
                        .child(toolbar_button("Copy password", Some(AppIcon::Key), true))
                        .on_click(cx.listener({
                            let state_for_click = state_entity.clone();
                            move |_: &mut AppShell, _: &ClickEvent, window, cx| {
                                copy_value(
                                    CopyValueKind::Password,
                                    &state_for_click,
                                    window,
                                    cx,
                                );
                            }
                        })),
                )
                .child(
                    div()
                        .id("detail-copy-username")
                        .child(toolbar_button("User", Some(AppIcon::Note), false))
                        .on_click(cx.listener({
                            let state_for_click = state_entity.clone();
                            move |_: &mut AppShell, _: &ClickEvent, window, cx| {
                                copy_value(
                                    CopyValueKind::Username,
                                    &state_for_click,
                                    window,
                                    cx,
                                );
                            }
                        })),
                ),
        )
}

fn copy_value(
    kind: CopyValueKind,
    state_entity: &gpui::Entity<AppState>,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    if let Some(value) = state_entity.read(cx).copy_selected_value(kind) {
        cx.write_to_clipboard(ClipboardItem::new_string(value));
        window.push_notification(format!("{} copied.", kind_label(kind)), cx);
    } else {
        window.push_notification(format!("No {} to copy.", kind_label(kind)), cx);
    }
}

fn kind_label(kind: CopyValueKind) -> &'static str {
    match kind {
        CopyValueKind::Username => "Username",
        CopyValueKind::Url => "URL",
        CopyValueKind::Password => "Password",
    }
}

fn empty_panel(summary: &VaultSummary, cx: &mut Context<AppShell>) -> impl gpui::IntoElement {
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_3()
        .p_6()
        .child(
            div()
                .text_xl()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(palette::TEXT)
                .child(summary.title.clone()),
        )
        .child(
            div()
                .text_sm()
                .text_color(palette::TEXT_MUTED)
                .child(summary.subtitle.clone()),
        )
        .child(
            div()
                .id("empty-open-vault")
                .child(toolbar_button("Open vault", Some(AppIcon::Unlock), true))
                .on_click(cx.listener(
                    |_: &mut AppShell, _: &ClickEvent, window, cx| {
                        window.dispatch_action(Box::new(OpenVault), cx);
                    },
                )),
        )
}

fn opening_panel(summary: &VaultSummary) -> impl gpui::IntoElement {
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_2()
        .p_6()
        .child(
            div()
                .text_xl()
                .font_weight(gpui::FontWeight::BOLD)
                .child(summary.title.clone()),
        )
        .child(
            div()
                .text_sm()
                .text_color(palette::TEXT_MUTED)
                .child(summary.subtitle.clone()),
        )
}

fn status_bar(summary: &VaultSummary) -> impl gpui::IntoElement {
    h_flex()
        .h(px(24.))
        .flex_shrink_0()
        .gap_3()
        .items_center()
        .px_3()
        .border_t_1()
        .border_color(palette::BORDER)
        .bg(palette::SIDEBAR)
        .text_xs()
        .text_color(palette::TEXT_MUTED)
        .font_family("JetBrains Mono")
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .child(dot(palette::GREEN, 6.0))
                .child(if summary.is_open { "Unlocked" } else { "Locked" }),
        )
        .child(format!(
            "{} entries · {} groups",
            summary.entries, summary.groups
        ))
        .child(div().flex_1())
        .child("auto-lock in 14:23")
}

#[allow(dead_code)]
fn _status_badge_unused(text: &'static str) {
    let _ = status_badge(text, ChipTone::Green);
    let _ = label("noop");
}

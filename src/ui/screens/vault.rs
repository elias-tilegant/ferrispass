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
    let state = shell.state().read(cx);
    let summary = state.summary();
    let vault_status = state.vault_status();
    let is_busy = matches!(vault_status, VaultStatus::Opening { .. });
    let is_open = matches!(vault_status, VaultStatus::Open { .. });

    // O(1) snapshot share — keeps render off the deep-clone path.
    let snapshot = match vault_status {
        VaultStatus::Open { document, .. } => Some(document.snapshot_rc()),
        _ => None,
    };
    let browser = state.vault_browser();

    let selection = browser
        .as_ref()
        .map(|b| b.selection.clone())
        .unwrap_or(crate::app::LibrarySelection::AllItems);
    let sidebar_el = sidebar(
        &summary,
        snapshot.as_deref(),
        &selection,
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
    selection: &crate::app::LibrarySelection,
    state_entity: gpui::Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    let provider = summary.provider.unwrap_or("OneDrive");
    let synced_at = summary.synced_at.unwrap_or("just now");
    let entry_count = summary.entries;
    let starred_count = snapshot
        .map(|s| s.entries_starred().len())
        .unwrap_or(0);
    let twofa_count = snapshot
        .map(|s| s.entries_with_tag("2FA").len())
        .unwrap_or(0);

    let groups = snapshot
        .map(|s| s.root.groups.clone())
        .unwrap_or_default();

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
                .id("sidebar-scroll")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .py_2()
                .child(library_section(
                    selection,
                    entry_count,
                    starred_count,
                    state_entity.clone(),
                    cx,
                ))
                .child(groups_section(&groups, selection, state_entity.clone(), cx))
                .child(tags_section(twofa_count, selection, state_entity.clone(), cx)),
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

fn library_section(
    selection: &crate::app::LibrarySelection,
    entry_count: usize,
    starred_count: usize,
    state_entity: gpui::Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    use crate::app::LibrarySelection as L;

    v_flex()
        .gap_0p5()
        .pb_2()
        .child(div().px_3p5().pb_1().child(section_heading("Library")))
        .child(nav_row(
            "lib-all",
            AppIcon::Key,
            "All items",
            Some(entry_count),
            selection.is_all_items(),
            palette::BLUE,
            state_entity.clone(),
            L::AllItems,
            cx,
        ))
        .child(nav_row(
            "lib-favorites",
            AppIcon::Note,
            "Favorites",
            Some(starred_count),
            selection.is_favorites(),
            palette::ORANGE,
            state_entity.clone(),
            L::Favorites,
            cx,
        ))
        .child(nav_row(
            "lib-recent",
            AppIcon::Cloud,
            "Recently used",
            None,
            selection.is_recently_used(),
            palette::TEXT_MUTED,
            state_entity.clone(),
            L::RecentlyUsed,
            cx,
        ))
        .child(nav_row(
            "lib-trash",
            AppIcon::Note,
            "Trash",
            None,
            selection.is_trash(),
            palette::TEXT_MUTED,
            state_entity,
            L::Trash,
            cx,
        ))
}

fn groups_section(
    groups: &[VaultGroup],
    selection: &crate::app::LibrarySelection,
    state_entity: gpui::Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    let palette_colors = [palette::BLUE, palette::ORANGE, palette::GREEN, palette::TEXT_MUTED];
    let selected_group = selection.group_id().unwrap_or_default().to_string();

    let mut col = v_flex()
        .gap_0p5()
        .pb_2()
        .child(div().px_3p5().pb_1().child(section_heading("Groups")));

    for (i, group) in groups.iter().enumerate() {
        let color = palette_colors[i % palette_colors.len()];
        let is_selected = group.id == selected_group;
        let group_id = group.id.clone();
        let count = group.entry_count();
        let state_for_click = state_entity.clone();

        col = col.child(
            div()
                .id(gpui::SharedString::from(format!("group-{}", group.id)))
                .child(nav_pill_visual(
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

fn tags_section(
    twofa_count: usize,
    selection: &crate::app::LibrarySelection,
    state_entity: gpui::Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    use crate::app::LibrarySelection as L;
    let selected_tag = selection.tag().unwrap_or_default().to_string();

    v_flex()
        .gap_0p5()
        .pb_2()
        .child(div().px_3p5().pb_1().child(section_heading("Tags")))
        .child(nav_row(
            "tag-2fa",
            AppIcon::Dot,
            "2FA enabled",
            Some(twofa_count),
            selected_tag.eq_ignore_ascii_case("2FA"),
            palette::BLUE,
            state_entity.clone(),
            L::Tag("2FA".to_string()),
            cx,
        ))
        .child(nav_row(
            "tag-personal",
            AppIcon::Dot,
            "Personal",
            None,
            selected_tag.eq_ignore_ascii_case("Personal"),
            palette::GREEN,
            state_entity.clone(),
            L::Tag("Personal".to_string()),
            cx,
        ))
        .child(nav_row(
            "tag-work",
            AppIcon::Dot,
            "Work",
            None,
            selected_tag.eq_ignore_ascii_case("Work"),
            palette::YELLOW,
            state_entity,
            L::Tag("Work".to_string()),
            cx,
        ))
}

#[allow(clippy::too_many_arguments)]
fn nav_row(
    id: &'static str,
    icon: AppIcon,
    label_text: &'static str,
    count: Option<usize>,
    selected: bool,
    icon_color: Hsla,
    state_entity: gpui::Entity<AppState>,
    target: crate::app::LibrarySelection,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    div()
        .id(id)
        .child(nav_pill_visual(icon, label_text, count, selected, icon_color))
        .on_click(cx.listener(move |_: &mut AppShell, _: &ClickEvent, _, cx| {
            let target = target.clone();
            state_entity.update(cx, |state, cx| {
                state.select_library(target, cx);
            });
        }))
}

fn nav_pill_visual(
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
        .font_weight(if selected {
            gpui::FontWeight::MEDIUM
        } else {
            gpui::FontWeight::NORMAL
        })
        .child(
            gpui_component::Icon::from(icon)
                .with_size(gpui_component::Size::Size(px(13.)))
                .text_color(icon_resolved),
        )
        .child(div().flex_1().min_w_0().truncate().child(label_owned))
        .when_some(count, |this, c| {
            this.child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(count_color)
                    .child(c.to_string()),
            )
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
    let toolbar = workspace_toolbar(summary, shell, is_open, cx).into_any_element();

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
        .min_h(px(0.))
        .h_full()
        .overflow_hidden()
        .child(toolbar)
        .child(content)
        .child(status)
}

fn workspace_toolbar(
    _summary: &VaultSummary,
    shell: &AppShell,
    is_open: bool,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    let search_input = shell.search_input().clone();
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
            div()
                .w(px(280.))
                .child(Input::new(&search_input).cleanable(true)),
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

#[derive(Clone, Copy)]
enum ActionStyle {
    Primary,
    Default,
}

#[allow(clippy::too_many_arguments)]
fn action_button(
    id: &'static str,
    text: &'static str,
    icon: AppIcon,
    style: ActionStyle,
    enabled: bool,
    flex: bool,
    kind: CopyValueKind,
    state_entity: gpui::Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    let (bg, fg, bd) = match (style, enabled) {
        (ActionStyle::Primary, true) => (palette::BLUE, palette::PANEL, palette::BLUE_HOVER),
        (ActionStyle::Primary, false) => {
            (palette::SIDEBAR, palette::TEXT_FAINT, palette::BORDER_STRONG)
        }
        (ActionStyle::Default, true) => (palette::PANEL, palette::TEXT, palette::BORDER_STRONG),
        (ActionStyle::Default, false) => (palette::SIDEBAR, palette::TEXT_FAINT, palette::BORDER),
    };

    let mut row = div().id(id);
    if flex {
        row = row.flex_1();
    }

    row.child(
        h_flex()
            .h(px(28.))
            .px(px(12.))
            .gap_1p5()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .bg(bg)
            .text_color(fg)
            .border_1()
            .border_color(bd)
            .text_xs()
            .font_weight(gpui::FontWeight::MEDIUM)
            .child(
                gpui_component::Icon::from(icon)
                    .with_size(gpui_component::Size::Size(px(12.)))
                    .text_color(fg),
            )
            .child(text),
    )
    .when(enabled, move |this| {
        this.on_click(cx.listener(move |_: &mut AppShell, _: &ClickEvent, window, cx| {
            copy_value(kind, &state_entity, window, cx);
        }))
    })
}

fn vault_split(
    browser: VaultBrowserModel,
    shell: &AppShell,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    let entries = std::rc::Rc::clone(&browser.entries);
    let selected_entry = browser.selected_entry.clone();
    let group_name = if browser.showing_search_results {
        "Search results".to_string()
    } else {
        browser.selection_label.clone()
    };

    h_flex()
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .overflow_hidden()
        .child(entry_list(
            entries,
            &group_name,
            browser.search_query.clone(),
            browser.showing_search_results,
            browser.selected_entry_id.clone(),
            shell.search_input(),
            shell.state().clone(),
            shell.entry_list_scroll().clone(),
            cx,
        ))
        .child(entry_detail(selected_entry, shell.state().clone(), cx))
}

/// One row in the virtual entry list — either a section heading or an index
/// into the shared `Rc<Vec<VaultEntry>>` (avoids cloning entries per frame).
#[derive(Clone, Copy)]
enum ListRow {
    Header { label: &'static str, count: usize },
    Entry(usize),
}

const ROW_HEIGHT: f32 = 56.0;
const ROW_GAP: f32 = 2.0;
const HEADER_HEIGHT: f32 = 28.0;

#[allow(clippy::too_many_arguments)]
fn entry_list(
    entries: std::rc::Rc<Vec<VaultEntry>>,
    group_name: &str,
    search_query: String,
    showing_search: bool,
    selected_entry_id: Option<String>,
    _search_input: &gpui::Entity<InputState>,
    state_entity: gpui::Entity<AppState>,
    scroll_handle_for_virtual: gpui_component::VirtualListScrollHandle,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    let total = entries.len();

    // Build the flat virtual-row list once per render. We store INDEX into the shared
    // Rc<Vec<VaultEntry>> rather than cloning each entry — keeps per-frame allocation
    // proportional to the number of rows, not the size of each entry's strings.
    let mut rows: Vec<ListRow> = Vec::with_capacity(total + 2);
    let mut starred_count = 0;
    let mut rest_count = 0;
    for entry in entries.iter() {
        if entry.starred {
            starred_count += 1;
        } else {
            rest_count += 1;
        }
    }

    if starred_count > 0 {
        rows.push(ListRow::Header {
            label: "Pinned",
            count: starred_count,
        });
        rows.extend(
            entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.starred)
                .map(|(ix, _)| ListRow::Entry(ix)),
        );
    }
    if rest_count > 0 {
        rows.push(ListRow::Header {
            label: if showing_search { "Results" } else { "All entries" },
            count: rest_count,
        });
        rows.extend(
            entries
                .iter()
                .enumerate()
                .filter(|(_, e)| !e.starred)
                .map(|(ix, _)| ListRow::Entry(ix)),
        );
    }

    let item_sizes: std::rc::Rc<Vec<gpui::Size<gpui::Pixels>>> = std::rc::Rc::new(
        rows.iter()
            .map(|row| match row {
                ListRow::Header { .. } => gpui::size(px(0.), px(HEADER_HEIGHT)),
                ListRow::Entry(_) => gpui::size(px(0.), px(ROW_HEIGHT + ROW_GAP)),
            })
            .collect(),
    );

    let rows_rc: std::rc::Rc<Vec<ListRow>> = std::rc::Rc::new(rows);
    let entries_for_render = std::rc::Rc::clone(&entries);
    let selected_id_for_render = selected_entry_id.clone();
    let state_for_render = state_entity.clone();

    let body: gpui::AnyElement = if total == 0 {
        v_flex()
            .id("entry-list-empty")
            .flex_1()
            .min_h(px(0.))
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(palette::TEXT_MUTED)
            .child("No entries")
            .into_any_element()
    } else {
        let scroll_handle = scroll_handle_for_virtual.clone();
        let virtual_list = gpui_component::v_virtual_list(
            cx.entity().clone(),
            "entry-list-virtual",
            item_sizes,
            move |_shell: &mut AppShell, range, _window, cx| {
                let rows = rows_rc.clone();
                let entries = std::rc::Rc::clone(&entries_for_render);
                let selected_id = selected_id_for_render.clone();
                let state_entity = state_for_render.clone();
                range
                    .map(|ix| -> gpui::AnyElement {
                        match rows.get(ix).copied() {
                            Some(ListRow::Header { label, count }) => {
                                list_section_heading(label, count).into_any_element()
                            }
                            Some(ListRow::Entry(entry_ix)) => {
                                let Some(entry) = entries.get(entry_ix) else {
                                    return div().into_any_element();
                                };
                                let is_selected =
                                    selected_id.as_deref() == Some(entry.id.as_str());
                                entry_row(
                                    entry.clone(),
                                    is_selected,
                                    state_entity.clone(),
                                    cx,
                                )
                                .into_any_element()
                            }
                            None => div().into_any_element(),
                        }
                    })
                    .collect()
            },
        )
        .track_scroll(&scroll_handle);

        // Pattern lifted from gpui-component's virtual_list_story: the virtual list
        // needs a definite-height parent (`flex_1 + min_h:0` provides that inside the
        // panel column) AND a `relative + size_full` immediate parent so its internal
        // `size_full + overflow_scroll` resolves to a real scroll viewport. Without
        // the inner wrapper, flex layout can leave the list's height ambiguous and
        // mouse-wheel events don't bind to a scrollable bounds.
        v_flex()
            .id("entry-list-viewport")
            .flex_1()
            .min_h(px(0.))
            .relative()
            .size_full()
            .px_2()
            .child(virtual_list)
            .into_any_element()
    };

    v_flex()
        .h_full()
        .min_h(px(0.))
        .w(px(360.))
        .flex_shrink_0()
        .overflow_hidden()
        .border_r_1()
        .border_color(palette::BORDER)
        .bg(palette::PANEL)
        .child(
            v_flex()
                .flex_shrink_0()
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
                                .flex_1()
                                .min_w(px(0.))
                                .truncate()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(palette::TEXT)
                                .child(group_name.to_string()),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(palette::TEXT_MUTED)
                                .child(format!("{} entries", total)),
                        ),
                )
                .when(showing_search, |this| {
                    this.child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(palette::TEXT_MUTED)
                            .child(format!("Across vault for \"{search_query}\"")),
                    )
                }),
        )
        .child(body)
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
                .h(px(56.))
                .flex_shrink_0()
                .p_2p5()
                .overflow_hidden()
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
                        .overflow_hidden()
                        .child(
                            h_flex()
                                .gap_1p5()
                                .items_center()
                                .min_w(px(0.))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.))
                                        .truncate()
                                        .text_sm()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(palette::TEXT)
                                        .child(title),
                                )
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
                                .truncate()
                                .text_xs()
                                .text_color(palette::TEXT_MUTED)
                                .font_family("JetBrains Mono")
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
                        .flex_shrink_0()
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
                                .whitespace_nowrap()
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
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .h_full()
        .overflow_hidden()
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
    let group = entry
        .group_path
        .last()
        .cloned()
        .unwrap_or_else(|| "Vault root".into());
    let updated = entry.updated.clone();
    let starred = entry.starred;
    let fav_color = favicon_color(entry.favicon.palette_index);
    let fav_letter = entry.favicon.letter.clone();
    let has_password = entry.has_password;
    let has_otp = entry.has_otp;
    let tags = entry.tags.clone();

    let mut chips_row = h_flex().gap_1().flex_wrap();
    for tag in tags.iter().take(4) {
        let tone = if tag.eq_ignore_ascii_case("Work") || tag.eq_ignore_ascii_case(&group) {
            ChipTone::Orange
        } else if tag.eq_ignore_ascii_case("2FA") {
            ChipTone::Green
        } else {
            ChipTone::Blue
        };
        chips_row = chips_row.child(chip(tag.clone(), tone));
    }

    let header = div()
        .flex_shrink_0()
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
                        .flex_shrink_0()
                        .child(fav_letter),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w(px(0.))
                        .gap_1()
                        .child(
                            div()
                                .truncate()
                                .text_lg()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(title),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(palette::TEXT_MUTED)
                                .child(match updated {
                                    Some(updated) => {
                                        format!("Updated {updated} · in {group}")
                                    }
                                    None => format!("in {group}"),
                                }),
                        )
                        .when(!tags.is_empty(), |this| this.child(chips_row)),
                )
                .when(starred, |this| {
                    this.child(
                        gpui_component::Icon::from(gpui_component::IconName::StarFill)
                            .with_size(gpui_component::Size::Size(px(16.)))
                            .text_color(palette::ORANGE),
                    )
                }),
        );

    let mut body_col = v_flex()
        .id("entry-detail-scroll")
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .overflow_y_scroll()
        .gap_3p5()
        .p_5()
        .child(detail_row("Username", value_or_dash(&username), true, false))
        .child(if has_password {
            detail_row("Password", String::new(), true, true)
        } else {
            detail_row("Password", "Not set".to_string(), false, false)
        })
        .child(detail_row("URL", value_or_dash(&url), false, false));

    if has_otp {
        body_col = body_col.child(detail_row("TOTP", "—".to_string(), true, false));
    }

    body_col = body_col
        .child(
            v_flex()
                .gap_1()
                .child(label("Notes"))
                .child(
                    div()
                        .min_h(px(54.))
                        .p_3()
                        .rounded(px(6.))
                        .bg(palette::PANEL)
                        .border_1()
                        .border_color(palette::BORDER)
                        .text_xs()
                        .text_color(if notes.is_empty() {
                            palette::TEXT_FAINT
                        } else {
                            palette::TEXT
                        })
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
                .child(if has_password {
                    strength_card(strength, length).into_any_element()
                } else {
                    div()
                        .p_3()
                        .rounded(px(6.))
                        .bg(palette::PANEL)
                        .border_1()
                        .border_color(palette::BORDER)
                        .text_xs()
                        .text_color(palette::TEXT_FAINT)
                        .child("No password stored for this entry.")
                        .into_any_element()
                }),
        );

    let username_present = !entry.username.is_empty();
    let url_present = !entry.url.is_empty();

    let footer = h_flex()
        .flex_shrink_0()
        .gap_2()
        .p_3()
        .border_t_1()
        .border_color(palette::BORDER)
        .child(action_button(
            "detail-copy-password",
            "Copy password",
            AppIcon::Key,
            ActionStyle::Primary,
            has_password,
            true,
            CopyValueKind::Password,
            state_entity.clone(),
            cx,
        ))
        .child(action_button(
            "detail-copy-username",
            "User",
            AppIcon::Note,
            ActionStyle::Default,
            username_present,
            false,
            CopyValueKind::Username,
            state_entity.clone(),
            cx,
        ))
        .child(action_button(
            "detail-copy-url",
            "URL",
            AppIcon::Cloud,
            ActionStyle::Default,
            url_present,
            false,
            CopyValueKind::Url,
            state_entity.clone(),
            cx,
        ));

    v_flex()
        .h_full()
        .min_h(px(0.))
        .min_w(px(0.))
        .overflow_hidden()
        .child(header)
        .child(body_col)
        .child(footer)
}

fn value_or_dash(value: &str) -> String {
    if value.is_empty() {
        "—".to_string()
    } else {
        value.to_string()
    }
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

use gpui::{
    AnyElement, AppContext as _, ClickEvent, Context, Hsla, InteractiveElement as _,
    IntoElement as _, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _,
    StyledImage as _, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Sizable as _, h_flex,
    input::{Input, InputState},
    v_flex,
};

use crate::app::{
    AppState, CopyValueKind, SaveStatus, VaultBrowserModel, VaultStatus, VaultSummary,
    actions::{LockVault, NewEntry, OpenSyncSettings, OpenVault, OpenVaultSwitcher, SyncNow},
};
use crate::domain::{FaviconImage, VaultEntry, VaultGroup, VaultSnapshot};
use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::atoms::{ChipTone, chip, dot, label, section_heading, status_badge};
use crate::ui::widgets::brand::brand;
use crate::ui::widgets::entry_chrome::favicon;
use crate::ui::widgets::password::strength_card;

pub fn render(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let state = shell.state().read(cx);
    let summary = state.summary();
    let save_status = state.save_status().clone();
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
        workspace(&summary, save_status, browser, shell, cx, is_busy, is_open).into_any_element();

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
    // Header chip text. `provider` is None when the vault is local-only;
    // `synced_at` is None in that case too. Fall back to a neutral "Local"
    // / "—" pair rather than showing stale OneDrive copy.
    let provider = summary.provider.clone().unwrap_or_else(|| "Local".into());
    let synced_at = summary.synced_at.clone().unwrap_or_else(|| "—".into());
    let entry_count = summary.entries;
    // One walk over the tree per render instead of two; library_counts
    // tallies starred + has_otp inline without allocating the
    // `Vec<&VaultEntry>` that `entries_starred()`/`entries_with_otp()`
    // would have produced just to take `.len()`.
    let counts = snapshot
        .map(VaultSnapshot::library_counts)
        .unwrap_or_default();
    let starred_count = counts.starred;
    let twofa_count = counts.with_otp;

    // Borrow root + recycle-bin-id directly off the snapshot — `snapshot`
    // is held alive by the caller's `Arc<VaultSnapshot>`, so there's no
    // need to deep-clone the entire group tree (3 500 entries × ~200 B
    // per `VaultEntry` adds up fast on every frame).
    let root_group = snapshot.map(|s| &s.root);
    let recycle_bin_id = snapshot.and_then(|s| s.recycle_bin_id.as_deref());

    v_flex()
        .w(px(220.))
        .flex_shrink_0()
        .h_full()
        .bg(palette::sidebar())
        .border_r_1()
        .border_color(palette::border())
        .child(
            h_flex()
                .id("sidebar-vault-header")
                .gap_2()
                .items_center()
                .px_3p5()
                .pt_3()
                .pb_2()
                .hover(|s| s.bg(palette::panel()))
                .on_click(cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                    window.dispatch_action(Box::new(OpenVaultSwitcher), cx);
                }))
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
                                .text_color(palette::text())
                                .child(summary.title.clone()),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .items_center()
                                .text_xs()
                                .text_color(palette::green())
                                .child(dot(palette::green(), 6.0))
                                .child(if summary.is_open { "Synced" } else { "Locked" }),
                        ),
                )
                .child(
                    gpui_component::Icon::from(gpui_component::IconName::ChevronDown)
                        .with_size(gpui_component::Size::Size(px(12.)))
                        .text_color(palette::text_muted()),
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
                .child(groups_section(
                    root_group,
                    recycle_bin_id,
                    selection,
                    state_entity.clone(),
                    cx,
                ))
                .child(tags_section(
                    twofa_count,
                    selection,
                    state_entity.clone(),
                    cx,
                )),
        )
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(palette::border())
                .text_xs()
                .text_color(palette::text_muted())
                .child(
                    gpui_component::Icon::from(AppIcon::Cloud)
                        .with_size(gpui_component::Size::Size(px(13.)))
                        .text_color(palette::blue()),
                )
                .child(div().flex_1().child(format!("{provider} · {synced_at}")))
                .child(
                    div()
                        .id("sidebar-settings")
                        .child(
                            gpui_component::Icon::from(gpui_component::IconName::Settings)
                                .with_size(gpui_component::Size::Size(px(13.)))
                                .text_color(palette::text_muted()),
                        )
                        .on_click(cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                            window.dispatch_action(Box::new(OpenSyncSettings), cx);
                        })),
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
            palette::blue(),
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
            palette::orange(),
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
            palette::text_muted(),
            state_entity.clone(),
            L::RecentlyUsed,
            cx,
        ))
        .child({
            // Trash gets the same drop wiring as group rows, but the
            // drop calls `delete_entry` (which lazily creates the
            // recycle bin if missing) — same semantics as the
            // explicit Delete button. Build the drop listener BEFORE
            // calling nav_row so the two cx borrows don't overlap (in
            // edition 2024 the `impl IntoElement` return captures cx).
            let state_for_drop = state_entity.clone();
            let drop_listener = cx.listener(move |_: &mut AppShell, drag: &EntryDrag, _, cx| {
                let entry_id = drag.entry_id.clone();
                state_for_drop.update(cx, |state, cx| {
                    let _ = state.delete_entry(&entry_id, cx);
                });
            });
            let trash_row = nav_row(
                "lib-trash",
                AppIcon::Note,
                "Trash",
                None,
                selection.is_trash(),
                palette::text_muted(),
                state_entity,
                L::Trash,
                cx,
            );
            div()
                .id("lib-trash-drop")
                .rounded(px(6.))
                .drag_over::<EntryDrag>(|this, _, _, _| {
                    this.bg(palette::orange_soft())
                        .border_1()
                        .border_color(palette::orange())
                })
                .on_drop(drop_listener)
                .child(trash_row)
        })
}

fn groups_section(
    root: Option<&VaultGroup>,
    recycle_bin_id: Option<&str>,
    selection: &crate::app::LibrarySelection,
    state_entity: gpui::Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    // Flatten the tree to a depth-tagged preorder list once, starting at
    // the database root so the root group is visible — KeePassXC does
    // the same, and entries that live directly at root would otherwise
    // be unreachable via the group nav (only via "All items"). Drops on
    // the root row move an entry back to top level. We deliberately
    // render the whole tree (no expand/collapse yet) because users
    // overwhelmingly want their structure visible at a glance;
    // collapsibility is a later polish.
    //
    // The recycle-bin group is dropped from the tree because it has its
    // own dedicated "Trash" affordance under the Library section —
    // surfacing it as a regular group here would just confuse the user
    // about where deleted entries live.
    let mut flat: Vec<(usize, &VaultGroup)> = Vec::new();
    fn collect<'a>(
        group: &'a VaultGroup,
        depth: usize,
        recycle_bin_id: Option<&str>,
        out: &mut Vec<(usize, &'a VaultGroup)>,
    ) {
        if recycle_bin_id == Some(group.id.as_str()) {
            return;
        }
        out.push((depth, group));
        // Skip children when the parent is collapsed. The flag lives on
        // the KeePass `Group::is_expanded` field, so collapse state is
        // persisted with the database and survives restarts and
        // round-trips through other clients.
        if !group.is_expanded {
            return;
        }
        for child in &group.groups {
            collect(child, depth + 1, recycle_bin_id, out);
        }
    }
    if let Some(root) = root {
        collect(root, 0, recycle_bin_id, &mut flat);
    }

    let palette_colors = [
        palette::blue(),
        palette::orange(),
        palette::green(),
        palette::text_muted(),
    ];
    let selected_group = selection.group_id().unwrap_or_default().to_string();

    let mut col = v_flex()
        .gap_0p5()
        .pb_2()
        .child(div().px_3p5().pb_1().child(section_heading("Groups")));

    for (i, (depth, group)) in flat.iter().enumerate() {
        let depth = *depth;
        let color = palette_colors[i % palette_colors.len()];
        let is_selected = group.id == selected_group;
        let group_id = group.id.clone();
        let count = group.entry_count();
        let has_children = !group.groups.is_empty();
        let is_expanded = group.is_expanded;
        let state_for_click = state_entity.clone();
        let group_id_for_drop = group.id.clone();
        let state_for_drop = state_entity.clone();
        let group_id_for_toggle = group.id.clone();
        let state_for_toggle = state_entity.clone();

        // Layout per row: [chevron column | nav_pill (flex_1)]. The
        // chevron and the pill are *siblings*, not nested — clicks on
        // the chevron toggle expansion without bubbling into the row's
        // select handler, matching the codebase pattern noted in
        // `password_row` (see "we don't have to manage stop_propagation").
        //
        // The chevron column is rendered even for leaf groups so all
        // rows align vertically; leaves just don't get an icon or a
        // listener.
        //
        // Depth-based left padding gives the tree shape. 12 px per
        // level lines up with the icon column and produces a readable
        // indent without eating the 220 px sidebar width even for
        // moderately deep trees (4–5 levels still fit).
        let chevron_id = gpui::SharedString::from(format!("group-chev-{}", group.id));
        let chevron = if has_children {
            div()
                .id(chevron_id)
                .w(px(16.))
                .h(px(16.))
                .flex()
                .items_center()
                .justify_center()
                .text_color(palette::text_muted())
                .hover(|s| s.text_color(palette::text()))
                .child(
                    gpui_component::Icon::from(if is_expanded {
                        gpui_component::IconName::ChevronDown
                    } else {
                        gpui_component::IconName::ChevronRight
                    })
                    .with_size(gpui_component::Size::Size(px(11.))),
                )
                .on_click(cx.listener(move |_: &mut AppShell, _: &ClickEvent, _, cx| {
                    let id = group_id_for_toggle.clone();
                    state_for_toggle.update(cx, |state, cx| state.toggle_group_expanded(&id, cx));
                }))
                .into_any_element()
        } else {
            div().w(px(16.)).h(px(16.)).into_any_element()
        };

        col = col.child(
            h_flex()
                .id(gpui::SharedString::from(format!("group-drop-{}", group.id)))
                .pl(px(depth as f32 * 12.))
                .items_center()
                .rounded(px(6.))
                .drag_over::<EntryDrag>(|this, _, _, _| {
                    this.bg(palette::blue_soft())
                        .border_1()
                        .border_color(palette::blue())
                })
                .on_drop(
                    cx.listener(move |_: &mut AppShell, drag: &EntryDrag, _, cx| {
                        let target = group_id_for_drop.clone();
                        let entry_id = drag.entry_id.clone();
                        state_for_drop.update(cx, |state, cx| {
                            let _ = state.move_entry(&entry_id, &target, cx);
                        });
                    }),
                )
                .child(chevron)
                .child(div().flex_1().min_w_0().child(nav_pill(
                    gpui::SharedString::from(format!("group-{}", group.id)),
                    AppIcon::Note,
                    group.icon.as_ref(),
                    &group.name,
                    Some(count),
                    is_selected,
                    color,
                    cx.listener(move |_: &mut AppShell, _: &ClickEvent, _, cx| {
                        let id = group_id.clone();
                        state_for_click.update(cx, |state, cx| {
                            state.select_group(id, cx);
                        });
                    }),
                ))),
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
            selection.is_totp_enabled(),
            palette::blue(),
            state_entity.clone(),
            L::TotpEnabled,
            cx,
        ))
        .child(nav_row(
            "tag-personal",
            AppIcon::Dot,
            "Personal",
            None,
            selected_tag.eq_ignore_ascii_case("Personal"),
            palette::green(),
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
            palette::yellow(),
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
    nav_pill(
        gpui::SharedString::from(id),
        icon,
        None,
        label_text,
        count,
        selected,
        icon_color,
        cx.listener(move |_: &mut AppShell, _: &ClickEvent, _, cx| {
            let target = target.clone();
            state_entity.update(cx, |state, cx| {
                state.select_library(target, cx);
            });
        }),
    )
}

/// Stateful nav row with hover state baked in. The element owns its `id`,
/// `on_click` handler, and `hover` style so we don't need a separate wrapper
/// (which would either swallow the inner background or create a click-vs-hover
/// region mismatch).
// `icon_image`, when Some, replaces the `AppIcon` glyph with a custom-icon
// image — used by group rows so KeePass `Icon::Custom(_)` shows the user's
// own bitmap instead of the generic note icon. Tinting doesn't apply to
// images (mirrors KeePassXC).
#[allow(clippy::too_many_arguments)]
fn nav_pill(
    id: gpui::SharedString,
    icon: AppIcon,
    icon_image: Option<&FaviconImage>,
    label_text: &str,
    count: Option<usize>,
    selected: bool,
    icon_color: Hsla,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl gpui::IntoElement {
    let fg = if selected {
        palette::panel()
    } else {
        palette::text()
    };
    let count_color = if selected {
        palette::panel()
    } else {
        palette::text_faint()
    };
    let icon_resolved = if selected {
        palette::panel()
    } else {
        icon_color
    };
    let label_owned = label_text.to_string();

    h_flex()
        .id(id)
        .gap_2()
        .items_center()
        .h(px(26.))
        .mx(px(6.))
        .px_3()
        .rounded(px(5.))
        // Selected: solid blue. Unselected: transparent so the sidebar shows through,
        // letting the hover overlay (BORDER tone) produce a visible state change.
        .bg(if selected {
            palette::blue()
        } else {
            gpui::transparent_black()
        })
        .text_color(fg)
        .text_sm()
        .font_weight(if selected {
            gpui::FontWeight::MEDIUM
        } else {
            gpui::FontWeight::NORMAL
        })
        .when(!selected, |this| this.hover(|s| s.bg(palette::border())))
        .on_click(on_click)
        .child(nav_pill_icon(icon, icon_image, icon_resolved))
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

/// Render the leading 13×13 icon slot for a nav pill. Custom-icon image
/// (when present) takes priority over the `AppIcon` fallback. The image
/// gets `with_fallback` so a corrupt blob falls back to the glyph
/// instead of an empty slot — same defensive treatment as the entry
/// favicon path in `entry_chrome::favicon`.
fn nav_pill_icon(icon: AppIcon, icon_image: Option<&FaviconImage>, icon_color: Hsla) -> AnyElement {
    if let Some(image) = icon_image {
        return div()
            .id("nav-pill-icon")
            .size(px(13.))
            .rounded(px(3.))
            .overflow_hidden()
            .child(
                gpui::img(image.0.clone())
                    .object_fit(gpui::ObjectFit::Cover)
                    .size(px(13.))
                    .with_fallback(move || {
                        gpui_component::Icon::from(icon)
                            .with_size(gpui_component::Size::Size(px(13.)))
                            .text_color(icon_color)
                            .into_any_element()
                    }),
            )
            .into_any_element();
    }
    gpui_component::Icon::from(icon)
        .with_size(gpui_component::Size::Size(px(13.)))
        .text_color(icon_color)
        .into_any_element()
}

fn workspace(
    summary: &VaultSummary,
    save_status: SaveStatus,
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

    let status = status_bar(summary, &save_status, cx).into_any_element();

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
        .border_color(palette::border())
        .bg(palette::panel())
        .child(
            div()
                .id("toolbar-new-entry")
                .child(toolbar_button("New entry", Some(AppIcon::Key), true))
                .on_click(cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                    window.dispatch_action(Box::new(NewEntry), cx);
                })),
        )
        .child(div().w(px(1.)).h(px(18.)).bg(palette::border()))
        .child(
            div()
                .id("toolbar-group")
                .child(toolbar_button("Group", Some(AppIcon::Note), false)),
        )
        .child(
            div()
                .id("toolbar-sync")
                .child(toolbar_button("Sync", Some(AppIcon::Sync), false))
                .on_click(cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                    window.dispatch_action(Box::new(SyncNow), cx);
                })),
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
                    Some(if is_open {
                        AppIcon::Lock
                    } else {
                        AppIcon::Unlock
                    }),
                    is_open,
                ))
                .on_click(
                    cx.listener(move |_: &mut AppShell, _: &ClickEvent, window, cx| {
                        if is_open {
                            window.dispatch_action(Box::new(LockVault), cx);
                        } else {
                            window.dispatch_action(Box::new(OpenVault), cx);
                        }
                    }),
                ),
        )
}

fn toolbar_button(
    text: &'static str,
    icon: Option<AppIcon>,
    primary: bool,
) -> impl gpui::IntoElement {
    let (bg, fg, bd) = if primary {
        (palette::blue(), palette::panel(), palette::blue_hover())
    } else {
        (palette::panel(), palette::text(), palette::border_strong())
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

#[derive(Clone, Copy)]
enum FooterStyle {
    Default,
    Danger,
}

/// Compact footer button (Edit / Delete / Restore / Confirm forever / Cancel)
/// — same dimensions as `action_button`'s default style but with an
/// `on_click` handler injected directly so the call site can control what
/// happens (no copy-to-clipboard wiring like `action_button`).
fn footer_button(
    id: &'static str,
    text: &'static str,
    style: FooterStyle,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let (bg, fg, bd) = match style {
        FooterStyle::Default => (palette::panel(), palette::text(), palette::border_strong()),
        FooterStyle::Danger => (palette::red(), palette::panel(), palette::red()),
    };
    div()
        .id(id)
        .h(px(28.))
        .px(px(12.))
        .rounded(px(6.))
        .bg(bg)
        .border_1()
        .border_color(bd)
        .text_color(fg)
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .flex()
        .items_center()
        .justify_center()
        .child(text)
        .on_click(on_click)
        .into_any_element()
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
        (ActionStyle::Primary, true) => (palette::blue(), palette::panel(), palette::blue_hover()),
        (ActionStyle::Primary, false) => (
            palette::sidebar(),
            palette::text_faint(),
            palette::border_strong(),
        ),
        (ActionStyle::Default, true) => {
            (palette::panel(), palette::text(), palette::border_strong())
        }
        (ActionStyle::Default, false) => {
            (palette::sidebar(), palette::text_faint(), palette::border())
        }
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
        let _ = state_entity; // routed through AppShell now; keeping the
        // parameter avoids churn at the four call sites below.
        this.on_click(
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, window, cx| {
                shell.copy_selected_value(kind, window, cx);
            }),
        )
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
        .child(entry_detail(
            selected_entry.clone(),
            browser.selected_strength,
            shell.pending_perma_delete().map(|s| s.to_string()),
            // When the user has the password reveal toggled, fetch the
            // decrypted value so the detail row can render it. Otherwise
            // pass `None` and the row stays masked.
            selected_entry.as_ref().and_then(|e| {
                if shell.is_password_revealed(&e.id) {
                    shell
                        .state()
                        .read(cx)
                        .copy_selected_value(crate::app::CopyValueKind::Password)
                } else {
                    None
                }
            }),
            shell.state().clone(),
            cx,
        ))
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
            label: if showing_search {
                "Results"
            } else {
                "All entries"
            },
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
            .text_color(palette::text_muted())
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
                                let is_selected = selected_id.as_deref() == Some(entry.id.as_str());
                                entry_row(entry.clone(), is_selected, state_entity.clone(), cx)
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
        .border_color(palette::border())
        .bg(palette::panel())
        .child(
            v_flex()
                .flex_shrink_0()
                .gap_2()
                .p_4()
                .border_b_1()
                .border_color(palette::border())
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
                                .text_color(palette::text())
                                .child(group_name.to_string()),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(palette::text_muted())
                                .child(format!("{} entries", total)),
                        ),
                )
                .when(showing_search, |this| {
                    this.child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(palette::text_muted())
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
        .text_color(palette::text_faint())
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
    let fav = entry.favicon.clone();

    let bg = if selected {
        palette::blue_soft()
    } else {
        palette::panel()
    };
    let border = if selected {
        palette::blue_border()
    } else {
        palette::panel()
    };

    let drag_payload = EntryDrag {
        entry_id: entry_id.clone(),
        title: if title.trim().is_empty() {
            "(untitled)".to_string()
        } else {
            title.clone()
        },
    };

    h_flex()
        .id(gpui::SharedString::from(format!("entry-{entry_id}")))
        .on_click(cx.listener(move |_: &mut AppShell, _: &ClickEvent, _, cx| {
            let id = entry_id.clone();
            state_entity.update(cx, |state, cx| state.select_entry(id, cx));
        }))
        // Drag-source. The closure builds a fresh preview entity
        // anchored at the cursor; GPUI handles offset + repaint.
        .on_drag(drag_payload, |drag, _offset, _window, cx| {
            let title = drag.title.clone();
            cx.new(|_| EntryDragPreview {
                title: title.into(),
            })
        })
        // `w_full` is essential here: without it the entry row sizes to its
        // *content* (favicon + title + tags) and the tag/time column ends up
        // snuggled against the title instead of pinned to the panel edge.
        // The virtual list passes us a `Definite` available width but a Div
        // without explicit width defaults to `auto` (content size).
        .w_full()
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
        .when(!selected, |this| {
            this.hover(|s| s.bg(palette::sidebar()).border_color(palette::border()))
        })
        .child(favicon(&fav, 28.))
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
                                .text_color(palette::text())
                                .child(title),
                        )
                        .when(starred, |this| {
                            this.child(
                                gpui_component::Icon::from(gpui_component::IconName::StarFill)
                                    .with_size(gpui_component::Size::Size(px(11.)))
                                    .text_color(palette::orange()),
                            )
                        }),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(palette::text_muted())
                        .font_family("JetBrains Mono")
                        .child(if username.is_empty() {
                            if url.is_empty() {
                                "—".to_string()
                            } else {
                                url
                            }
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
                        .text_color(palette::text_faint())
                        .child(updated),
                ),
        )
}

fn entry_detail(
    selected: Option<VaultEntry>,
    selected_strength: Option<crate::keepass::StrengthReport>,
    pending_perma_delete: Option<String>,
    revealed_password: Option<String>,
    state_entity: gpui::Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    let body: AnyElement = match selected {
        Some(entry) => entry_detail_body(
            entry,
            selected_strength,
            pending_perma_delete,
            revealed_password,
            state_entity,
            cx,
        )
        .into_any_element(),
        None => v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_1()
            .text_color(palette::text_muted())
            .child(
                div()
                    .text_base()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Select an entry"),
            )
            .child(div().text_sm().child("Entry details will appear here."))
            .into_any_element(),
    };

    v_flex()
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .h_full()
        .overflow_hidden()
        .bg(palette::sidebar())
        .border_l_1()
        .border_color(palette::border())
        .child(body)
}

fn entry_detail_body(
    entry: VaultEntry,
    selected_strength: Option<crate::keepass::StrengthReport>,
    pending_perma_delete: Option<String>,
    revealed_password: Option<String>,
    state_entity: gpui::Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    let title = entry.title.clone();
    let username = entry.username.clone();
    let url = entry.url.clone();
    let notes = entry.notes.clone();
    // Prefer the real zxcvbn report; fall back to the synthesized snapshot value
    // (length-based) when the entry has no decryptable password.
    let (strength, length, bits) = match selected_strength {
        Some(report) => (report.strength, report.length, Some(report.bits)),
        None => (entry.strength, entry.password_length, None),
    };
    let group = entry
        .group_path
        .last()
        .cloned()
        .unwrap_or_else(|| "Vault root".into());
    let updated = entry.updated.clone();
    let starred = entry.starred;
    let entry_id_for_star = entry.id.clone();
    let state_for_star = state_entity.clone();
    let fav = entry.favicon.clone();
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
        .border_color(palette::border())
        .child(
            h_flex()
                .gap_3()
                .items_start()
                .child(div().flex_shrink_0().child(favicon(&fav, 44.)))
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
                                .text_color(palette::text_muted())
                                .child(match updated {
                                    Some(updated) => {
                                        format!("Updated {updated} · in {group}")
                                    }
                                    None => format!("in {group}"),
                                }),
                        )
                        .when(!tags.is_empty(), |this| this.child(chips_row)),
                )
                .child(
                    div()
                        .id("entry-detail-star")
                        .flex_shrink_0()
                        .p_1p5()
                        .rounded(px(6.))
                        .hover(|s| s.bg(palette::panel()))
                        .on_click(cx.listener(move |_: &mut AppShell, _: &ClickEvent, _, cx| {
                            let id = entry_id_for_star.clone();
                            state_for_star.update(cx, |state, cx| {
                                let _ = state.toggle_starred(&id, cx);
                            });
                        }))
                        .child(
                            gpui_component::Icon::from(if starred {
                                gpui_component::IconName::StarFill
                            } else {
                                gpui_component::IconName::Star
                            })
                            .with_size(gpui_component::Size::Size(px(16.)))
                            .text_color(if starred {
                                palette::orange()
                            } else {
                                palette::text_faint()
                            }),
                        ),
                ),
        );

    let username_for_row = username.clone();
    let url_for_row = url.clone();
    let entry_id_for_password = entry.id.clone();
    let entry_id_for_reveal = entry.id.clone();

    let mut body_col = v_flex()
        .id("entry-detail-scroll")
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .overflow_y_scroll()
        .gap_3p5()
        .p_5()
        .child(clickable_field_row(
            "detail-row-username",
            "Username",
            value_or_dash(&username_for_row),
            true,
            !username.is_empty(),
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, window, cx| {
                shell.copy_selected_value(crate::app::CopyValueKind::Username, window, cx);
            }),
        ))
        .child(password_row(
            has_password,
            revealed_password.clone(),
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, window, cx| {
                let _ = entry_id_for_password;
                shell.copy_selected_value(crate::app::CopyValueKind::Password, window, cx);
            }),
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                shell.toggle_password_reveal(entry_id_for_reveal.clone(), cx);
            }),
        ))
        .child(clickable_field_row(
            "detail-row-url",
            "URL",
            value_or_dash(&url_for_row),
            false,
            !url.is_empty(),
            cx.listener(move |_: &mut AppShell, _: &ClickEvent, _, cx| {
                cx.open_url(&ensure_scheme(&url_for_row));
            }),
        ));

    if has_otp {
        // Pull the live code each render — the AppShell tick fires `cx.notify`
        // on AppState every second, which causes this re-render with a fresh
        // value + countdown. Read once to avoid borrowing state twice.
        let otp = state_entity.read(cx).totp_for_selected_entry();
        let label_text = match &otp {
            Some(o) => format!("TOTP · {}s", o.remaining_secs),
            None => "TOTP".to_string(),
        };
        let display = otp
            .as_ref()
            .map(|o| o.code.clone())
            .unwrap_or_else(|| "—".to_string());

        // Warn the user when the code is about to rotate. The thresholds
        // mirror KeePassXC: <=5s = orange (about to expire), then back to
        // neutral once a fresh code lands. The 30s window is short enough
        // that visual warning is more reliable than reading the countdown.
        let warning = otp.as_ref().is_some_and(|o| o.remaining_secs <= 5);
        let (border_color, text_color) = if warning {
            (palette::orange(), palette::orange_deep())
        } else {
            (palette::border(), palette::text())
        };

        // Raw digits (without the thin space we insert for readability) for
        // clipboard. Closure captures by move into the click handler.
        let raw_for_clipboard = otp
            .as_ref()
            .map(|o| o.code.replace(' ', ""))
            .unwrap_or_default();
        let copyable = !raw_for_clipboard.is_empty();

        body_col = body_col.child(
            v_flex()
                .gap_1()
                .min_w(px(0.))
                .child(label(label_text))
                .child({
                    // Single-line mono code box. Click → copy raw digits +
                    // toast. Cursor stays the default pointer-style; if we
                    // want a hand cursor later we can wire `.cursor_pointer()`
                    // on the stateful div.
                    let mut row = div()
                        .id("detail-totp-code")
                        .h(px(34.))
                        .w_full()
                        .min_w(px(0.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(border_color)
                        .bg(palette::sidebar())
                        .px_3()
                        .py_2()
                        .text_sm()
                        .text_color(text_color)
                        .truncate()
                        .font_family("JetBrains Mono")
                        .child(display);
                    if copyable {
                        row = row.on_click(cx.listener(
                            move |shell: &mut AppShell, _: &ClickEvent, window, cx| {
                                shell.copy_with_auto_clear(
                                    raw_for_clipboard.clone(),
                                    "TOTP",
                                    window,
                                    cx,
                                );
                            },
                        ));
                    }
                    row
                }),
        );
    }

    body_col = body_col
        .child(
            v_flex().gap_1().child(label("Notes")).child(
                div()
                    .min_h(px(54.))
                    .p_3()
                    .rounded(px(6.))
                    .bg(palette::panel())
                    .border_1()
                    .border_color(palette::border())
                    .text_xs()
                    .text_color(if notes.is_empty() {
                        palette::text_faint()
                    } else {
                        palette::text()
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
                    strength_card(strength, length, bits).into_any_element()
                } else {
                    div()
                        .p_3()
                        .rounded(px(6.))
                        .bg(palette::panel())
                        .border_1()
                        .border_color(palette::border())
                        .text_xs()
                        .text_color(palette::text_faint())
                        .child("No password stored for this entry.")
                        .into_any_element()
                }),
        );

    let username_present = !entry.username.is_empty();
    let url_present = !entry.url.is_empty();

    let in_trash = entry.in_recycle_bin;
    let entry_id_for_actions = entry.id.clone();
    let perma_armed = pending_perma_delete.as_deref() == Some(entry.id.as_str());

    // When the user has armed "Delete forever", we replace the entire footer
    // with a destructive confirmation strip — same height as the normal
    // footer, but only Cancel + the destructive primary remain. Hiding the
    // copy/restore actions has two benefits:
    //   1. Removes any chance of accidentally clicking the wrong button mid-
    //      confirmation.
    //   2. Avoids the 6-button layout that overflowed the panel and clipped
    //      the trailing buttons on narrow widths.
    let footer = if perma_armed {
        let confirm_id = entry_id_for_actions.clone();
        h_flex()
            .flex_shrink_0()
            .gap_2()
            .p_3()
            .items_center()
            .border_t_1()
            .border_color(palette::border())
            .bg(palette::orange_soft())
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .text_xs()
                    .text_color(palette::text())
                    .child("Permanently delete this entry?"),
            )
            .child(footer_button(
                "detail-perma-cancel",
                "Cancel",
                FooterStyle::Default,
                cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
                    shell.clear_perma_delete(cx);
                }),
            ))
            .child(footer_button(
                "detail-perma-confirm",
                "Delete forever",
                FooterStyle::Danger,
                cx.listener(move |shell: &mut AppShell, _: &ClickEvent, window, cx| {
                    shell.confirm_perma_delete(confirm_id.clone(), window, cx);
                }),
            ))
    } else {
        // Normal footer: trailing pair depends on trash vs. live entry.
        let trailing_primary = if in_trash {
            footer_button(
                "detail-restore-entry",
                "Restore",
                FooterStyle::Default,
                cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
                    shell.restore_selected_entry(window, cx);
                }),
            )
        } else {
            footer_button(
                "detail-edit-entry",
                "Edit",
                FooterStyle::Default,
                cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
                    shell.begin_edit_selected_entry(window, cx);
                }),
            )
        };
        let trailing_secondary = if in_trash {
            let arm_id = entry_id_for_actions.clone();
            footer_button(
                "detail-perma-arm",
                "Delete forever",
                FooterStyle::Danger,
                cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                    shell.arm_perma_delete(arm_id.clone(), cx);
                }),
            )
        } else {
            footer_button(
                "detail-delete-entry",
                "Delete",
                FooterStyle::Default,
                cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
                    shell.delete_selected_entry(window, cx);
                }),
            )
        };

        h_flex()
            .flex_shrink_0()
            .gap_2()
            .p_3()
            .border_t_1()
            .border_color(palette::border())
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
            ))
            .child(trailing_primary)
            .child(trailing_secondary)
    };

    v_flex()
        .h_full()
        .min_h(px(0.))
        .min_w(px(0.))
        .overflow_hidden()
        .child(header)
        .child(body_col)
        .child(footer)
}

/// Click-to-act detail row used for username (copy) and URL (open in
/// browser). Visual shape matches the old `detail_row` widget so swapping
/// between row kinds in `entry_detail_body` doesn't shift the layout.
/// `enabled = false` paints the row in a muted style and skips the
/// click handler — used when the field is empty (`"—"`).
fn clickable_field_row<F>(
    id: &'static str,
    label_text: &'static str,
    display: String,
    mono: bool,
    enabled: bool,
    on_click: F,
) -> AnyElement
where
    F: Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
{
    let inner = div()
        .h(px(34.))
        .w_full()
        .min_w(px(0.))
        .rounded(px(6.))
        .border_1()
        .border_color(palette::border())
        .bg(palette::sidebar())
        .px_3()
        .py_2()
        .text_sm()
        .text_color(if enabled {
            palette::text()
        } else {
            palette::text_faint()
        })
        .truncate()
        .when(mono, |this| this.font_family("JetBrains Mono"))
        .child(display);

    let mut row = div().id(id).child(label_widget(label_text));
    if enabled {
        row = row.child(
            div()
                .id(gpui::SharedString::from(format!("{id}-inner")))
                .cursor_pointer()
                .hover(|s| s.bg(palette::panel()))
                .rounded(px(6.))
                .on_click(on_click)
                .child(inner),
        );
    } else {
        row = row.child(inner);
    }
    v_flex().gap_1().min_w(px(0.)).child(row).into_any_element()
}

/// Password detail row: masked or revealed value with a click-to-copy
/// area on the left and an eye-icon reveal toggle on the right. Click
/// targets are siblings (not nested), so neither bubbles into the
/// other — we don't have to manage `stop_propagation`.
fn password_row<F1, F2>(
    has_password: bool,
    revealed_value: Option<String>,
    on_click_copy: F1,
    on_click_reveal: F2,
) -> AnyElement
where
    F1: Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
    F2: Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
{
    if !has_password {
        // Mirror the old "Not set" presentation — non-clickable, faint.
        return clickable_field_row(
            "detail-row-password",
            "Password",
            "Not set".to_string(),
            false,
            false,
            |_, _, _| {},
        );
    }

    let revealed = revealed_value.is_some();
    let display = revealed_value.unwrap_or_else(|| "••••••••••••••••".to_string());

    let value_box = div()
        .id("detail-row-password-value")
        .flex_1()
        .min_w(px(0.))
        .h(px(34.))
        .rounded(px(6.))
        .border_1()
        .border_color(palette::border())
        .bg(palette::sidebar())
        .px_3()
        .py_2()
        .text_sm()
        .truncate()
        .font_family("JetBrains Mono")
        .cursor_pointer()
        .hover(|s| s.bg(palette::panel()))
        .on_click(on_click_copy)
        .child(display);

    let reveal_button = div()
        .id("detail-row-password-reveal")
        .flex_shrink_0()
        .h(px(34.))
        .w(px(34.))
        .rounded(px(6.))
        .border_1()
        .border_color(palette::border())
        .bg(palette::sidebar())
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.bg(palette::panel()))
        .on_click(on_click_reveal)
        .child(
            gpui_component::Icon::from(if revealed {
                gpui_component::IconName::EyeOff
            } else {
                gpui_component::IconName::Eye
            })
            .with_size(gpui_component::Size::Size(px(14.)))
            .text_color(if revealed {
                palette::blue()
            } else {
                palette::text_muted()
            }),
        );

    v_flex()
        .gap_1()
        .min_w(px(0.))
        .child(label_widget("Password"))
        .child(
            h_flex()
                .gap_2()
                .min_w(px(0.))
                .child(value_box)
                .child(reveal_button),
        )
        .into_any_element()
}

/// Inline copy of `widgets::password::label` (private over there) so
/// the click-row helpers can render the same caption style without
/// re-exporting it.
fn label_widget(text: &'static str) -> AnyElement {
    div()
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(palette::text_muted())
        .child(text)
        .into_any_element()
}

/// Prepend `https://` to bare URLs so `cx.open_url` doesn't fail on
/// `github.com`-style entries. Schemes already present (`http://`,
/// `https://`) are left untouched. Other schemes (`mailto:`, `ftp:`,
/// …) get rewritten — KeePass URL fields almost always hold web URLs,
/// and the user can always add the scheme explicitly if needed.
fn ensure_scheme(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    }
}

fn value_or_dash(value: &str) -> String {
    if value.is_empty() {
        "—".to_string()
    } else {
        value.to_string()
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
                .text_color(palette::text())
                .child(summary.title.clone()),
        )
        .child(
            div()
                .text_sm()
                .text_color(palette::text_muted())
                .child(summary.subtitle.clone()),
        )
        .child(
            div()
                .id("empty-open-vault")
                .child(toolbar_button("Open vault", Some(AppIcon::Unlock), true))
                .on_click(cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                    window.dispatch_action(Box::new(OpenVault), cx);
                })),
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
                .text_color(palette::text_muted())
                .child(summary.subtitle.clone()),
        )
}

fn status_bar(
    summary: &VaultSummary,
    save_status: &SaveStatus,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    h_flex()
        .h(px(24.))
        .flex_shrink_0()
        .gap_3()
        .items_center()
        .px_3()
        .border_t_1()
        .border_color(palette::border())
        .bg(palette::sidebar())
        .text_xs()
        .text_color(palette::text_muted())
        .font_family("JetBrains Mono")
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .child(dot(palette::green(), 6.0))
                .child(if summary.is_open {
                    "Unlocked"
                } else {
                    "Locked"
                }),
        )
        .child(format!(
            "{} entries · {} groups",
            summary.entries, summary.groups
        ))
        .child(div().flex_1())
        .child(save_status_pill(save_status, cx))
}

/// Right-aligned save indicator. Three visual modes:
///
/// * **Idle / Saved** → green dot + "Saved". The Idle and Saved cases collapse
///   visually because for the user they mean the same thing: the on-disk file
///   matches the in-memory state. Distinguishing them would just be noise.
/// * **Saving** → muted dot + "Saving…". Auto-save runs in the background, so
///   this state is usually visible only for the ~500 ms Argon2 KDF window.
/// * **Failed** → red dot + truncated error + clickable "Retry" chip. Retry
///   re-fires the same `SaveVault` action that `cmd-s` triggers.
fn save_status_pill(status: &SaveStatus, cx: &mut Context<AppShell>) -> impl gpui::IntoElement {
    match status {
        SaveStatus::Idle | SaveStatus::Saved => h_flex()
            .gap_1()
            .items_center()
            .child(dot(palette::green(), 6.0))
            .child("Saved")
            .into_any_element(),
        SaveStatus::Saving => h_flex()
            .gap_1()
            .items_center()
            .child(dot(palette::text_faint(), 6.0))
            .child("Saving…")
            .into_any_element(),
        SaveStatus::Failed(msg) => {
            // Truncate long error messages so they don't push the Retry pill
            // off-screen. The full text could go in a tooltip later.
            let short = if msg.chars().count() > 48 {
                let mut s: String = msg.chars().take(45).collect();
                s.push('…');
                s
            } else {
                msg.to_string()
            };
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(dot(palette::red(), 6.0))
                        .child(
                            div()
                                .text_color(palette::red())
                                .child(format!("Save failed: {short}")),
                        ),
                )
                .child(
                    div()
                        .id("status-save-retry")
                        .h(px(18.))
                        .px(px(8.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(palette::border_strong())
                        .bg(palette::panel())
                        .text_color(palette::text())
                        .flex()
                        .items_center()
                        .justify_center()
                        .child("Retry")
                        .on_click(cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                            window.dispatch_action(Box::new(crate::app::actions::SaveVault), cx);
                        })),
                )
                .into_any_element()
        }
    }
}

#[allow(dead_code)]
fn _status_badge_unused(text: &'static str) {
    let _ = status_badge(text, ChipTone::Green);
    let _ = label("noop");
}

// ============================================================
// Drag-and-drop: relocate entries between groups
// ============================================================

/// Payload that travels with an entry-row drag. Carries the id (for
/// the move) and the title (for the drag preview that follows the
/// cursor). Captured as `T` in `on_drag::<EntryDrag, _>(...)`.
#[derive(Clone)]
pub struct EntryDrag {
    pub entry_id: String,
    pub title: String,
}

/// Tiny floating chip rendered as the drag preview. GPUI requires the
/// preview to be a `Render` entity, so this gets `cx.new`-ed inside
/// the `on_drag` constructor.
pub struct EntryDragPreview {
    title: gpui::SharedString,
}

impl Render for EntryDragPreview {
    fn render(&mut self, _: &mut Window, _cx: &mut Context<Self>) -> impl gpui::IntoElement {
        h_flex()
            .h(px(28.))
            .px(px(10.))
            .rounded(px(6.))
            .bg(palette::panel())
            .border_1()
            .border_color(palette::blue_border())
            .text_xs()
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(palette::text())
            .items_center()
            .gap_1p5()
            .child(
                gpui_component::Icon::from(AppIcon::Note)
                    .with_size(gpui_component::Size::Size(px(11.)))
                    .text_color(palette::blue()),
            )
            .child(self.title.clone())
    }
}

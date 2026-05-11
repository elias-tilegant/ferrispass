//! Vault switcher overlay — a centred modal with a filter input, an
//! "Open" section listing every vault that's currently decrypted in
//! memory (active + parked), a "Recent" section for cold vaults, and a
//! "Browse other vault…" fallback row. Reachable via ⌘O from any vault
//! state and via clicking the sidebar header inside a vault.
//!
//! Click on an Open row → instant switch with no password prompt
//! (`AppShell::open_recent` consults `AppState::switch_to_unlocked`).
//! Click on a Recent row → routes through the normal cold-unlock flow.
//!
//! The filter narrows *both* sections in-place; Enter on the input
//! opens the topmost match (Open section wins when both have hits).

use std::path::{Path, PathBuf};

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
    px,
};
use gpui_component::{ActiveTheme as _, Sizable as _, h_flex, input::Input, v_flex};

use crate::app::RecentEntry;
use crate::app::actions::OpenVault;
use crate::app::time::relative_time_label;
use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;

pub fn render(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let bg = cx.theme().background;
    let state = shell.state().read(cx);
    let recents: Vec<RecentEntry> = state.recents().to_vec();
    let unlocked: Vec<PathBuf> = state.unlocked_paths();
    let active_path = state.current_vault_path();
    let query = shell.vault_switcher_input().read(cx).value().to_string();
    let needle = query.trim().to_lowercase();
    let now = chrono::Local::now();

    // Open section: every unlocked vault, filtered by the same needle as
    // recents. Order matches `unlocked_paths` — older parks first, active
    // last — so the row most likely to be wanted (the active one) sits
    // near the top of the visual hierarchy without us having to track a
    // separate "last switched" timestamp.
    let unlocked_filtered: Vec<PathBuf> = unlocked
        .iter()
        .filter(|p| matches_needle(p, &needle))
        .cloned()
        .collect();

    // Recent section: any recent path that isn't currently in the
    // unlocked set. De-dupe by path string match, not by canonicalisation
    // — same comparison rule `RecentEntry`'s PartialEq uses elsewhere.
    let recents_filtered: Vec<&RecentEntry> = recents
        .iter()
        .filter(|entry| !unlocked.iter().any(|p| p == &entry.path))
        .filter(|entry| matches_needle(&entry.path, &needle))
        .collect();

    let header = v_flex()
        .gap_1()
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::BOLD)
                .child("Switch vault"),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette::text_muted())
                .child(
                    "Open vaults switch instantly. Recent vaults prompt for the master password.",
                ),
        );

    let filter_field = div()
        .w_full()
        .child(Input::new(shell.vault_switcher_input()).cleanable(true));

    let mut sections: Vec<AnyElement> = Vec::new();
    if !unlocked_filtered.is_empty() {
        let rows: Vec<AnyElement> = unlocked_filtered
            .iter()
            .enumerate()
            .map(|(idx, path)| {
                let is_active = active_path.as_deref() == Some(path.as_path());
                open_row(idx, path, is_active, cx).into_any_element()
            })
            .collect();
        sections.push(
            section_block("Open", rows)
                .into_any_element(),
        );
    }
    if !recents_filtered.is_empty() {
        let rows: Vec<AnyElement> = recents_filtered
            .iter()
            .enumerate()
            .map(|(idx, entry)| recent_row(idx, entry, now, cx).into_any_element())
            .collect();
        sections.push(
            section_block("Recent", rows)
                .into_any_element(),
        );
    }
    let list_body: AnyElement = if sections.is_empty() {
        empty_state(needle.is_empty()).into_any_element()
    } else {
        v_flex().gap_4().children(sections).into_any_element()
    };

    let browse_row = div()
        .id("vault-switcher-browse")
        .on_click(
            cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
                shell
                    .state()
                    .clone()
                    .update(cx, |state, cx| state.close_overlay(cx));
                window.dispatch_action(Box::new(OpenVault), cx);
            }),
        )
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .p_3()
                .rounded(px(8.))
                .bg(palette::sidebar())
                .border_1()
                .border_color(palette::border())
                .child(
                    div()
                        .size(px(28.))
                        .rounded(px(6.))
                        .bg(palette::panel())
                        .border_1()
                        .border_color(palette::border())
                        .text_color(palette::blue())
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            gpui_component::Icon::from(AppIcon::Cloud)
                                .with_size(gpui_component::Size::Size(px(13.))),
                        ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(palette::text())
                                .child("Browse other vault…"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette::text_faint())
                                .child("Pick a .kdbx file from disk"),
                        ),
                ),
        );

    let footer = h_flex()
        .gap_2()
        .items_center()
        .text_xs()
        .text_color(palette::text_faint())
        .child("Enter to open · Esc to cancel");

    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(bg)
        .child(
            v_flex()
                .w(px(520.))
                .max_h(px(640.))
                .p_8()
                .bg(palette::panel())
                .border_1()
                .border_color(palette::border())
                .rounded(px(12.))
                .gap_4()
                .child(header)
                .child(filter_field)
                .child(
                    v_flex()
                        .id("vault-switcher-list")
                        .gap_1p5()
                        .min_h(px(0.))
                        .max_h(px(360.))
                        .overflow_y_scroll()
                        .child(list_body),
                )
                .child(browse_row)
                .child(
                    div()
                        .pt_3()
                        .border_t_1()
                        .border_color(palette::border())
                        .child(footer),
                ),
        )
        .into_any_element()
}

fn matches_needle(path: &Path, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let parent = path
        .parent()
        .map(|p| p.display().to_string().to_lowercase())
        .unwrap_or_default();
    file_name.contains(needle) || parent.contains(needle)
}

fn section_block(title: &'static str, rows: Vec<AnyElement>) -> impl IntoElement {
    v_flex()
        .gap_2()
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(palette::text_faint())
                .child(title),
        )
        .child(v_flex().gap_1p5().children(rows))
}

fn open_row(
    idx: usize,
    path: &Path,
    is_active: bool,
    cx: &mut Context<AppShell>,
) -> impl IntoElement {
    let path_owned = path.to_path_buf();
    let id: SharedString = format!("vault-switcher-open-{idx}").into();
    let file_name: SharedString = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "(unknown)".into())
        .into();
    let parent: SharedString = path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
        .into();
    let badge: SharedString = if is_active { "Active" } else { "Unlocked" }.into();

    div()
        .id(id)
        .on_click(
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, window, cx| {
                let path = path_owned.clone();
                shell
                    .state()
                    .clone()
                    .update(cx, |state, cx| state.close_overlay(cx));
                shell.open_recent(path, window, cx);
            }),
        )
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .p_3()
                .rounded(px(8.))
                .bg(palette::sidebar())
                .border_1()
                .border_color(palette::border())
                .when(is_active, |this| {
                    this.bg(palette::blue_soft())
                        .border_color(palette::blue_border())
                })
                .child(
                    div()
                        .size(px(28.))
                        .rounded(px(6.))
                        .bg(palette::panel())
                        .border_1()
                        .border_color(palette::border())
                        .text_color(palette::blue())
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            gpui_component::Icon::from(AppIcon::Note)
                                .with_size(gpui_component::Size::Size(px(13.))),
                        ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(palette::text())
                                .child(file_name),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette::text_faint())
                                .child(parent),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(palette::blue())
                        .child(badge),
                ),
        )
}

fn recent_row(
    idx: usize,
    entry: &RecentEntry,
    now: chrono::DateTime<chrono::Local>,
    cx: &mut Context<AppShell>,
) -> impl IntoElement {
    let path = entry.path.clone();
    let id: SharedString = format!("vault-switcher-recent-{idx}").into();
    let file_name: SharedString = entry
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "(unknown)".into())
        .into();
    let parent: SharedString = entry
        .path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
        .into();
    let elapsed: SharedString = relative_time_label(entry.last_opened_at, now).into();

    div()
        .id(id)
        .on_click(
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, window, cx| {
                let path = path.clone();
                shell
                    .state()
                    .clone()
                    .update(cx, |state, cx| state.close_overlay(cx));
                shell.open_recent(path, window, cx);
            }),
        )
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .p_3()
                .rounded(px(8.))
                .bg(palette::sidebar())
                .border_1()
                .border_color(palette::border())
                .child(
                    div()
                        .size(px(28.))
                        .rounded(px(6.))
                        .bg(palette::panel())
                        .border_1()
                        .border_color(palette::border())
                        .text_color(palette::text_muted())
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            gpui_component::Icon::from(AppIcon::Note)
                                .with_size(gpui_component::Size::Size(px(13.))),
                        ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(palette::text())
                                .child(file_name),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette::text_faint())
                                .child(parent),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette::text_faint())
                        .child(elapsed),
                ),
        )
}

fn empty_state(no_query: bool) -> impl IntoElement {
    let message = if no_query {
        "No vaults yet. Browse to add one below."
    } else {
        "No vaults match the filter. Press Enter to browse for one."
    };
    div()
        .p_4()
        .rounded(px(8.))
        .bg(palette::sidebar())
        .border_1()
        .border_color(palette::border())
        .text_xs()
        .text_color(palette::text_muted())
        .child(message)
}

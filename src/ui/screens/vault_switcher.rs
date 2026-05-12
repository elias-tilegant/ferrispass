//! Vault switcher command modal. It overlays the current screen so ⌘O
//! feels like a route picker, not a navigation away from the active task.

use std::path::{Path, PathBuf};

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::{h_flex, input::Input, v_flex};

use crate::app::RecentEntry;
use crate::app::actions::OpenVault;
use crate::app::time::relative_time_label;
use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::command_row::{RowTone, command_row};

pub fn render(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let state = shell.state().read(cx);
    let recents: Vec<RecentEntry> = state.recents().to_vec();
    let unlocked: Vec<PathBuf> = state.unlocked_paths();
    let active_path = state.current_vault_path();
    let query = shell.vault_switcher_input().read(cx).value().to_string();
    let needle = query.trim().to_lowercase();
    let now = chrono::Local::now();

    let unlocked_filtered: Vec<PathBuf> = unlocked
        .iter()
        .filter(|p| matches_needle(p, &needle))
        .cloned()
        .collect();
    let recents_filtered: Vec<&RecentEntry> = recents
        .iter()
        .filter(|entry| !unlocked.iter().any(|p| p == &entry.path))
        .filter(|entry| matches_needle(&entry.path, &needle))
        .collect();

    let mut sections: Vec<AnyElement> = Vec::new();
    if !unlocked_filtered.is_empty() {
        sections.push(
            section_block(
                "Open",
                unlocked_filtered
                    .iter()
                    .enumerate()
                    .map(|(idx, path)| {
                        open_row(
                            idx,
                            path,
                            active_path.as_deref() == Some(path.as_path()),
                            cx,
                        )
                        .into_any_element()
                    })
                    .collect(),
            )
            .into_any_element(),
        );
    }
    if !recents_filtered.is_empty() {
        sections.push(
            section_block(
                "Recent",
                recents_filtered
                    .iter()
                    .enumerate()
                    .map(|(idx, entry)| recent_row(idx, entry, now, cx).into_any_element())
                    .collect(),
            )
            .into_any_element(),
        );
    }

    let list_body: AnyElement = if sections.is_empty() {
        empty_state(needle.is_empty()).into_any_element()
    } else {
        v_flex().gap_4().children(sections).into_any_element()
    };

    div()
        .id("vault-switcher-backdrop")
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .left_0()
        .bg(palette::transparent_overlay())
        .occlude()
        .on_click(cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
            shell
                .state()
                .clone()
                .update(cx, |state, cx| state.close_overlay(cx));
        }))
        .flex()
        .items_start()
        .justify_center()
        .pt(px(72.))
        .px(px(20.))
        .pb(px(20.))
        .child(
            v_flex()
                .id("vault-switcher-panel")
                .w(px(560.))
                .max_h(px(620.))
                .rounded(px(10.))
                .bg(palette::panel())
                .border_1()
                .border_color(palette::border_strong())
                .overflow_hidden()
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(header())
                .child(
                    div()
                        .px_4()
                        .pb_3()
                        .child(Input::new(shell.vault_switcher_input()).cleanable(true)),
                )
                .child(
                    v_flex()
                        .id("vault-switcher-list")
                        .flex_1()
                        .min_h(px(0.))
                        .max_h(px(360.))
                        .overflow_y_scroll()
                        .px_4()
                        .pb_3()
                        .child(list_body),
                )
                .child(div().px_4().pb_4().child(browse_row(cx)))
                .child(footer()),
        )
        .into_any_element()
}

fn header() -> AnyElement {
    v_flex()
        .gap_0p5()
        .min_w(px(0.))
        .p_4()
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(palette::text())
                .child("Switch vault"),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette::text_muted())
                .child("Open vaults switch instantly. Recent vaults ask for the password."),
        )
        .into_any_element()
}

fn browse_row(cx: &mut Context<AppShell>) -> AnyElement {
    command_row(
        "vault-switcher-browse",
        AppIcon::Cloud,
        "Browse other vault...",
        "Pick a .kdbx file from disk",
        RowTone::Default,
        Some("Browse".into()),
        cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
            shell
                .state()
                .clone()
                .update(cx, |state, cx| state.close_overlay(cx));
            window.dispatch_action(Box::new(OpenVault), cx);
        }),
    )
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
        .gap_1p5()
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(palette::text_muted())
                .child(title),
        )
        .child(v_flex().gap_1().children(rows))
}

fn open_row(
    idx: usize,
    path: &Path,
    is_active: bool,
    cx: &mut Context<AppShell>,
) -> impl IntoElement {
    let path_owned = path.to_path_buf();
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

    command_row(
        SharedString::from(format!("vault-switcher-open-{idx}")),
        AppIcon::Note,
        file_name,
        parent,
        if is_active {
            RowTone::Primary
        } else {
            RowTone::Default
        },
        Some(badge),
        cx.listener(move |shell: &mut AppShell, _: &ClickEvent, window, cx| {
            let path = path_owned.clone();
            shell
                .state()
                .clone()
                .update(cx, |state, cx| state.close_overlay(cx));
            shell.open_recent(path, window, cx);
        }),
    )
}

fn recent_row(
    idx: usize,
    entry: &RecentEntry,
    now: chrono::DateTime<chrono::Local>,
    cx: &mut Context<AppShell>,
) -> impl IntoElement {
    let path = entry.path.clone();
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

    command_row(
        SharedString::from(format!("vault-switcher-recent-{idx}")),
        AppIcon::Note,
        file_name,
        parent,
        RowTone::Default,
        Some(elapsed),
        cx.listener(move |shell: &mut AppShell, _: &ClickEvent, window, cx| {
            let path = path.clone();
            shell
                .state()
                .clone()
                .update(cx, |state, cx| state.close_overlay(cx));
            shell.open_recent(path, window, cx);
        }),
    )
}

fn empty_state(no_query: bool) -> impl IntoElement {
    let message = if no_query {
        "No vaults yet. Browse to add one below."
    } else {
        "No vaults match the filter. Press Enter to browse."
    };
    div()
        .h(px(74.))
        .rounded(px(7.))
        .bg(palette::panel())
        .border_1()
        .border_color(palette::border())
        .flex()
        .items_center()
        .px_3()
        .text_xs()
        .text_color(palette::text_muted())
        .child(message)
}

fn footer() -> AnyElement {
    h_flex()
        .items_center()
        .justify_between()
        .border_t_1()
        .border_color(palette::border())
        .px_4()
        .py_3()
        .text_xs()
        .text_color(palette::text_faint())
        .child("Enter to open")
        .child("Esc to cancel")
        .into_any_element()
}

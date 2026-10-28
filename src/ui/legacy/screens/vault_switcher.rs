//! Vault switcher command modal. It overlays the current screen so ⌘O
//! feels like a route picker, not a navigation away from the active task.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gpui::{
    AnyElement, ClickEvent, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    WeakEntity, Window, div, px,
};
use gpui_component::{
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};

use crate::app::time::relative_time_label;
use crate::app::{AppState, RecentEntry};
use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::command_row::{RowTone, command_row};

pub struct VaultSwitcher {
    shell: WeakEntity<AppShell>,
    state: Entity<AppState>,
    input: Entity<InputState>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl VaultSwitcher {
    pub fn new(
        shell: WeakEntity<AppShell>,
        state: Entity<AppState>,
        input: Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let _subscriptions = vec![
            cx.subscribe_in(&input, window, Self::on_input_event),
            cx.observe(&state, |_: &mut Self, _, cx| cx.notify()),
        ];

        Self {
            shell,
            state,
            input,
            _subscriptions,
        }
    }

    fn on_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => cx.notify(),
            InputEvent::PressEnter { .. } => {
                let _ = self.shell.update(cx, move |shell, cx| {
                    shell.activate_vault_switcher_top(window, cx);
                });
            }
            _ => {}
        }
    }
}

impl Render for VaultSwitcher {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        render_switcher(self, cx)
    }
}

fn render_switcher(switcher: &VaultSwitcher, cx: &mut Context<VaultSwitcher>) -> AnyElement {
    let state = switcher.state.read(cx);
    let recents: Vec<RecentEntry> = state.recents().to_vec();
    let unlocked = state.unlocked_paths();
    let unlocked_set: HashSet<&Path> = unlocked.iter().map(PathBuf::as_path).collect();
    let active_path = state.current_vault_path();
    let query = switcher.input.read(cx).value().to_string();
    let needle = query.trim().to_lowercase();
    let now = chrono::Local::now();

    let unlocked_filtered: Vec<PathBuf> = unlocked
        .iter()
        .filter(|p| matches_needle(p, &needle))
        .cloned()
        .collect();
    let recents_filtered: Vec<&RecentEntry> = recents
        .iter()
        .filter(|entry| !unlocked_set.contains(entry.path.as_path()))
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
                            &switcher.shell,
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
                    .map(|(idx, entry)| {
                        recent_row(idx, entry, now, &switcher.shell, cx).into_any_element()
                    })
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
        .on_click(
            cx.listener(|switcher: &mut VaultSwitcher, _: &ClickEvent, _, cx| {
                switcher
                    .state
                    .update(cx, |state, cx| state.close_overlay(cx));
            }),
        )
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
                        .child(Input::new(&switcher.input).cleanable(true)),
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
                .child("Switch to an unlocked vault, or reopen a recent vault."),
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
    shell: &WeakEntity<AppShell>,
    cx: &mut Context<VaultSwitcher>,
) -> impl IntoElement {
    let path_owned = path.to_path_buf();
    let shell = shell.clone();
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
        cx.listener(move |_: &mut VaultSwitcher, _: &ClickEvent, window, cx| {
            let path = path_owned.clone();
            let _ = shell.update(cx, move |shell, cx| {
                shell
                    .state()
                    .clone()
                    .update(cx, |state, cx| state.close_overlay(cx));
                shell.open_recent(path, window, cx);
            });
        }),
    )
}

fn recent_row(
    idx: usize,
    entry: &RecentEntry,
    now: chrono::DateTime<chrono::Local>,
    shell: &WeakEntity<AppShell>,
    cx: &mut Context<VaultSwitcher>,
) -> impl IntoElement {
    let path = entry.path.clone();
    let shell = shell.clone();
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
        cx.listener(move |_: &mut VaultSwitcher, _: &ClickEvent, window, cx| {
            let path = path.clone();
            let _ = shell.update(cx, move |shell, cx| {
                shell
                    .state()
                    .clone()
                    .update(cx, |state, cx| state.close_overlay(cx));
                shell.open_recent(path, window, cx);
            });
        }),
    )
}

fn empty_state(no_query: bool) -> impl IntoElement {
    let message = if no_query {
        "No vaults to switch to. Use + to open another vault."
    } else {
        "No vaults match the filter."
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
        .child("Click a vault to switch")
        .child("Esc to cancel")
        .into_any_element()
}

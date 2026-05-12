use gpui::{
    AnyElement, App, ClickEvent, Context, IntoElement, ParentElement as _, SharedString,
    Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};

use crate::app::RecentEntry;
use crate::app::actions::{CreateVault, OpenConnect};
use crate::app::time::relative_time_label;
use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::brand::brand;
use crate::ui::widgets::command_row::{RowTone, command_row};
use crate::ui::widgets::update_chip;
use crate::update::UpdateStatus;

const RECENTS_LIMIT: usize = 4;

pub fn render(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let recents: Vec<RecentEntry> = shell.state().read(cx).recents().to_vec();
    let update_status = shell.state().read(cx).update_status().clone();

    div()
        .size_full()
        .bg(cx.theme().background)
        .flex()
        .items_center()
        .justify_center()
        .p(px(28.))
        .child(
            v_flex()
                .w_full()
                .max_w(px(760.))
                .gap_5()
                .child(header(&update_status, cx))
                .when(!recents.is_empty(), |this| {
                    this.child(recents_section(&recents, cx))
                })
                .child(actions_section(cx))
                .child(footer()),
        )
        .into_any_element()
}

fn header(status: &UpdateStatus, cx: &mut Context<AppShell>) -> AnyElement {
    h_flex()
        .items_start()
        .justify_between()
        .gap_4()
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .min_w(px(0.))
                .child(brand(34.))
                .child(
                    v_flex()
                        .gap_0p5()
                        .min_w(px(0.))
                        .child(
                            div()
                                .text_xl()
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(palette::text())
                                .child("FerrisPass"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(palette::text_muted())
                                .child("Open a vault or create a new encrypted database."),
                        ),
                ),
        )
        .when_some(update_chip(status, cx), |this, chip| this.child(chip))
        .into_any_element()
}

fn actions_section(cx: &mut Context<AppShell>) -> AnyElement {
    v_flex()
        .gap_2()
        .child(section_label("Actions"))
        .child(action_row(
            "welcome-open",
            AppIcon::Note,
            "Open Vault",
            "Pick a .kdbx file from disk",
            false,
            cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
                shell.prompt_for_vault_path(window, cx);
            }),
        ))
        .child(action_row(
            "welcome-cloud",
            AppIcon::Cloud,
            "Connect OneDrive",
            "Sync an existing vault from the cloud",
            true,
            cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                window.dispatch_action(Box::new(OpenConnect), cx);
            }),
        ))
        .child(action_row(
            "welcome-create",
            AppIcon::Key,
            "New Vault",
            "Start with an empty encrypted database",
            false,
            cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                window.dispatch_action(Box::new(CreateVault), cx);
            }),
        ))
        .into_any_element()
}

fn action_row<F>(
    id: &'static str,
    icon: AppIcon,
    title: &'static str,
    meta: &'static str,
    primary: bool,
    on_click: F,
) -> impl IntoElement
where
    F: Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
{
    command_row(
        id,
        icon,
        title,
        meta,
        if primary {
            RowTone::Primary
        } else {
            RowTone::Default
        },
        Some("Open".into()),
        on_click,
    )
}

fn recents_section(recents: &[RecentEntry], cx: &mut Context<AppShell>) -> impl IntoElement {
    let now = chrono::Local::now();
    let total = recents.len();
    let shown = total.min(RECENTS_LIMIT);
    let counter: Option<SharedString> = if total > RECENTS_LIMIT {
        Some(format!("Showing {shown} of {total}").into())
    } else {
        None
    };
    v_flex()
        .gap_2()
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(section_label("Recent vaults"))
                .when_some(counter, |this, label| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(palette::text_faint())
                            .child(label),
                    )
                }),
        )
        .child(
            v_flex()
                .gap_1()
                .children(recents.iter().take(RECENTS_LIMIT).enumerate().map(
                    |(idx, entry)| {
                        let path_for_listener = entry.path.clone();
                        let on_click = cx.listener(
                            move |shell: &mut AppShell, _: &ClickEvent, window, cx| {
                                shell.open_recent(path_for_listener.clone(), window, cx);
                            },
                        );
                        recent_row(idx, entry, now, on_click).into_any_element()
                    },
                )),
        )
}

fn recent_row<F>(
    idx: usize,
    entry: &RecentEntry,
    now: chrono::DateTime<chrono::Local>,
    on_click: F,
) -> impl IntoElement
where
    F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
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
        SharedString::from(format!("welcome-recent-{idx}")),
        AppIcon::Note,
        file_name,
        parent,
        RowTone::Default,
        Some(elapsed),
        on_click,
    )
}

fn footer() -> AnyElement {
    h_flex()
        .items_center()
        .justify_between()
        .pt_2()
        .text_xs()
        .text_color(palette::text_faint())
        .child(format!("v{} · KDBX 4.1", crate::app::APP_VERSION))
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .child(
                    gpui_component::Icon::from(AppIcon::Shield)
                        .with_size(gpui_component::Size::Size(px(12.)))
                        .text_color(palette::text_faint()),
                )
                .child("AES-256 · Argon2id"),
        )
        .into_any_element()
}

fn section_label(label: &'static str) -> AnyElement {
    div()
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(palette::text_muted())
        .child(label)
        .into_any_element()
}

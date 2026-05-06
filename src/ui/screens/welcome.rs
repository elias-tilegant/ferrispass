use gpui::{
    AnyElement, App, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};

use crate::app::RecentEntry;
use crate::app::actions::{CreateVault, OpenConnect};
use crate::app::time::relative_time_label;
use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::brand::brand;
use crate::update::UpdateStatus;

pub fn render(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let theme = cx.theme();
    let bg_overlay = format!(
        "background:radial-gradient(ellipse at 30% 20%,{} 0%, transparent 60%),",
        css_color(palette::blue_soft())
    );
    let _ = bg_overlay;

    // Snapshot recents up front so the listener closures can move clones
    // of each path without re-borrowing `shell` inside the render tree.
    let recents: Vec<RecentEntry> = shell.state().read(cx).recents().to_vec();

    // Snapshot update status so the conditional banner below has a stable
    // value to render against without re-reading state mid-build.
    let update_status = shell.state().read(cx).update_status().clone();

    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.background)
        .child(
            v_flex()
                .w(px(460.))
                .p_10()
                .bg(palette::panel())
                .border_1()
                .border_color(palette::border())
                .rounded(px(12.))
                .gap_5()
                .child(brand(48.))
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_xl()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child("Welcome to FerrisPass"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(palette::text_muted())
                                .child(
                                    "A native, Rust-built password manager. Your vault is encrypted locally and synced through your own cloud.",
                                ),
                        ),
                )
                .when(!recents.is_empty(), |this| {
                    this.child(recents_section(&recents, cx))
                })
                .child(
                    v_flex()
                        .gap_2()
                        .child(choice_row(
                            "welcome-open",
                            AppIcon::Note,
                            "Open existing database",
                            ".kdbx file from disk",
                            false,
                            cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
                                shell.prompt_for_vault_path(window, cx);
                            }),
                        ))
                        .child(choice_row(
                            "welcome-cloud",
                            AppIcon::Cloud,
                            "Connect OneDrive",
                            "Sync an existing vault from the cloud",
                            true,
                            cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                                window.dispatch_action(Box::new(OpenConnect), cx);
                            }),
                        ))
                        .child(choice_row(
                            "welcome-create",
                            AppIcon::Key,
                            "Create new database",
                            "Start fresh with a new .kdbx vault",
                            false,
                            cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                                window.dispatch_action(Box::new(CreateVault), cx);
                            }),
                        )),
                )
                .when(update_status.has_visible_update(), |this| {
                    this.child(update_banner(&update_status, cx))
                })
                .child(
                    div()
                        .pt_4()
                        .border_t_1()
                        .border_color(palette::border())
                        .child(
                            h_flex()
                                .items_center()
                                .justify_between()
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
                                ),
                        ),
                ),
        )
        .into_any_element()
}

fn update_banner(status: &UpdateStatus, cx: &mut Context<AppShell>) -> AnyElement {
    let (label, action_label, clickable) = match status {
        UpdateStatus::Available(info) => (
            SharedString::from(format!("Update available: FerrisPass {}", info.version)),
            Some(SharedString::from("Install")),
            true,
        ),
        UpdateStatus::Downloading { .. } => (
            "Downloading update…".into(),
            None,
            false,
        ),
        UpdateStatus::ReadyToRestart => (
            "Update installed. Restart FerrisPass to apply.".into(),
            None,
            false,
        ),
        // Other variants don't pass `has_visible_update`, but match
        // exhaustively so a future variant can't sneak through silently.
        UpdateStatus::Idle | UpdateStatus::Checking | UpdateStatus::Failed(_) => {
            return div().into_any_element();
        }
    };

    let action_button = action_label.map(|label| {
        div()
            .id("welcome-update-install")
            .cursor_pointer()
            .hover(|s| s.opacity(0.85))
            .h(px(28.))
            .px(px(12.))
            .rounded(px(6.))
            .bg(palette::blue())
            .text_color(palette::panel())
            .text_xs()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .flex()
            .items_center()
            .justify_center()
            .child(label)
            .on_click(cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
                shell.state().clone().update(cx, |state, cx| {
                    state.install_update(cx);
                });
            }))
    });

    let row = h_flex()
        .gap_3()
        .items_center()
        .justify_between()
        .px(px(12.))
        .py(px(10.))
        .rounded(px(8.))
        .bg(palette::blue_soft())
        .border_1()
        .border_color(palette::blue_border())
        .child(
            div()
                .flex_1()
                .text_xs()
                .text_color(palette::text())
                .child(label),
        );

    let row = if let Some(button) = action_button {
        row.child(button)
    } else {
        row
    };

    let _ = clickable;
    row.into_any_element()
}

fn choice_row<F>(
    id: &'static str,
    icon: AppIcon,
    title: &'static str,
    meta: &'static str,
    accent: bool,
    on_click: F,
) -> impl gpui::IntoElement
where
    F: Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
{
    let bg = if accent {
        palette::blue_soft()
    } else {
        palette::sidebar()
    };
    let border = if accent {
        palette::blue_border()
    } else {
        palette::border()
    };
    let icon_bg = if accent {
        palette::blue()
    } else {
        palette::panel()
    };
    let icon_color = if accent {
        palette::panel()
    } else {
        palette::text()
    };
    let title_color = if accent {
        palette::blue_hover()
    } else {
        palette::text()
    };

    div().id(id).on_click(on_click).child(
        h_flex()
            .gap_3()
            .items_center()
            .p_3()
            .rounded(px(8.))
            .bg(bg)
            .border_1()
            .border_color(border)
            .child(
                div()
                    .size(px(32.))
                    .rounded(px(7.))
                    .bg(icon_bg)
                    .text_color(icon_color)
                    .border_1()
                    .border_color(if accent {
                        palette::blue()
                    } else {
                        palette::border()
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        gpui_component::Icon::from(icon)
                            .with_size(gpui_component::Size::Size(px(15.))),
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
                            .text_color(title_color)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette::text_muted())
                            .child(meta),
                    ),
            )
            .child(
                gpui_component::Icon::from(gpui_component::IconName::ChevronRight)
                    .with_size(gpui_component::Size::Size(px(14.)))
                    .text_color(palette::text_faint()),
            ),
    )
}

/// "Recent" section above the three choice rows. Hidden when the
/// recents list is empty (`render` skips this whole subtree via `.when`).
/// Caps the visible list at 5 — enough to give a useful shortcut without
/// dwarfing the primary actions.
fn recents_section(recents: &[RecentEntry], cx: &mut Context<AppShell>) -> impl IntoElement {
    let now = chrono::Local::now();
    v_flex()
        .gap_2()
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(palette::text_muted())
                .child("Recent"),
        )
        .children(recents.iter().take(5).enumerate().map(|(idx, entry)| {
            let path_for_listener = entry.path.clone();
            let on_click = cx.listener(move |shell: &mut AppShell, _: &ClickEvent, window, cx| {
                shell.open_recent(path_for_listener.clone(), window, cx);
            });
            recent_row(idx, entry, now, on_click)
        }))
}

fn recent_row<F>(
    idx: usize,
    entry: &RecentEntry,
    now: chrono::DateTime<chrono::Local>,
    on_click: F,
) -> impl gpui::IntoElement
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
    let id = SharedString::from(format!("welcome-recent-{idx}"));

    div().id(id).on_click(on_click).child(
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

fn css_color(c: gpui::Hsla) -> String {
    let r = (c.l * 255.0) as u8;
    format!("rgb({r},{r},{r})")
}

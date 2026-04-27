use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement as _,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};

use crate::app::actions::OpenConflictDemo;
use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::atoms::{ChipTone, chip};
use crate::ui::widgets::sync_row::{SyncOutcome, sync_row};
use crate::ui::widgets::toggle_row::toggle_row;

pub fn render(_shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let sidebar = settings_sidebar();
    let panel = content_panel(cx);

    h_flex()
        .size_full()
        .bg(cx.theme().background)
        .child(sidebar)
        .child(panel)
        .into_any_element()
}

fn settings_sidebar() -> AnyElement {
    let items = [
        (AppIcon::Key, "General", false),
        (AppIcon::Shield, "Security", false),
        (AppIcon::Cloud, "Sync", true),
        (AppIcon::Sync, "Auto-type", false),
        (AppIcon::Note, "Backups", false),
        (AppIcon::Refresh, "Advanced", false),
    ];

    let mut col = v_flex()
        .w(px(200.))
        .flex_shrink_0()
        .h_full()
        .pt_4()
        .bg(palette::SIDEBAR)
        .border_r_1()
        .border_color(palette::BORDER)
        .child(
            div()
                .px_3p5()
                .pb_2p5()
                .text_xs()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(palette::TEXT_FAINT)
                .child("SETTINGS"),
        );

    for (icon, label, selected) in items {
        let bg = if selected { palette::BLUE } else { palette::SIDEBAR };
        let fg = if selected { palette::PANEL } else { palette::TEXT };
        let icon_color = if selected { palette::PANEL } else { palette::TEXT_MUTED };
        col = col.child(
            h_flex()
                .gap_2()
                .items_center()
                .h(px(28.))
                .mx(px(6.))
                .px_3p5()
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
                        .text_color(icon_color),
                )
                .child(label),
        );
    }

    col.into_any_element()
}

fn content_panel(cx: &mut Context<AppShell>) -> AnyElement {
    let close = close_button(cx);
    let activity = activity_section(cx);

    v_flex()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .bg(palette::PANEL)
        .child(
            v_flex()
                .gap_1()
                .px_8()
                .pt_5()
                .pb_4()
                .border_b_1()
                .border_color(palette::BORDER)
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(palette::TEXT_MUTED)
                        .child("SETTINGS"),
                )
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xl()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child("Cloud sync"),
                        )
                        .child(close),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette::TEXT_MUTED)
                        .child(
                            "Keep your encrypted vault in sync across devices via your own cloud storage.",
                        ),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_h(px(0.))
                .gap_6()
                .p_8()
                .child(connected_card())
                .child(activity)
                .child(behavior_section()),
        )
        .into_any_element()
}

fn connected_card() -> AnyElement {
    v_flex()
        .gap_4()
        .p_4()
        .rounded(px(10.))
        .bg(palette::BLUE_SOFT)
        .border_1()
        .border_color(palette::BLUE_BORDER)
        .child(
            h_flex()
                .gap_3p5()
                .items_center()
                .child(
                    div()
                        .size(px(44.))
                        .rounded(px(9.))
                        .bg(palette::PANEL)
                        .border_1()
                        .border_color(palette::BLUE_BORDER)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            gpui_component::Icon::from(AppIcon::Cloud)
                                .with_size(gpui_component::Size::Size(px(22.)))
                                .text_color(palette::BLUE),
                        ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .gap_0p5()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .child("OneDrive"),
                                )
                                .child(chip("Connected", ChipTone::Green)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette::TEXT_MUTED)
                                .font_family("JetBrains Mono")
                                .child("jonas.ritter@gmx.de · 47.2 GB of 1 TB used"),
                        ),
                )
                .child(
                    div()
                        .h(px(28.))
                        .px(px(10.))
                        .rounded(px(6.))
                        .bg(palette::PANEL)
                        .border_1()
                        .border_color(palette::BORDER_STRONG)
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(palette::TEXT)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child("Disconnect"),
                ),
        )
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .p_3()
                .rounded(px(7.))
                .bg(palette::PANEL)
                .border_1()
                .border_color(palette::BLUE_BORDER)
                .child(
                    div()
                        .size(px(32.))
                        .rounded(px(6.))
                        .bg(palette::ORANGE_SOFT)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            gpui_component::Icon::from(AppIcon::Key)
                                .with_size(gpui_component::Size::Size(px(15.)))
                                .text_color(palette::ORANGE),
                        ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .gap_0p5()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .font_family("JetBrains Mono")
                                .child("/Apps/KeePassRS/Personal.kdbx"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette::TEXT_MUTED)
                                .child("4.8 MB · last modified 2 minutes ago"),
                        ),
                )
                .child(
                    div()
                        .h(px(24.))
                        .px(px(8.))
                        .rounded(px(4.))
                        .bg(palette::PANEL)
                        .border_1()
                        .border_color(palette::BORDER_STRONG)
                        .text_xs()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(palette::TEXT)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child("Change…"),
                ),
        )
        .into_any_element()
}

fn activity_section(cx: &mut Context<AppShell>) -> AnyElement {
    let sync_now_button = div()
        .id("sync-now")
        .h(px(24.))
        .px(px(10.))
        .rounded(px(4.))
        .bg(palette::PANEL)
        .border_1()
        .border_color(palette::BORDER_STRONG)
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(palette::TEXT)
        .flex()
        .items_center()
        .justify_center()
        .gap_1()
        .child(
            gpui_component::Icon::from(AppIcon::Sync)
                .with_size(gpui_component::Size::Size(px(11.)))
                .text_color(palette::TEXT),
        )
        .child("Sync now")
        .on_click(cx.listener(
            |_: &mut AppShell, _: &ClickEvent, window, cx| {
                window.dispatch_action(Box::new(OpenConflictDemo), cx);
            },
        ));

    v_flex()
        .gap_2p5()
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child("Sync activity"),
                )
                .child(sync_now_button),
        )
        .child(
            v_flex()
                .border_1()
                .border_color(palette::BORDER)
                .rounded(px(8.))
                .bg(palette::PANEL)
                .overflow_hidden()
                .child(sync_row(
                    SyncOutcome::Success,
                    "Pulled 2 changes",
                    "Added: Linear · Modified: AWS Console",
                    "2 min ago",
                    false,
                ))
                .child(sync_row(
                    SyncOutcome::Success,
                    "Push complete",
                    "1 entry uploaded",
                    "14 min ago",
                    false,
                ))
                .child(sync_row(
                    SyncOutcome::Merge,
                    "Conflict resolved",
                    "Kept local version of 'GitHub'",
                    "3 hours ago",
                    false,
                ))
                .child(sync_row(
                    SyncOutcome::Success,
                    "Pulled 5 changes",
                    "From iPhone · 12.4.0",
                    "Yesterday, 18:42",
                    false,
                ))
                .child(sync_row(
                    SyncOutcome::Success,
                    "Initial sync",
                    "142 entries · 4.8 MB",
                    "3 days ago",
                    true,
                )),
        )
        .into_any_element()
}

fn behavior_section() -> AnyElement {
    v_flex()
        .gap_2p5()
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::BOLD)
                .child("Behavior"),
        )
        .child(
            v_flex()
                .border_1()
                .border_color(palette::BORDER)
                .rounded(px(8.))
                .bg(palette::PANEL)
                .child(toggle_row(
                    "Sync automatically",
                    "Push changes within 30 seconds, pull on focus",
                    true,
                    false,
                ))
                .child(toggle_row(
                    "Sync on unlock",
                    "Pull latest version when you unlock the vault",
                    true,
                    false,
                ))
                .child(toggle_row(
                    "Background sync when locked",
                    "Periodic checks every 15 minutes",
                    false,
                    false,
                ))
                .child(toggle_row(
                    "Conflict resolution",
                    "Three-way merge with manual review",
                    true,
                    true,
                )),
        )
        .into_any_element()
}

fn close_button(cx: &mut Context<AppShell>) -> AnyElement {
    div()
        .id("settings-close")
        .h(px(28.))
        .px(px(10.))
        .rounded(px(6.))
        .bg(palette::PANEL)
        .border_1()
        .border_color(palette::BORDER_STRONG)
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(palette::TEXT)
        .flex()
        .items_center()
        .justify_center()
        .child("Close")
        .on_click(cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
            let state = shell.state().clone();
            state.update(cx, |state, cx| {
                let _ = state.close_overlay(cx);
            });
        }))
        .into_any_element()
}

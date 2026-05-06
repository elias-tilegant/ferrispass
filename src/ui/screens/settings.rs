//! Unified Settings overlay (⌘,). Mac-style: left sidebar with tabs,
//! right content panel. Currently two tabs are wired (General, Sync);
//! the rest are stub placeholders that match the previous mock so the
//! visual hierarchy doesn't change as we fill them in.

use gpui::{
    AnyElement, App, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};

use crate::app::actions::DownloadFavicons;
use crate::app::{AppSettings, FaviconDownloadStatus, VaultStatus};
use crate::ui::app_shell::{AppShell, SettingsTab};
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::update::UpdateStatus;

const AUTO_LOCK_PRESETS: &[Option<u64>] = &[Some(60), Some(240), Some(900), None];
const CLIPBOARD_CLEAR_PRESETS: &[Option<u64>] = &[Some(5), Some(10), Some(30), None];

pub fn render(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let active = shell.settings_tab();

    let (title, subtitle, body) = match active {
        SettingsTab::General => (
            "General",
            "Timeouts and behaviour for the running app. Apply immediately and save on disk.",
            general_tab_body(shell, cx).into_any_element(),
        ),
        SettingsTab::Sync => (
            "Cloud sync",
            "Keep your encrypted vault in sync across devices via your own cloud storage.",
            crate::ui::screens::sync_settings::render_tab_body(shell, cx),
        ),
    };

    h_flex()
        .size_full()
        .bg(cx.theme().background)
        .child(sidebar(active, cx))
        .child(content_panel(title, subtitle, body, cx))
        .into_any_element()
}

// --------------- chrome ---------------

fn sidebar(active: SettingsTab, cx: &mut Context<AppShell>) -> AnyElement {
    // (icon, label, this-tab, enabled). Disabled stubs preserve the
    // visual roadmap from the original mock; they're not clickable.
    let items: &[(AppIcon, &str, Option<SettingsTab>, bool)] = &[
        (AppIcon::Key, "General", Some(SettingsTab::General), true),
        (AppIcon::Shield, "Security", None, false),
        (AppIcon::Cloud, "Sync", Some(SettingsTab::Sync), true),
        (AppIcon::Sync, "Auto-type", None, false),
        (AppIcon::Note, "Backups", None, false),
        (AppIcon::Refresh, "Advanced", None, false),
    ];

    let mut col = v_flex()
        .w(px(200.))
        .flex_shrink_0()
        .h_full()
        .pt_4()
        .bg(palette::sidebar())
        .border_r_1()
        .border_color(palette::border())
        .child(
            div()
                .px_3p5()
                .pb_2p5()
                .text_xs()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(palette::text_faint())
                .child("SETTINGS"),
        );

    for (icon, label, tab, enabled) in items {
        let is_active = tab.is_some_and(|t| t == active);
        col = col.child(sidebar_item(*icon, label, *tab, *enabled, is_active, cx));
    }

    col.into_any_element()
}

fn sidebar_item(
    icon: AppIcon,
    label: &'static str,
    tab: Option<SettingsTab>,
    enabled: bool,
    selected: bool,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let bg = if selected {
        palette::blue()
    } else {
        palette::sidebar()
    };
    let fg = if selected {
        palette::panel()
    } else if enabled {
        palette::text()
    } else {
        palette::text_faint()
    };
    let icon_color = if selected {
        palette::panel()
    } else if enabled {
        palette::text_muted()
    } else {
        palette::text_faint()
    };

    let row = h_flex()
        .gap_2()
        .items_center()
        .h(px(28.))
        .mx(px(6.))
        .px_3p5()
        .rounded(px(5.))
        .bg(bg)
        .text_color(fg)
        .text_sm()
        .child(
            gpui_component::Icon::from(icon)
                .with_size(gpui_component::Size::Size(px(13.)))
                .text_color(icon_color),
        )
        .child(label);

    if let (true, Some(target)) = (enabled, tab) {
        let id = SharedString::from(format!("settings-tab-{label}"));
        div()
            .id(id)
            .cursor_pointer()
            .on_click(
                cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                    shell.set_settings_tab(target, cx);
                }),
            )
            .child(row)
            .into_any_element()
    } else {
        row.into_any_element()
    }
}

fn content_panel(
    title: &'static str,
    subtitle: &'static str,
    body: AnyElement,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    v_flex()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .bg(palette::panel())
        .child(
            v_flex()
                .gap_1()
                .px_8()
                .pt_5()
                .pb_4()
                .border_b_1()
                .border_color(palette::border())
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(palette::text_muted())
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
                                .child(title),
                        )
                        .child(close_button(cx)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette::text_muted())
                        .child(subtitle),
                ),
        )
        .child(v_flex().flex_1().min_h(px(0.)).gap_6().p_8().child(body))
        .into_any_element()
}

fn close_button(cx: &mut Context<AppShell>) -> AnyElement {
    div()
        .id("close-settings")
        .h(px(28.))
        .px(px(10.))
        .rounded(px(5.))
        .bg(palette::panel())
        .border_1()
        .border_color(palette::border_strong())
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(palette::text())
        .flex()
        .items_center()
        .justify_center()
        .child("Close")
        .cursor_pointer()
        .hover(|s| s.opacity(0.85))
        .on_click(cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
            shell.state().clone().update(cx, |state, cx| {
                let _ = state.close_overlay(cx);
            });
        }))
        .into_any_element()
}

// --------------- General tab body ---------------

fn general_tab_body(shell: &AppShell, cx: &mut Context<AppShell>) -> impl IntoElement {
    let settings = shell.settings().clone();
    let state = shell.state().read(cx);
    let favicon_status = state.favicon_status().clone();
    let update_status = state.update_status().clone();
    let vault_open = matches!(state.vault_status(), VaultStatus::Open { .. });
    v_flex()
        .gap_6()
        .child(auto_lock_section(&settings, cx))
        .child(clipboard_section(&settings, cx))
        .child(favicon_section(&favicon_status, vault_open, cx))
        .child(updates_section(&settings, &update_status, cx))
}

fn auto_lock_section(settings: &AppSettings, cx: &mut Context<AppShell>) -> impl IntoElement {
    let current = settings.auto_lock_secs;
    let mut row = h_flex().gap_2().flex_wrap();
    for (idx, preset) in AUTO_LOCK_PRESETS.iter().enumerate() {
        let preset_value = *preset;
        let selected = preset_value == current;
        let baseline = settings.clone();
        row = row.child(preset_chip(
            SharedString::from(format!("auto-lock-preset-{idx}")),
            format_auto_lock_label(preset_value),
            selected,
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                shell.update_settings(
                    AppSettings {
                        auto_lock_secs: preset_value,
                        ..baseline.clone()
                    },
                    cx,
                );
            }),
        ));
    }
    section_frame(
        "Auto-lock vault",
        "Lock the vault after this much idle time without keyboard or mouse activity.",
        row,
    )
}

fn clipboard_section(settings: &AppSettings, cx: &mut Context<AppShell>) -> impl IntoElement {
    let current = settings.clipboard_clear_secs;
    let mut row = h_flex().gap_2().flex_wrap();
    for (idx, preset) in CLIPBOARD_CLEAR_PRESETS.iter().enumerate() {
        let preset_value = *preset;
        let selected = preset_value == current;
        let baseline = settings.clone();
        row = row.child(preset_chip(
            SharedString::from(format!("clipboard-clear-preset-{idx}")),
            format_clipboard_label(preset_value),
            selected,
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                shell.update_settings(
                    AppSettings {
                        clipboard_clear_secs: preset_value,
                        ..baseline.clone()
                    },
                    cx,
                );
            }),
        ));
    }
    section_frame(
        "Clear clipboard after copy",
        "Wipe a copied password / username / TOTP after this many seconds. \
         The clipboard always wipes when you lock the vault.",
        row,
    )
}

fn favicon_section(
    status: &FaviconDownloadStatus,
    vault_open: bool,
    cx: &mut Context<AppShell>,
) -> impl IntoElement {
    let running = status.is_running();
    // Click is gated on (vault open + not currently running). When the
    // gate fails we still render the same chrome but skip wiring the
    // listener — the chip styles below mute the colours so the user can
    // see why it isn't actionable.
    let enabled = vault_open && !running;

    let label: SharedString = match status {
        FaviconDownloadStatus::Idle => "Download favicons".into(),
        FaviconDownloadStatus::Running { done, total, .. } => {
            format!("Downloading… {done}/{total}").into()
        }
        FaviconDownloadStatus::Finished { succeeded, total } => {
            format!("Download favicons · last run: {succeeded}/{total}").into()
        }
    };

    let mut button = h_flex()
        .id("download-favicons")
        .h(px(28.))
        .px_3()
        .gap_2()
        .items_center()
        .rounded(px(6.))
        .border_1()
        .border_color(if enabled {
            palette::blue_border()
        } else {
            palette::border()
        })
        .bg(if enabled {
            palette::blue_soft()
        } else {
            palette::sidebar()
        })
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(if enabled {
            palette::blue()
        } else {
            palette::text_muted()
        })
        .child(
            gpui_component::Icon::from(AppIcon::Cloud)
                .with_size(gpui_component::Size::Size(px(12.))),
        )
        .child(label);
    if enabled {
        button = button.on_click(cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
            window.dispatch_action(Box::new(DownloadFavicons), cx);
        }));
    }

    let hint = match (vault_open, status) {
        (false, _) => "Open a vault first — favicons are stored inside the database.",
        (true, FaviconDownloadStatus::Running { .. }) => {
            "Fetching one site at a time so we don't hammer the icon service."
        }
        (
            true,
            FaviconDownloadStatus::Finished {
                succeeded: 0,
                total: 0,
            },
        ) => "Every URL entry already has a custom icon. Nothing to do.",
        (true, _) => {
            "Pulls a small icon from DuckDuckGo's icon service for every URL \
             entry that doesn't already have a custom icon. Sends each \
             hostname to icons.duckduckgo.com."
        }
    };

    section_frame("Favicons", hint, button)
}

fn updates_section(
    settings: &AppSettings,
    update_status: &UpdateStatus,
    cx: &mut Context<AppShell>,
) -> impl IntoElement {
    let auto_check = settings.auto_update_check_enabled;

    // Status line — concise plain text below the chips. Cycles through
    // checking → available → idle/failed depending on what `start_update_check`
    // last produced.
    let status_label: SharedString = match update_status {
        UpdateStatus::Idle => "You're on the latest version.".into(),
        UpdateStatus::Checking => "Checking for updates…".into(),
        UpdateStatus::Available(info) => {
            SharedString::from(format!("Update available: FerrisPass {}", info.version))
        }
        UpdateStatus::Downloading { .. } => "Downloading update…".into(),
        UpdateStatus::ReadyToRestart => "Update installed. Restart FerrisPass to apply.".into(),
        UpdateStatus::Failed(msg) => SharedString::from(format!("Update check failed: {msg}")),
    };

    // Toggle: On/Off chip pair. Mirrors the auto-lock preset row visually.
    let on_baseline = settings.clone();
    let off_baseline = settings.clone();
    let toggle_row = h_flex()
        .gap_2()
        .child(preset_chip(
            "auto-update-on".into(),
            "On".into(),
            auto_check,
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                shell.update_settings(
                    AppSettings {
                        auto_update_check_enabled: true,
                        ..on_baseline.clone()
                    },
                    cx,
                );
            }),
        ))
        .child(preset_chip(
            "auto-update-off".into(),
            "Off".into(),
            !auto_check,
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                shell.update_settings(
                    AppSettings {
                        auto_update_check_enabled: false,
                        ..off_baseline.clone()
                    },
                    cx,
                );
            }),
        ));

    // "Check now" — manual trigger regardless of auto-check setting.
    let check_now = preset_chip(
        "auto-update-check-now".into(),
        "Check now".into(),
        false,
        cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
            shell.state().clone().update(cx, |state, cx| {
                state.start_update_check(cx);
            });
        }),
    );

    let body = v_flex()
        .gap_3()
        .child(toggle_row)
        .child(check_now)
        .child(
            div()
                .text_xs()
                .text_color(palette::text_muted())
                .child(status_label),
        );

    section_frame(
        "Updates",
        "FerrisPass checks GitHub for new releases on launch (rate-limited \
         to once per day). Update bundles are verified against an embedded \
         Ed25519 public key before install.",
        body,
    )
}

fn section_frame(
    title: &'static str,
    description: &'static str,
    chips: impl IntoElement,
) -> impl IntoElement {
    v_flex()
        .gap_2()
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette::text_muted())
                .child(description),
        )
        .child(chips)
}

fn preset_chip<F>(
    id: SharedString,
    label: SharedString,
    selected: bool,
    on_click: F,
) -> impl IntoElement
where
    F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let (bg, fg, border) = if selected {
        (palette::blue(), palette::panel(), palette::blue_hover())
    } else {
        (
            palette::sidebar(),
            palette::text(),
            palette::border_strong(),
        )
    };

    div()
        .id(id)
        .cursor_pointer()
        .hover(|s| s.opacity(0.85))
        .on_click(on_click)
        .child(
            h_flex()
                .h(px(30.))
                .px(px(14.))
                .items_center()
                .justify_center()
                .rounded(px(6.))
                .bg(bg)
                .text_color(fg)
                .border_1()
                .border_color(border)
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(label),
        )
}

fn format_auto_lock_label(secs: Option<u64>) -> SharedString {
    match secs {
        None => SharedString::from("Never"),
        Some(60) => SharedString::from("1 min"),
        Some(s) if s % 60 == 0 => SharedString::from(format!("{} min", s / 60)),
        Some(s) => SharedString::from(format!("{s} s")),
    }
}

fn format_clipboard_label(secs: Option<u64>) -> SharedString {
    match secs {
        None => SharedString::from("Never"),
        Some(s) => SharedString::from(format!("{s} s")),
    }
}

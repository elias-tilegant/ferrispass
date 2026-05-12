//! Unified Settings overlay (⌘,). Mac-style: left sidebar with tabs,
//! right content panel. Currently two tabs are wired (General, Sync);
//! the rest are stub placeholders that match the previous mock so the
//! visual hierarchy doesn't change as we fill them in.

use gpui::{
    AnyElement, App, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder, px,
};
use gpui_component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};

use crate::app::actions::{DownloadFavicons, InstallUpdate, OpenWhatsNew, RestartToUpdate};
use crate::app::{AppSettings, FaviconDownloadStatus, VaultStatus};
use crate::ui::app_shell::{AppShell, SettingsTab};
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::update::UpdateStatus;

const AUTO_LOCK_PRESETS: &[Option<u64>] = &[Some(60), Some(240), Some(900), None];
const CLIPBOARD_CLEAR_PRESETS: &[Option<u64>] = &[Some(5), Some(10), Some(30), None];
/// Allowed cleanup TTLs for the launch tempfile. Bracketed by
/// `LAUNCH_CLEANUP_SECS_RANGE` (10..=60) so any preset added here
/// stays within the validated band.
const LAUNCH_CLEANUP_PRESETS: &[u32] = &[10, 30, 60];

pub fn render(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    // Cloud sync only makes sense once a vault is decrypted — we don't
    // have document state to sync against otherwise, and exposing the
    // OneDrive picker before unlock would invite users to authorise an
    // account they then can't actually attach to anything. Sidebar
    // renders Sync as disabled, and if the user somehow lands on the
    // Sync tab (e.g. auto-lock while Settings is open) we silently
    // fall back to the General body.
    let vault_unlocked = matches!(
        shell.state().read(cx).vault_status(),
        VaultStatus::Open { .. }
    );
    let active = shell.settings_tab();
    let effective = if matches!(active, SettingsTab::Sync) && !vault_unlocked {
        SettingsTab::General
    } else {
        active
    };

    let (title, subtitle, body) = match effective {
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
        SettingsTab::AutoType => (
            "Auto-Type",
            "Press a global hotkey to type the matching entry's username and password into \
             the previously-focused window — works in any app or browser.",
            auto_type_tab_body(shell, cx).into_any_element(),
        ),
    };

    h_flex()
        .size_full()
        .bg(cx.theme().background)
        .child(sidebar(effective, vault_unlocked, cx))
        .child(content_panel(title, subtitle, body, cx))
        .into_any_element()
}

// --------------- chrome ---------------

fn sidebar(active: SettingsTab, vault_unlocked: bool, cx: &mut Context<AppShell>) -> AnyElement {
    // (icon, label, this-tab, enabled). Disabled stubs preserve the
    // visual roadmap from the original mock; they're not clickable.
    // Sync is gated on `vault_unlocked` — see `render` for the rationale.
    let items: &[(AppIcon, &str, Option<SettingsTab>, bool)] = &[
        (AppIcon::Key, "General", Some(SettingsTab::General), true),
        (AppIcon::Shield, "Security", None, false),
        (
            AppIcon::Cloud,
            "Sync",
            Some(SettingsTab::Sync),
            vault_unlocked,
        ),
        (
            AppIcon::Sync,
            "Auto-type",
            Some(SettingsTab::AutoType),
            true,
        ),
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
    let whats_new_available = state.whats_new_info().is_some();
    let vault_open = matches!(state.vault_status(), VaultStatus::Open { .. });
    v_flex()
        .gap_6()
        .child(auto_lock_section(&settings, cx))
        .child(clipboard_section(&settings, cx))
        .child(launch_cleanup_section(&settings, cx))
        .child(favicon_section(&favicon_status, vault_open, cx))
        .child(updates_section(
            &settings,
            &update_status,
            whats_new_available,
            cx,
        ))
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

/// "Auto-clean launch payloads after N seconds" picker. Mirrors the
/// shape of the clipboard picker — three preset chips, the selected
/// one highlighted. Range is clamped on read in `AppSettings` so a
/// hand-edited settings file can't push the timer outside 10..=60 s.
fn launch_cleanup_section(settings: &AppSettings, cx: &mut Context<AppShell>) -> impl IntoElement {
    let current = settings.launch_cleanup_secs_clamped();
    let mut row = h_flex().gap_2().flex_wrap();
    for (idx, preset) in LAUNCH_CLEANUP_PRESETS.iter().enumerate() {
        let preset_value = *preset;
        let selected = preset_value == current;
        let baseline = settings.clone();
        row = row.child(preset_chip(
            SharedString::from(format!("launch-cleanup-preset-{idx}")),
            SharedString::from(format!("{preset_value} s")),
            selected,
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                shell.update_settings(
                    AppSettings {
                        launch_cleanup_secs: preset_value,
                        ..baseline.clone()
                    },
                    cx,
                );
            }),
        ));
    }
    section_frame(
        "Auto-clean launch payloads",
        "How long the temp file used to launch external apps (e.g. SAP GUI) \
         lives before FerrisPass deletes it. The file is also removed when \
         you lock the vault or quit.",
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

// --------------- Auto-Type tab body ---------------

/// Three hotkey presets shipped with the v1 UI. Custom combos beyond
/// these require editing `settings.json` directly until we ship a
/// proper "press the keys" capture input — adding the InputState
/// plumbing for an interactive combo picker is more work than this
/// scope warrants. The presets all avoid macOS's reserved combos
/// (Spotlight ⌘Space, Mission Control ⌃↑, Dock toggle ⌘⌥D).
const AUTO_TYPE_HOTKEY_PRESETS: &[(&str, &str)] = &[
    ("ctrl+alt+super+KeyV", "⌃⌥⌘V"),
    ("alt+shift+KeyV", "⌥⇧V"),
    ("ctrl+shift+KeyV", "⌃⇧V"),
];

/// One-click templates for the sequence editor. Each is a working
/// example the user can either accept verbatim or use as a starting
/// point for free-form edits in the input below. Order matters —
/// `Default` first because that's what 90% of users want.
const AUTO_TYPE_SEQUENCE_PRESETS: &[(&str, &str)] = &[
    ("Default", "{USERNAME}{TAB}{PASSWORD}{ENTER}"),
    ("Username only", "{USERNAME}{ENTER}"),
    (
        "With 250 ms delay",
        "{USERNAME}{TAB}{DELAY 250}{PASSWORD}{ENTER}",
    ),
    ("No Enter", "{USERNAME}{TAB}{PASSWORD}"),
];

fn auto_type_tab_body(shell: &AppShell, cx: &mut Context<AppShell>) -> impl IntoElement {
    let settings = shell.settings().clone();
    let trusted = shell.auto_type_is_trusted();
    let hotkey_err = shell.auto_type_hotkey_error().map(String::from);
    let sequence_err = shell
        .auto_type_sequence_error()
        .map(|e| format!("Sequence invalid: {e}"));

    v_flex()
        .gap_6()
        .child(auto_type_toggle_section(&settings, cx))
        .child(auto_type_hotkey_section(&settings, hotkey_err, cx))
        .child(auto_type_sequence_section(
            shell,
            &settings,
            sequence_err,
            cx,
        ))
        .child(auto_type_permission_section(trusted, cx))
}

fn auto_type_toggle_section(
    settings: &AppSettings,
    cx: &mut Context<AppShell>,
) -> impl IntoElement {
    let enabled = settings.auto_type_enabled;
    let on_baseline = settings.clone();
    let off_baseline = settings.clone();
    let row = h_flex()
        .gap_2()
        .child(preset_chip(
            "auto-type-on".into(),
            "On".into(),
            enabled,
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                shell.update_settings(
                    AppSettings {
                        auto_type_enabled: true,
                        ..on_baseline.clone()
                    },
                    cx,
                );
            }),
        ))
        .child(preset_chip(
            "auto-type-off".into(),
            "Off".into(),
            !enabled,
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                shell.update_settings(
                    AppSettings {
                        auto_type_enabled: false,
                        ..off_baseline.clone()
                    },
                    cx,
                );
            }),
        ));
    section_frame(
        "Auto-Type",
        "When on, FerrisPass registers a system-wide hotkey. Press it from any window to \
         type the matching entry's credentials into the focused field.",
        row,
    )
}

fn auto_type_hotkey_section(
    settings: &AppSettings,
    error: Option<String>,
    cx: &mut Context<AppShell>,
) -> impl IntoElement {
    let current = settings.auto_type_hotkey.clone();
    let mut row = h_flex().gap_2().flex_wrap();
    for (idx, (combo, label)) in AUTO_TYPE_HOTKEY_PRESETS.iter().enumerate() {
        let selected = current == *combo;
        let baseline = settings.clone();
        let combo_owned = combo.to_string();
        row = row.child(preset_chip(
            SharedString::from(format!("auto-type-hotkey-{idx}")),
            SharedString::from(*label),
            selected,
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                shell.update_settings(
                    AppSettings {
                        auto_type_hotkey: combo_owned.clone(),
                        ..baseline.clone()
                    },
                    cx,
                );
            }),
        ));
    }

    let body = v_flex().gap_2().child(row).when_some(error, |col, err| {
        col.child(
            div()
                .text_xs()
                .text_color(palette::red())
                .child(format!("Hotkey registration failed: {err}")),
        )
    });

    section_frame(
        "Trigger hotkey",
        "Pressed from any app or browser to launch Auto-Type. ⌃⌥⌘V is the KeePassXC \
         default — choose another preset if it conflicts with something else on your Mac.",
        body,
    )
}

fn auto_type_sequence_section(
    shell: &AppShell,
    settings: &AppSettings,
    error: Option<String>,
    cx: &mut Context<AppShell>,
) -> impl IntoElement {
    let current = settings.auto_type_sequence.clone();

    // Preset chips: clicking one sets both the input value and the
    // persisted setting in one shot via `set_auto_type_sequence`. The
    // `selected` flag highlights the chip whose template matches the
    // current value exactly so the user can see what's active.
    let mut presets_row = h_flex().gap_2().flex_wrap();
    for (idx, (label, template)) in AUTO_TYPE_SEQUENCE_PRESETS.iter().enumerate() {
        let selected = current == *template;
        let template_owned = template.to_string();
        presets_row = presets_row.child(preset_chip(
            SharedString::from(format!("auto-type-sequence-preset-{idx}")),
            SharedString::from(*label),
            selected,
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, window, cx| {
                shell.set_auto_type_sequence(&template_owned, window, cx);
            }),
        ));
    }

    // Free-form input — same widget pattern the rest of FerrisPass uses
    // (Input + InputState). The on-change subscription set up in
    // AppShell::new pipes edits straight into settings via update_settings,
    // which in turn re-parses the template and refreshes the error cache
    // (`auto_type_sequence_error`) — so the inline red label below this
    // input updates in real time as the user types.
    let editor = div()
        .w_full()
        .child(gpui_component::input::Input::new(shell.auto_type_sequence_input()).cleanable(true));

    let body = v_flex()
        .gap_3()
        .child(presets_row)
        .child(editor)
        .when_some(error, |col, err| {
            col.child(div().text_xs().text_color(palette::red()).child(err))
        });

    section_frame(
        "Type sequence",
        "Placeholders: {USERNAME}, {PASSWORD}, {TAB}, {ENTER}, {DELAY N} (max 30000 ms). \
         Pick a preset to start, or edit the template below — changes save automatically.",
        body,
    )
}

fn auto_type_permission_section(trusted: bool, cx: &mut Context<AppShell>) -> impl IntoElement {
    let (status_label, status_color) = if trusted {
        (
            SharedString::from("Granted — Auto-Type is ready to use."),
            palette::text(),
        )
    } else {
        (
            SharedString::from(
                "Not granted — Auto-Type cannot type into other apps until you allow access.",
            ),
            palette::text_muted(),
        )
    };
    let body = v_flex()
        .gap_2()
        .child(div().text_xs().text_color(status_color).child(status_label))
        .when(!trusted, |col| {
            col.child(h_flex().gap_2().child(preset_chip(
                "auto-type-grant-access".into(),
                "Grant Accessibility access…".into(),
                true,
                cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
                    shell.auto_type_request_trust();
                    // Re-render so the status line updates after the
                    // system prompt closes. macOS only refreshes the
                    // process trust bit on next launch, so the label
                    // typically still says "Not granted" until the user
                    // restarts — that's why the help text below mentions
                    // restarting.
                    cx.notify();
                }),
            )))
        });

    section_frame(
        "macOS Accessibility access",
        "Both keystroke simulation and reading the foreground window's title use the macOS \
         Accessibility framework. After granting access in System Settings, restart FerrisPass \
         so the new permission takes effect.",
        body,
    )
}

fn updates_section(
    settings: &AppSettings,
    update_status: &UpdateStatus,
    whats_new_available: bool,
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
        UpdateStatus::ReadyToRestart(_) => "Update installed. Restart FerrisPass to apply.".into(),
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

    let action_chip: Option<AnyElement> = match update_status {
        UpdateStatus::Available(_) => Some(
            preset_chip(
                "auto-update-install".into(),
                "Install update".into(),
                true,
                cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                    window.dispatch_action(Box::new(InstallUpdate), cx);
                }),
            )
            .into_any_element(),
        ),
        UpdateStatus::ReadyToRestart(_) => Some(
            preset_chip(
                "auto-update-restart".into(),
                "Restart to Update".into(),
                true,
                cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                    window.dispatch_action(Box::new(RestartToUpdate), cx);
                }),
            )
            .into_any_element(),
        ),
        _ => None,
    };

    let whats_new_chip = whats_new_available.then(|| {
        preset_chip(
            "auto-update-whats-new".into(),
            "View What's New".into(),
            false,
            cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                window.dispatch_action(Box::new(OpenWhatsNew), cx);
            }),
        )
        .into_any_element()
    });

    let action_row = h_flex()
        .gap_2()
        .child(check_now)
        .when_some(action_chip, |row, chip| row.child(chip))
        .when_some(whats_new_chip, |row, chip| row.child(chip));

    let body = v_flex().gap_3().child(toggle_row).child(action_row).child(
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

//! Unified Settings overlay (⌘,). Mac-style: left sidebar with tabs,
//! right content panel. Currently two tabs are wired (General, Sync);
//! the rest are stub placeholders that match the previous mock so the
//! visual hierarchy doesn't change as we fill them in.

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder, px,
};
use gpui_component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};

use crate::app::actions::{DownloadFavicons, InstallUpdate, OpenWhatsNew, RestartToUpdate};
use crate::app::{AppSettings, FaviconDownloadStatus, VaultStatus};
use crate::ui::app_shell::{AppShell, SettingsTab};
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::atoms::{ChipTone, chip};
use crate::ui::widgets::interaction::Interaction as _;
use crate::ui::widgets::settings_form::{
    ActionKind, action_button, option_group, section_card, segment_item, setting_switch,
};
use crate::update::UpdateStatus;

const AUTO_LOCK_PRESETS: &[Option<u64>] = &[Some(60), Some(240), Some(900), None];
const CLIPBOARD_CLEAR_PRESETS: &[Option<u64>] = &[Some(5), Some(10), Some(30), None];
/// Allowed cleanup TTLs for the launch tempfile. Bracketed by
/// `LAUNCH_CLEANUP_SECS_RANGE` (10..=60) so any preset added here
/// stays within the validated band.
const LAUNCH_CLEANUP_PRESETS: &[u32] = &[10, 30, 60];
/// Background auto-sync cadences offered in the UI. `None` = "Never"
/// (auto-sync + token keep-alive off). All `Some` values sit at or above
/// the 60 s floor `AppSettings::auto_sync_secs_clamped` enforces.
const AUTO_SYNC_PRESETS: &[Option<u64>] = &[Some(300), Some(900), Some(1800), None];

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

    // Stub tabs (Security/Backups/Advanced) carry `tab: None` and
    // `enabled: false`. Render a "Soon" pill next to them so users see
    // a roadmap signal instead of unexplained greyed-out rows.
    let is_stub = tab.is_none() && !enabled;
    let row = h_flex()
        .gap_2()
        .items_center()
        .justify_between()
        .h(px(28.))
        .mx(px(6.))
        .px_3p5()
        .rounded(px(5.))
        .bg(bg)
        .text_color(fg)
        .text_sm()
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    gpui_component::Icon::from(icon)
                        .with_size(gpui_component::Size::Size(px(13.)))
                        .text_color(icon_color),
                )
                .child(label),
        )
        .when(is_stub, |row| row.child(chip("Soon", ChipTone::Gray)));

    if let (true, Some(target)) = (enabled, tab) {
        let id = SharedString::from(format!("settings-tab-{label}"));
        div()
            .id(id)
            .pressable_dim()
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
        .min_h(px(0.))
        .h_full()
        .bg(palette::panel())
        .overflow_hidden()
        .child(
            v_flex()
                .flex_shrink_0()
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
                        .child(action_button(
                            "close-settings",
                            "Close",
                            ActionKind::Ghost,
                            true,
                            cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
                                shell.state().clone().update(cx, |state, cx| {
                                    let _ = state.close_overlay(cx);
                                });
                            }),
                        )),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette::text_muted())
                        .child(subtitle),
                ),
        )
        .child(
            // Body is scrollable: the General + Auto-Type tabs already
            // overflow the panel on the default ~720 px window height
            // (Updates section is the casualty), and stub-tab content
            // added later will grow longer too. `min_h(0)` is the usual
            // flexbox shrink fix so this child can actually shrink below
            // its content size; without it the scrollbar never engages.
            v_flex()
                .id("settings-body-scroll")
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .gap_4()
                .p_8()
                .overflow_y_scroll()
                .child(body),
        )
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
    // Touch ID section is only useful on Macs where biometric auth
    // is plausibly reachable; on Linux/Windows the noop backend
    // returns `false` and we hide the whole section so the panel
    // doesn't show a setting the user can't make anything of.
    let biometric_supported = state.biometric_store().is_supported();
    v_flex()
        .gap_4()
        .child(auto_lock_section(&settings, cx))
        .when(biometric_supported, |this| {
            this.child(touch_id_section(&settings, cx))
        })
        .child(clipboard_section(&settings, cx))
        .child(auto_sync_section(&settings, cx))
        .child(launch_cleanup_section(&settings, cx))
        .child(favicon_section(&favicon_status, vault_open, cx))
        .child(updates_section(
            &settings,
            &update_status,
            whats_new_available,
            cx,
        ))
}

fn touch_id_section(settings: &AppSettings, cx: &mut Context<AppShell>) -> impl IntoElement {
    let allow_fallback = settings.biometric_allow_passcode_fallback;
    let baseline = settings.clone();
    let toggle = setting_switch(
        "biometric-passcode-fallback-toggle",
        allow_fallback,
        cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
            shell.update_settings(
                AppSettings {
                    biometric_allow_passcode_fallback: !allow_fallback,
                    ..baseline.clone()
                },
                cx,
            );
        }),
    );
    let toggle_row = h_flex()
        .gap_3()
        .items_center()
        .child(
            div()
                .flex_1()
                .text_xs()
                .text_color(palette::text_muted())
                .child(
                    "Accept your macOS account password as a fallback when Touch ID isn't \
                     reachable (e.g. MacBook lid closed at a docking station).",
                ),
        )
        .child(toggle);

    section_card(
        "Touch ID",
        "Touch ID unlock is opt-in per vault from the unlock screen. \
         With the fallback off, the prompt only accepts Touch ID. \
         With it on, the prompt also accepts your macOS account \
         password — the same surface macOS itself uses for \
         system-level Touch ID dialogs — which is what lets you \
         unlock in clamshell mode where the sensor is unreachable.",
        toggle_row,
    )
}

fn auto_lock_section(settings: &AppSettings, cx: &mut Context<AppShell>) -> impl IntoElement {
    let current = settings.auto_lock_secs;
    let items: Vec<_> = AUTO_LOCK_PRESETS
        .iter()
        .enumerate()
        .map(|(idx, preset)| {
            let preset_value = *preset;
            let baseline = settings.clone();
            segment_item(
                SharedString::from(format!("auto-lock-preset-{idx}")),
                format_auto_lock_label(preset_value),
                preset_value == current,
                cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                    shell.update_settings(
                        AppSettings {
                            auto_lock_secs: preset_value,
                            ..baseline.clone()
                        },
                        cx,
                    );
                }),
            )
        })
        .collect();
    section_card(
        "Auto-lock vault",
        "Lock the vault after this much idle time without keyboard or mouse activity.",
        option_group(items),
    )
}

fn auto_sync_section(settings: &AppSettings, cx: &mut Context<AppShell>) -> impl IntoElement {
    let current = settings.auto_sync_secs_clamped();
    let items: Vec<_> = AUTO_SYNC_PRESETS
        .iter()
        .enumerate()
        .map(|(idx, preset)| {
            let preset_value = *preset;
            let baseline = settings.clone();
            segment_item(
                SharedString::from(format!("auto-sync-preset-{idx}")),
                format_auto_sync_label(preset_value),
                preset_value == current,
                cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                    shell.update_settings(
                        AppSettings {
                            auto_sync_secs: preset_value,
                            ..baseline.clone()
                        },
                        cx,
                    );
                }),
            )
        })
        .collect();
    section_card(
        "Auto-sync with cloud",
        "Check the remote this often and pull in changes from your other \
         devices. This also keeps your Microsoft sign-in alive — leave it on \
         to avoid the periodic reconnect.",
        option_group(items),
    )
}

fn clipboard_section(settings: &AppSettings, cx: &mut Context<AppShell>) -> impl IntoElement {
    let current = settings.clipboard_clear_secs;
    let items: Vec<_> = CLIPBOARD_CLEAR_PRESETS
        .iter()
        .enumerate()
        .map(|(idx, preset)| {
            let preset_value = *preset;
            let baseline = settings.clone();
            segment_item(
                SharedString::from(format!("clipboard-clear-preset-{idx}")),
                format_clipboard_label(preset_value),
                preset_value == current,
                cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                    shell.update_settings(
                        AppSettings {
                            clipboard_clear_secs: preset_value,
                            ..baseline.clone()
                        },
                        cx,
                    );
                }),
            )
        })
        .collect();
    section_card(
        "Clear clipboard after copy",
        "Wipe a copied password / username / TOTP after this many seconds. \
         The clipboard always wipes when you lock the vault.",
        option_group(items),
    )
}

/// "Auto-clean launch payloads after N seconds" picker. Mirrors the
/// shape of the clipboard picker — three preset chips, the selected
/// one highlighted. Range is clamped on read in `AppSettings` so a
/// hand-edited settings file can't push the timer outside 10..=60 s.
fn launch_cleanup_section(settings: &AppSettings, cx: &mut Context<AppShell>) -> impl IntoElement {
    let current = settings.launch_cleanup_secs_clamped();
    let items: Vec<_> = LAUNCH_CLEANUP_PRESETS
        .iter()
        .enumerate()
        .map(|(idx, preset)| {
            let preset_value = *preset;
            let baseline = settings.clone();
            segment_item(
                SharedString::from(format!("launch-cleanup-preset-{idx}")),
                SharedString::from(format!("{preset_value} s")),
                preset_value == current,
                cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                    shell.update_settings(
                        AppSettings {
                            launch_cleanup_secs: preset_value,
                            ..baseline.clone()
                        },
                        cx,
                    );
                }),
            )
        })
        .collect();
    section_card(
        "Auto-clean launch payloads",
        "How long the temp file used to launch external apps (e.g. SAP GUI) \
         lives before FerrisPass deletes it. The file is also removed when \
         you lock the vault or quit.",
        option_group(items),
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

    // Wrap the lone button in a row so it doesn't stretch to the full
    // card width (it sits in a v_flex card body whose default
    // `items: stretch` would otherwise expand it).
    let button = h_flex().self_start().child(action_button(
        "download-favicons",
        label,
        ActionKind::Secondary,
        enabled,
        cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
            window.dispatch_action(Box::new(DownloadFavicons), cx);
        }),
    ));

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

    section_card("Favicons", hint, button)
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
        .gap_4()
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
    let baseline = settings.clone();
    section_card(
        "Auto-Type",
        "When on, FerrisPass registers a system-wide hotkey. Press it from any window to \
         type the matching entry's credentials into the focused field.",
        setting_switch(
            "auto-type-toggle",
            enabled,
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                shell.update_settings(
                    AppSettings {
                        auto_type_enabled: !enabled,
                        ..baseline.clone()
                    },
                    cx,
                );
            }),
        ),
    )
}

fn auto_type_hotkey_section(
    settings: &AppSettings,
    error: Option<String>,
    cx: &mut Context<AppShell>,
) -> impl IntoElement {
    let current = settings.auto_type_hotkey.clone();
    let items: Vec<_> = AUTO_TYPE_HOTKEY_PRESETS
        .iter()
        .enumerate()
        .map(|(idx, (combo, label))| {
            let baseline = settings.clone();
            let combo_owned = combo.to_string();
            segment_item(
                SharedString::from(format!("auto-type-hotkey-{idx}")),
                SharedString::from(*label),
                current == *combo,
                cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                    shell.update_settings(
                        AppSettings {
                            auto_type_hotkey: combo_owned.clone(),
                            ..baseline.clone()
                        },
                        cx,
                    );
                }),
            )
        })
        .collect();

    let body = v_flex()
        .gap_2()
        .child(option_group(items))
        .when_some(error, |col, err| {
            col.child(
                div()
                    .text_xs()
                    .text_color(palette::red())
                    .child(format!("Hotkey registration failed: {err}")),
            )
        });

    section_card(
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

    // Preset segments: clicking one sets both the input value and the
    // persisted setting in one shot via `set_auto_type_sequence`. The
    // `selected` flag highlights the segment whose template matches the
    // current value exactly so the user can see what's active.
    let preset_items: Vec<_> = AUTO_TYPE_SEQUENCE_PRESETS
        .iter()
        .enumerate()
        .map(|(idx, (label, template))| {
            let template_owned = template.to_string();
            segment_item(
                SharedString::from(format!("auto-type-sequence-preset-{idx}")),
                SharedString::from(*label),
                current == *template,
                cx.listener(move |shell: &mut AppShell, _: &ClickEvent, window, cx| {
                    shell.set_auto_type_sequence(&template_owned, window, cx);
                }),
            )
        })
        .collect();
    let presets_row = option_group(preset_items);

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

    section_card(
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
            col.child(h_flex().self_start().child(action_button(
                "auto-type-grant-access",
                "Grant Accessibility access…",
                ActionKind::Primary,
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

    section_card(
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

    // Real toggle switch — same affordance as Auto-Type On/Off.
    let baseline = settings.clone();
    let toggle = setting_switch(
        "auto-update-toggle",
        auto_check,
        cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
            shell.update_settings(
                AppSettings {
                    auto_update_check_enabled: !auto_check,
                    ..baseline.clone()
                },
                cx,
            );
        }),
    );
    let toggle_row = h_flex()
        .gap_3()
        .items_center()
        .child(
            div()
                .flex_1()
                .text_xs()
                .text_color(palette::text_muted())
                .child("Automatically check on launch (once per day)."),
        )
        .child(toggle);

    let check_now = action_button(
        "auto-update-check-now",
        "Check now",
        ActionKind::Secondary,
        true,
        cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
            shell.state().clone().update(cx, |state, cx| {
                state.start_update_check(cx);
            });
        }),
    );

    let action_chip: Option<AnyElement> = match update_status {
        UpdateStatus::Available(_) => Some(action_button(
            "auto-update-install",
            "Install update",
            ActionKind::Primary,
            true,
            cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                window.dispatch_action(Box::new(InstallUpdate), cx);
            }),
        )),
        UpdateStatus::ReadyToRestart(_) => Some(action_button(
            "auto-update-restart",
            "Restart to Update",
            ActionKind::Primary,
            true,
            cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                window.dispatch_action(Box::new(RestartToUpdate), cx);
            }),
        )),
        _ => None,
    };

    let whats_new_chip = whats_new_available.then(|| {
        action_button(
            "auto-update-whats-new",
            "View What's New",
            ActionKind::Secondary,
            true,
            cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                window.dispatch_action(Box::new(OpenWhatsNew), cx);
            }),
        )
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

    section_card(
        "Updates",
        "FerrisPass checks GitHub for new releases on launch (rate-limited \
         to once per day). Update bundles are verified against an embedded \
         Ed25519 public key before install.",
        body,
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

fn format_auto_sync_label(secs: Option<u64>) -> SharedString {
    match secs {
        None => SharedString::from("Never"),
        Some(60) => SharedString::from("1 min"),
        Some(s) if s % 60 == 0 => SharedString::from(format!("{} min", s / 60)),
        Some(s) => SharedString::from(format!("{s} s")),
    }
}

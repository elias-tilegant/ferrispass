//! Sync settings overlay — data-driven from `AppState::sync` +
//! `AppState::sync_status`.
//!
//! Three top-level shapes:
//! - **Connected**: shows account / file / last-sync, with Sync now + Disconnect.
//! - **Reconnect required**: refresh token expired; offers Reconnect button.
//! - **Disconnected**: vault is local-only; offers Connect button (sends
//!   user back through the Connect overlay).
//!
//! The activity log + behavior-toggles sections from the original mockup
//! were removed because they were 100% static demo data — re-introducing
//! them as live data is a future job (would need a sync-event log).

use gpui::{
    div, prelude::FluentBuilder as _, px, AnyElement, ClickEvent, Context, InteractiveElement as _,
    IntoElement as _, ParentElement as _, SharedString, StatefulInteractiveElement as _,
    Styled as _,
};
use gpui_component::{h_flex, v_flex, Sizable as _, WindowExt as _};

use crate::app::actions::OpenConnect;
use crate::app::time::relative_time_label;
use crate::app::{SyncBinding, SyncChangeKind, SyncHistoryEntry, SyncStatus};
use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::atoms::{chip, ChipTone};

/// Render the Sync tab body — content only, no chrome. The unified
/// Settings overlay (`screens::settings`) wraps this with the sidebar
/// and header. The three Connected / Reconnect / Disconnected shapes
/// are picked from `AppState::sync_binding` + `sync_status`.
pub fn render_tab_body(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let state_handle = shell.state().clone();
    let snapshot = state_handle.read(cx);
    let binding = snapshot.sync_binding().cloned_for_render();
    let status = snapshot.sync_status().clone();
    let history: Vec<SyncHistoryEntry> = snapshot.sync_history().to_vec();

    match (&binding, &status) {
        (_, SyncStatus::Reconnect) => render_reconnect(cx),
        (Some(b), _) => render_connected(b, &status, &history, cx),
        (None, _) => render_disconnected(cx),
    }
}

// --------------- bodies ---------------

fn render_connected(
    binding: &BindingSnapshot,
    status: &SyncStatus,
    history: &[SyncHistoryEntry],
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let provider_name = match binding.provider {
        crate::sync::config::SyncProvider::SharePoint => "SharePoint",
    };
    let status_chip = match status {
        SyncStatus::Idle => chip("Idle", ChipTone::Gray),
        SyncStatus::Synced { .. } => chip("Synced", ChipTone::Green),
        SyncStatus::Syncing => chip("Syncing", ChipTone::Blue),
        SyncStatus::Failed(_) => chip("Failed", ChipTone::Orange),
        SyncStatus::Conflict(_) => chip("Conflict", ChipTone::Orange),
        SyncStatus::Connecting | SyncStatus::Restoring => chip("Connecting", ChipTone::Blue),
        SyncStatus::Disconnected | SyncStatus::Reconnect => chip("Off", ChipTone::Gray),
    };
    let last_sync = match status {
        SyncStatus::Synced { at, auto_merged } => {
            let base = format!("Last synced at {}", at.format("%H:%M:%S"));
            if *auto_merged > 0 {
                // Counts both fresh remote-only entries AND existing entries
                // where remote had a strictly newer last_modification (the
                // merge module auto-resolves those). "merged" covers both
                // cases — "pulled in N new entries" was misleading after
                // last-write-wins landed.
                let noun = if *auto_merged == 1 {
                    "entry"
                } else {
                    "entries"
                };
                format!("{base} · merged {auto_merged} {noun} from remote")
            } else {
                base
            }
        }
        SyncStatus::Failed(msg) => format!("Last attempt failed: {msg}"),
        SyncStatus::Syncing => "Syncing now…".into(),
        SyncStatus::Connecting => "Connecting…".into(),
        SyncStatus::Conflict(_) => "Awaiting conflict resolution".into(),
        _ => "—".into(),
    };

    v_flex()
        .gap_4()
        .child(
            v_flex()
                .gap_4()
                .p_4()
                .rounded(px(10.))
                .bg(palette::blue_soft())
                .border_1()
                .border_color(palette::blue_border())
                .child(
                    h_flex()
                        .gap_3p5()
                        .items_center()
                        .child(
                            div()
                                .size(px(44.))
                                .rounded(px(9.))
                                .bg(palette::panel())
                                .border_1()
                                .border_color(palette::blue_border())
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    gpui_component::Icon::from(AppIcon::Cloud)
                                        .with_size(gpui_component::Size::Size(px(22.)))
                                        .text_color(palette::blue()),
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
                                                .child(provider_name),
                                        )
                                        .child(status_chip),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(palette::text_muted())
                                        .font_family("JetBrains Mono")
                                        .child(binding.account_email.clone()),
                                ),
                        )
                        .child(disconnect_button(cx)),
                )
                .child(file_card(binding))
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(palette::text_muted())
                                .child(last_sync),
                        )
                        .child(sync_now_button(cx)),
                ),
        )
        .when(!history.is_empty(), |this| {
            this.child(history_section(history))
        })
        .into_any_element()
}

fn history_section(history: &[SyncHistoryEntry]) -> AnyElement {
    let total = history.len();
    let now = chrono::Local::now();
    // Most recent first — visually matches the "latest at the top" reading
    // order users expect from activity logs.
    let rows: Vec<AnyElement> = history
        .iter()
        .rev()
        .enumerate()
        .map(|(idx, entry)| history_row(idx, entry, now))
        .collect();
    let header_meta: Option<SharedString> = if total > 0 {
        Some(format!("{total} change{}", if total == 1 { "" } else { "s" }).into())
    } else {
        None
    };

    v_flex()
        .gap_2()
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(palette::text_muted())
                        .child("Recent activity"),
                )
                .when_some(header_meta, |this, meta| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(palette::text_faint())
                            .child(meta),
                    )
                }),
        )
        .child(
            v_flex()
                .id("sync-history-list")
                .gap_1()
                .max_h(px(240.))
                .overflow_y_scroll()
                .pr_1()
                .children(rows),
        )
        .into_any_element()
}

fn history_row(
    idx: usize,
    entry: &SyncHistoryEntry,
    now: chrono::DateTime<chrono::Local>,
) -> AnyElement {
    let (dot_color, kind_label) = match entry.kind {
        SyncChangeKind::AddedFromRemote => (palette::green(), "Added"),
        SyncChangeKind::UpdatedFromRemote => (palette::blue(), "Updated"),
        SyncChangeKind::ResolvedKeptRemote => (palette::orange(), "Resolved → remote"),
        SyncChangeKind::ResolvedKeptLocal => (palette::text_faint(), "Resolved → local"),
    };
    let title: SharedString = if entry.entry_title.trim().is_empty() {
        "(Untitled)".into()
    } else {
        entry.entry_title.clone().into()
    };
    let elapsed: SharedString = relative_time_label(entry.at, now).into();
    // GPUI tracks hover state only on *stateful* (id'd) interactive
    // elements — without the id the row repaints only when something
    // else nudges the tree (e.g. a click), which surfaces as
    // "hover only shows up after I click and is laggy".
    let id: SharedString = format!("sync-history-row-{idx}").into();

    h_flex()
        .id(id)
        .gap_2p5()
        .items_center()
        .h(px(28.))
        .px_2()
        .rounded(px(5.))
        .hover(|s| s.bg(palette::sidebar()))
        .child(
            div()
                .flex_shrink_0()
                .size(px(6.))
                .rounded_full()
                .bg(dot_color),
        )
        .child(
            div()
                .flex_shrink_0()
                .w(px(120.))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(palette::text_muted())
                .child(kind_label),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_xs()
                .text_color(palette::text())
                .font_family("JetBrains Mono")
                .child(title),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(palette::text_faint())
                .child(elapsed),
        )
        .into_any_element()
}

fn file_card(binding: &BindingSnapshot) -> AnyElement {
    h_flex()
        .gap_3()
        .items_center()
        .p_3()
        .rounded(px(7.))
        .bg(palette::panel())
        .border_1()
        .border_color(palette::blue_border())
        .child(
            div()
                .size(px(32.))
                .rounded(px(6.))
                .bg(palette::orange_soft())
                .flex()
                .items_center()
                .justify_center()
                .child(
                    gpui_component::Icon::from(AppIcon::Key)
                        .with_size(gpui_component::Size::Size(px(15.)))
                        .text_color(palette::orange()),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w(px(0.))
                .gap_0p5()
                .child(
                    div()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .font_family("JetBrains Mono")
                        .truncate()
                        .child(binding.local_path_display.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette::text_muted())
                        .truncate()
                        .child(binding.remote_url.clone()),
                ),
        )
        .into_any_element()
}

fn render_disconnected(cx: &mut Context<AppShell>) -> AnyElement {
    v_flex()
        .gap_4()
        .child(
            div()
                .p_4()
                .rounded(px(10.))
                .bg(palette::sidebar())
                .border_1()
                .border_color(palette::border())
                .child(
                    div()
                        .text_sm()
                        .text_color(palette::text())
                        .child("This vault is local-only — no cloud sync configured."),
                ),
        )
        .child(
            div()
                .id("connect-button")
                .h(px(32.))
                .px(px(14.))
                .rounded(px(6.))
                .bg(palette::blue())
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(palette::panel())
                .flex()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    gpui_component::Icon::from(AppIcon::Cloud)
                        .with_size(gpui_component::Size::Size(px(13.)))
                        .text_color(palette::panel()),
                )
                .child("Connect to SharePoint")
                .on_click(cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                    window.dispatch_action(Box::new(OpenConnect), cx);
                })),
        )
        .into_any_element()
}

fn render_reconnect(cx: &mut Context<AppShell>) -> AnyElement {
    v_flex()
        .gap_4()
        .child(
            div()
                .p_4()
                .rounded(px(10.))
                .bg(palette::orange_soft())
                .border_1()
                .border_color(palette::orange_border())
                .child(
                    div()
                        .text_sm()
                        .text_color(palette::text())
                        .child("Your Microsoft sign-in has expired — reconnect to keep syncing."),
                ),
        )
        .child(
            div()
                .id("reconnect-button")
                .h(px(32.))
                .px(px(14.))
                .rounded(px(6.))
                .bg(palette::blue())
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(palette::panel())
                .flex()
                .items_center()
                .justify_center()
                .child("Reconnect")
                .on_click(cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                    window.dispatch_action(Box::new(OpenConnect), cx);
                })),
        )
        .into_any_element()
}

// --------------- buttons ---------------

fn sync_now_button(cx: &mut Context<AppShell>) -> AnyElement {
    div()
        .id("sync-now")
        .h(px(28.))
        .px(px(12.))
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
        .gap_1p5()
        .child(
            gpui_component::Icon::from(AppIcon::Sync)
                .with_size(gpui_component::Size::Size(px(11.)))
                .text_color(palette::text()),
        )
        .child("Sync now")
        .on_click(cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
            shell
                .state()
                .clone()
                .update(cx, |state, cx| state.sync_now(cx));
        }))
        .into_any_element()
}

fn disconnect_button(cx: &mut Context<AppShell>) -> AnyElement {
    div()
        .id("disconnect-button")
        .h(px(28.))
        .px(px(10.))
        .rounded(px(6.))
        .bg(palette::panel())
        .border_1()
        .border_color(palette::border_strong())
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(palette::text())
        .flex()
        .items_center()
        .justify_center()
        .child("Disconnect")
        .on_click(
            cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
                shell.state().clone().update(cx, |state, cx| {
                    state.disconnect_sync(cx);
                    let _ = state.close_overlay(cx);
                });
                window.push_notification("Cloud sync disconnected.", cx);
            }),
        )
        .into_any_element()
}

// --------------- helpers ---------------

/// Plain-old-data snapshot of SyncBinding for renderers — `AppState::sync`
/// is held as `Option<SyncBinding>` and `SyncBinding` isn't `Clone` (it
/// owns an `AccessToken` which we deliberately keep non-Clone). This
/// snapshot only carries the bits the UI displays.
struct BindingSnapshot {
    provider: crate::sync::config::SyncProvider,
    account_email: String,
    local_path_display: String,
    remote_url: String,
}

trait BindingForRender {
    fn cloned_for_render(self) -> Option<BindingSnapshot>;
}
impl BindingForRender for Option<&SyncBinding> {
    fn cloned_for_render(self) -> Option<BindingSnapshot> {
        self.map(|b| BindingSnapshot {
            provider: b.config.provider,
            account_email: b.config.account_email.clone(),
            local_path_display: b.config.local_path.display().to_string(),
            remote_url: b.config.remote_url.clone(),
        })
    }
}

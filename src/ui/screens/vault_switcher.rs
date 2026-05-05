//! Vault switcher overlay — a centred modal with a filter input + the
//! recents list + a "Browse other vault…" fallback row. Reachable via
//! ⌘O from any vault state and via clicking the sidebar header inside a
//! vault. The filter narrows the recents list in-place; Enter on the
//! input opens whichever row is currently on top.
//!
//! Selection: simple click model. Keyboard navigation beyond Enter
//! (Up/Down to highlight a specific row) is intentionally deferred —
//! Enter on the top match covers the typical "type a few letters →
//! switch" motion for any reasonable recents list size.

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
    px,
};
use gpui_component::{ActiveTheme as _, Sizable as _, h_flex, input::Input, v_flex};

use crate::app::RecentEntry;
use crate::app::actions::OpenVault;
use crate::app::time::relative_time_label;
use crate::ui::app_shell::{AppShell, filter_recents};
use crate::ui::icons::AppIcon;
use crate::ui::palette;

pub fn render(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    // Clone the theme handle up front so the renderer doesn't keep `cx`
    // borrowed across the row-building closure that needs `cx.listener`.
    let bg = cx.theme().background;
    let state = shell.state().read(cx);
    let recents: Vec<RecentEntry> = state.recents().to_vec();
    let current_path = state.current_vault_path();
    let query = shell.vault_switcher_input().read(cx).value().to_string();

    let filtered: Vec<&RecentEntry> = filter_recents(&recents, &query, current_path.as_deref());
    let now = chrono::Local::now();

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
                .child("Pick a recent vault, or browse for a different .kdbx file."),
        );

    let filter_field = div()
        .w_full()
        .child(Input::new(shell.vault_switcher_input()).cleanable(true));

    let list_body = if filtered.is_empty() {
        empty_state(query.trim().is_empty()).into_any_element()
    } else {
        let rows: Vec<AnyElement> = filtered
            .iter()
            .enumerate()
            .map(|(idx, entry)| recent_row(idx, entry, now, cx).into_any_element())
            .collect();
        v_flex().gap_1p5().children(rows).into_any_element()
    };

    let browse_row = div()
        .id("vault-switcher-browse")
        .on_click(
            cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
                // Close the overlay first so the system file picker isn't
                // racing the still-mounted modal for the focused window.
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
                .when(idx == 0, |this| {
                    // The top row is what Enter activates — give it the
                    // same treatment as a focused button so users can see
                    // that.
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
        "No recent vaults yet. Browse to add one below."
    } else {
        "No recents match the filter. Press Enter to browse for one."
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

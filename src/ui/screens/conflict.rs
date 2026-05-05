//! Conflict-resolution overlay. Reads `SyncStatus::Conflict(state)` and
//! renders one side-by-side column pair per `EntryConflict`. The user
//! picks Local or Remote per entry; "Apply resolution" sends the merged
//! file back to SharePoint.
//!
//! Bottom-of-screen affordances stay sticky (sticky footer pattern from
//! AddEntry) so even with many conflicts the Apply button is always
//! reachable.

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, hsla, prelude::FluentBuilder as _, px,
};
use gpui_component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};

use crate::app::SyncStatus;
use crate::keepass::merge::{EntryConflict, EntryView, FieldDiff, Side};
use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::atoms::{ChipTone, chip};

pub fn render(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    // Snapshot the conflict so we don't hold the AppState borrow across the
    // closures the buttons need.
    let snapshot = match shell.state().read(cx).sync_status() {
        SyncStatus::Conflict(state) => Some(ConflictSnapshot {
            conflicts: state.report.conflicts.clone(),
            local_only_count: state.report.local_only.len(),
            remote_only_count: state.report.remote_only.len(),
            picks: state.picks.clone(),
        }),
        _ => None,
    };

    let Some(snapshot) = snapshot else {
        // Fallback: shouldn't be visible if SyncStatus isn't Conflict, but
        // render an empty body rather than panicking if we get here mid-
        // transition (e.g., a successful Apply has cleared the status).
        return v_flex()
            .size_full()
            .bg(cx.theme().background)
            .into_any_element();
    };

    let header = header(&snapshot, cx);
    let body = body(snapshot, cx);

    v_flex()
        .size_full()
        .bg(cx.theme().background)
        .child(header)
        .child(body)
        .into_any_element()
}

fn header(snapshot: &ConflictSnapshot, cx: &mut Context<AppShell>) -> AnyElement {
    let n = snapshot.conflicts.len();
    let title = if n == 1 {
        "Sync conflict on 1 entry".to_string()
    } else {
        format!("Sync conflict on {n} entries")
    };
    let mut subtitle_parts = Vec::new();
    if snapshot.local_only_count > 0 {
        subtitle_parts.push(format!(
            "{} local-only auto-merged",
            snapshot.local_only_count
        ));
    }
    if snapshot.remote_only_count > 0 {
        subtitle_parts.push(format!(
            "{} remote-only auto-merged",
            snapshot.remote_only_count
        ));
    }
    let subtitle = if subtitle_parts.is_empty() {
        "Pick a version per entry. Apply when you're ready.".to_string()
    } else {
        format!("Pick a version per entry. {}.", subtitle_parts.join(", "))
    };

    h_flex()
        .gap_3()
        .items_center()
        .px_6()
        .py_3p5()
        .border_b_1()
        .border_color(palette::border())
        .bg(palette::orange_soft())
        .child(
            div()
                .size(px(32.))
                .rounded(px(7.))
                .bg(palette::panel())
                .border_1()
                .border_color(palette::orange_border())
                .flex()
                .items_center()
                .justify_center()
                .child(
                    gpui_component::Icon::from(AppIcon::Sync)
                        .with_size(gpui_component::Size::Size(px(16.)))
                        .text_color(palette::orange()),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette::text_muted())
                        .child(subtitle),
                ),
        )
        .child(cancel_button(cx))
        .child(apply_button(cx))
        .into_any_element()
}

fn body(snapshot: ConflictSnapshot, cx: &mut Context<AppShell>) -> AnyElement {
    if snapshot.conflicts.is_empty() {
        // Edge case: report is non-empty (remote_only > 0) but no per-entry
        // conflicts. Just show a "ready to merge" prompt + the auto-merge
        // counts. User clicks Apply to commit.
        return div()
            .flex_1()
            .min_h(px(0.))
            .p_8()
            .child(
                div()
                    .p_6()
                    .rounded(px(10.))
                    .bg(palette::sidebar())
                    .border_1()
                    .border_color(palette::border())
                    .text_sm()
                    .text_color(palette::text())
                    .child("No per-entry conflicts. Click Apply to push the auto-merged result."),
            )
            .into_any_element();
    }

    let mut col = v_flex()
        .id("conflict-scroll")
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .gap_5()
        .p_6();

    for conflict in snapshot.conflicts {
        let pick = snapshot
            .picks
            .get(&conflict.id)
            .copied()
            .unwrap_or(Side::Local);
        col = col.child(conflict_block(conflict, pick, cx));
    }

    col.into_any_element()
}

fn conflict_block(conflict: EntryConflict, pick: Side, cx: &mut Context<AppShell>) -> AnyElement {
    h_flex()
        .gap_3p5()
        .child(column(
            "This Mac",
            "Local",
            &conflict.local,
            &conflict.fields,
            pick == Side::Local,
            conflict.id.clone(),
            Side::Local,
            cx,
        ))
        .child(column(
            "SharePoint",
            "Remote",
            &conflict.remote,
            &conflict.fields,
            pick == Side::Remote,
            conflict.id.clone(),
            Side::Remote,
            cx,
        ))
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn column(
    title: &'static str,
    device: &'static str,
    view: &EntryView,
    fields: &[FieldDiff],
    selected: bool,
    entry_id: String,
    side: Side,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let header_bg = if selected {
        palette::blue_soft()
    } else {
        palette::sidebar()
    };
    let border = if selected {
        palette::blue()
    } else {
        palette::border()
    };
    let highlight_bg = hsla(0.072_464, 0.851_852, 0.97, 1.0);
    let modified = view
        .modified
        .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "—".into());
    let entry_id_for_click = entry_id;

    let pick_chip = if selected {
        chip("Keeping", ChipTone::Blue)
    } else {
        chip("Keep this", ChipTone::Gray)
    };

    let mut col = v_flex()
        .flex_1()
        .id(("conflict-col", side as u8 as u32))
        .rounded(px(10.))
        .border_1()
        .border_color(border)
        .bg(palette::panel())
        .overflow_hidden()
        .child(
            h_flex()
                .gap_2p5()
                .items_center()
                .p_3()
                .border_b_1()
                .border_color(palette::border())
                .bg(header_bg)
                .child(
                    div()
                        .size(px(24.))
                        .rounded(px(5.))
                        .bg(if selected {
                            palette::blue()
                        } else {
                            palette::panel()
                        })
                        .border_1()
                        .border_color(if selected {
                            palette::blue()
                        } else {
                            palette::border()
                        })
                        .text_color(if selected {
                            palette::panel()
                        } else {
                            palette::text_muted()
                        })
                        .text_xs()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(if selected { "✓" } else { "☁" }),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette::text_muted())
                                .font_family("JetBrains Mono")
                                .child(format!("{device} · {modified}")),
                        ),
                )
                .child(pick_chip),
        );

    // Click anywhere on the header → make this the picked side. Wrapping
    // the whole column with on_click is also tempting, but field-row clicks
    // would then accidentally toggle the pick. Header-only is safer.
    col = col.on_click(
        cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
            shell.state().clone().update(cx, |state, cx| {
                state.set_conflict_pick(&entry_id_for_click, side, cx);
            });
        }),
    );

    for (i, f) in fields.iter().enumerate() {
        let last = i == fields.len() - 1;
        let value = match f.label {
            "Title" => &view.title,
            "Username" => &view.username,
            "Password" => match side {
                Side::Local => &f.local,
                Side::Remote => &f.remote,
            },
            "URL" => &view.url,
            "Notes" => &view.notes,
            _ => "",
        };
        let chip_el = if f.differs {
            Some(chip("Differs", ChipTone::Orange))
        } else if !value.is_empty() {
            Some(chip("Same", ChipTone::Gray))
        } else {
            None
        };
        col = col.child(
            v_flex()
                .gap_1()
                .p_3()
                .when(!last, |this| {
                    this.border_b_1().border_color(palette::border())
                })
                .when(f.differs, |this| this.bg(highlight_bg))
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(palette::text_faint())
                                .child(f.label),
                        )
                        .when_some(chip_el, |this, c| this.child(c)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette::text())
                        .font_family(if f.label == "Notes" {
                            ""
                        } else {
                            "JetBrains Mono"
                        })
                        .child(value.to_string()),
                ),
        );
    }

    col.into_any_element()
}

fn apply_button(cx: &mut Context<AppShell>) -> AnyElement {
    div()
        .id("apply-resolution")
        .h(px(30.))
        .px(px(14.))
        .rounded(px(6.))
        .bg(palette::blue())
        .border_1()
        .border_color(palette::blue_hover())
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(palette::panel())
        .flex()
        .items_center()
        .justify_center()
        .gap_1p5()
        .child(
            gpui_component::Icon::from(gpui_component::IconName::Check)
                .with_size(gpui_component::Size::Size(px(13.)))
                .text_color(palette::panel()),
        )
        .child("Apply resolution")
        .on_click(
            cx.listener(|shell: &mut AppShell, _: &ClickEvent, _window, cx| {
                shell
                    .state()
                    .clone()
                    .update(cx, |state, cx| state.apply_conflict_resolution(cx));
            }),
        )
        .into_any_element()
}

fn cancel_button(cx: &mut Context<AppShell>) -> AnyElement {
    div()
        .id("conflict-cancel")
        .h(px(30.))
        .px(px(12.))
        .rounded(px(6.))
        .bg(palette::panel())
        .border_1()
        .border_color(palette::border_strong())
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(palette::text())
        .flex()
        .items_center()
        .justify_center()
        .child("Cancel")
        .on_click(
            cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
                shell.state().clone().update(cx, |state, cx| {
                    let _ = state.close_overlay(cx);
                });
                window.push_notification(
                    "Conflict left pending — click Sync now in Sync settings to retry.",
                    cx,
                );
            }),
        )
        .into_any_element()
}

/// Cheap clone of the conflict shape — separates the borrow lifetime from
/// the `Context<AppShell>` we hand to button listeners.
struct ConflictSnapshot {
    conflicts: Vec<EntryConflict>,
    local_only_count: usize,
    remote_only_count: usize,
    picks: std::collections::HashMap<String, Side>,
}

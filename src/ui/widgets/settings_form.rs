//! Form primitives used exclusively by the Settings overlay.
//!
//! Replaces the previous in-file `preset_chip` / `section_frame` helpers
//! in `screens::settings` so that toggle / option-pick / action affordances
//! each get their own visual identity instead of all rendering as
//! identical chips.

use gpui::{
    AnyElement, App, ClickEvent, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    px,
};
use gpui_component::{h_flex, v_flex};

use crate::ui::palette;
use crate::ui::widgets::toggle_row::switch_visual;

/// Baseline height for interactive controls in the Settings overlay.
/// Buttons, segmented-control segments and sidebar items all align to it.
pub const CONTROL_H: gpui::Pixels = px(28.);

/// Boxed click listener. We use a trait-object alias here so the public
/// builder APIs (`segment_item`, `action_button`) can stay non-generic and
/// callers can stuff `cx.listener(...)` results into a `Vec`.
pub type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// One segment of an `option_group`. Use `segment_item` to construct.
pub struct SegmentItem {
    id: SharedString,
    label: SharedString,
    selected: bool,
    on_click: ClickHandler,
}

pub fn segment_item<F>(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    selected: bool,
    on_click: F,
) -> SegmentItem
where
    F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    SegmentItem {
        id: id.into(),
        label: label.into(),
        selected,
        on_click: Box::new(on_click),
    }
}

/// Segmented control — one of N options is highlighted. Replaces the
/// 4-loose-chips pattern used for time presets, hotkey presets, sequence
/// presets etc. Renders as a single rounded container with shared
/// borders so it reads as an interconnected control, not a stack of
/// independent buttons.
pub fn option_group(items: Vec<SegmentItem>) -> AnyElement {
    let count = items.len();
    // `self_start()` opts the row out of its parent's `items: stretch` —
    // without it, sitting inside a `v_flex` body the bordered container
    // expands to the full card width and the segments cluster on the
    // left with a wide empty bar to their right.
    let mut row = h_flex()
        .self_start()
        .h(CONTROL_H)
        .rounded(px(6.))
        .border_1()
        .border_color(palette::border_strong())
        .bg(palette::panel())
        .overflow_hidden();

    for (idx, item) in items.into_iter().enumerate() {
        let SegmentItem {
            id,
            label,
            selected,
            on_click,
        } = item;
        let is_last = idx + 1 == count;
        let (bg, fg) = if selected {
            (palette::blue(), palette::panel())
        } else {
            (palette::panel(), palette::text())
        };
        let mut seg = div()
            .id(id)
            .h_full()
            .px(px(14.))
            .flex()
            .items_center()
            .justify_center()
            .bg(bg)
            .text_color(fg)
            .text_xs()
            .font_weight(gpui::FontWeight::MEDIUM)
            .cursor_pointer()
            .hover(|s| s.opacity(0.85))
            .on_click(on_click)
            .child(label);
        if !is_last {
            // 1-px divider between segments — only on the right side so the
            // outer container's border owns the outside edges.
            seg = seg.border_r_1().border_color(palette::border_strong());
        }
        row = row.child(seg);
    }

    row.into_any_element()
}

/// Card-shaped section container. Adds subtle bg + border so the General
/// tab gets the same visual grouping the Sync tab already has.
pub fn section_card(
    title: &'static str,
    description: &'static str,
    body: impl IntoElement,
) -> AnyElement {
    v_flex()
        .gap_3()
        .p_4()
        .rounded(px(10.))
        .border_1()
        .border_color(palette::border())
        .bg(palette::panel())
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(palette::text())
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette::text_muted())
                        .child(description),
                ),
        )
        .child(body.into_any_element())
        .into_any_element()
}

/// Real switch control — visual slider rather than the prior two-chip
/// "On / Off" hack. Reuses `toggle_row::switch_visual` so the switch
/// look-and-feel is identical to the one in entry-detail toggles.
pub fn setting_switch<F>(id: impl Into<ElementId>, on: bool, on_click: F) -> AnyElement
where
    F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    div()
        .id(id.into())
        .cursor_pointer()
        .hover(|s| s.opacity(0.85))
        .on_click(on_click)
        .child(switch_visual(on))
        .into_any_element()
}

/// Visual flavour for `action_button`. The flat `preset_chip(selected=true)`
/// look used for "Install update" and "Restart" maps to `Primary`; the
/// soft-blue "Download favicons" look maps to `Secondary`; the bordered
/// "Close" button maps to `Ghost`.
#[derive(Clone, Copy)]
pub enum ActionKind {
    Primary,
    Secondary,
    Ghost,
}

/// Single unified action button. Consolidates the half-dozen ad-hoc
/// button styles that previously lived inline in `screens::settings`
/// (and the action-shaped `preset_chip` calls).
///
/// `enabled = false` mutes the colours and skips wiring the click
/// handler — callers can render a disabled state without an `Option`
/// dance.
pub fn action_button<F>(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    kind: ActionKind,
    enabled: bool,
    on_click: F,
) -> AnyElement
where
    F: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let label = label.into();
    let (bg, fg, border) = match (kind, enabled) {
        (ActionKind::Primary, true) => (palette::blue(), palette::panel(), palette::blue_hover()),
        (ActionKind::Primary, false) => (
            palette::sidebar(),
            palette::text_faint(),
            palette::border(),
        ),
        (ActionKind::Secondary, true) => (
            palette::blue_soft(),
            palette::blue(),
            palette::blue_border(),
        ),
        (ActionKind::Secondary, false) => (
            palette::sidebar(),
            palette::text_muted(),
            palette::border(),
        ),
        (ActionKind::Ghost, true) => (palette::panel(), palette::text(), palette::border_strong()),
        (ActionKind::Ghost, false) => (
            palette::panel(),
            palette::text_faint(),
            palette::border(),
        ),
    };

    // Note on sizing: callers placing `action_button` as the sole body of
    // a `section_card` (i.e. inside a `v_flex` whose default `items:
    // stretch` would expand the button to the card's width) should wrap
    // the result in `h_flex().self_start().child(...)`. We don't apply
    // `self_start` here because the close-button uses `action_button`
    // inside a header `h_flex` where `self_start` would override the
    // parent's vertical centering.
    let mut button = div()
        .id(id.into())
        .h(CONTROL_H)
        .px(px(12.))
        .rounded(px(6.))
        .border_1()
        .border_color(border)
        .bg(bg)
        .text_color(fg)
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .flex()
        .items_center()
        .justify_center()
        .child(label);

    if enabled {
        button = button
            .cursor_pointer()
            .hover(|s| s.opacity(0.85))
            .on_click(on_click);
    } else {
        // Make sure the listener gets dropped — important when callers
        // wrap a heavy closure (settings state etc.). Discarding it here
        // is the simplest way to signal "no, really, don't fire".
        let _ = on_click;
    }

    button.into_any_element()
}

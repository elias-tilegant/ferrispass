//! Add-vault command modal. This is intentionally separate from the
//! vault switcher: switching chooses between known/unlocked vaults,
//! adding opens a new local or SharePoint-backed vault.

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::{h_flex, v_flex};

use crate::app::actions::{AddSharePointVault, OpenVault};
use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::command_row::{RowTone, command_row};

pub fn render(cx: &mut Context<AppShell>) -> AnyElement {
    div()
        .id("add-vault-backdrop")
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .left_0()
        .bg(palette::transparent_overlay())
        .occlude()
        .on_click(cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
            shell
                .state()
                .clone()
                .update(cx, |state, cx| state.close_overlay(cx));
        }))
        .flex()
        .items_start()
        .justify_center()
        .pt(px(92.))
        .px(px(20.))
        .child(
            v_flex()
                .id("add-vault-panel")
                .w(px(520.))
                .rounded(px(10.))
                .bg(palette::panel())
                .border_1()
                .border_color(palette::border_strong())
                .overflow_hidden()
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(header())
                .child(
                    v_flex()
                        .gap_2()
                        .px_4()
                        .pb_4()
                        .child(sharepoint_row(cx))
                        .child(local_row(cx)),
                )
                .child(footer()),
        )
        .into_any_element()
}

fn header() -> AnyElement {
    v_flex()
        .gap_0p5()
        .p_4()
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(palette::text())
                .child("Open another vault"),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette::text_muted())
                .child("Choose where the separate vault should come from."),
        )
        .into_any_element()
}

fn sharepoint_row(cx: &mut Context<AppShell>) -> AnyElement {
    command_row(
        "add-vault-sharepoint",
        AppIcon::Cloud,
        "From SharePoint...",
        "Download a separate synced .kdbx vault",
        RowTone::Primary,
        Some("Connect".into()),
        cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
            shell
                .state()
                .clone()
                .update(cx, |state, cx| state.close_overlay(cx));
            window.dispatch_action(Box::new(AddSharePointVault), cx);
        }),
    )
}

fn local_row(cx: &mut Context<AppShell>) -> AnyElement {
    command_row(
        "add-vault-local",
        AppIcon::Key,
        "From this Mac...",
        "Open another local .kdbx file",
        RowTone::Default,
        Some("Browse".into()),
        cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
            shell
                .state()
                .clone()
                .update(cx, |state, cx| state.close_overlay(cx));
            window.dispatch_action(Box::new(OpenVault), cx);
        }),
    )
}

fn footer() -> AnyElement {
    h_flex()
        .justify_end()
        .border_t_1()
        .border_color(palette::border())
        .px_4()
        .py_3()
        .text_xs()
        .text_color(palette::text_faint())
        .child("Esc to cancel")
        .into_any_element()
}

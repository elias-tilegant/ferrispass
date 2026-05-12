//! "About FerrisPass" modal — small dialog with brand, version and a
//! repo link. Reachable from the macOS menu bar (FerrisPass → About
//! FerrisPass) and from the Welcome footer by clicking the version
//! string. The `OpenAbout` action can be dispatched from anywhere so
//! adding further entry points is trivial.

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, px,
};
use gpui_component::v_flex;

use crate::ui::app_shell::AppShell;
use crate::ui::palette;
use crate::ui::widgets::brand::brand;

const REPO_URL: &str = "https://github.com/elias-tilegant/ferrispass";

pub fn render(_shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    div()
        .id("about-backdrop")
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .left_0()
        .bg(palette::transparent_overlay())
        .occlude()
        .on_click(cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
            shell.state().clone().update(cx, |state, cx| {
                let _ = state.close_overlay(cx);
            });
        }))
        .flex()
        .items_center()
        .justify_center()
        .p_6()
        .child(
            v_flex()
                .id("about-panel")
                .w(px(360.))
                .rounded(px(10.))
                .bg(palette::panel())
                .border_1()
                .border_color(palette::border_strong())
                .shadow_lg()
                .overflow_hidden()
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(body(cx)),
        )
        .into_any_element()
}

fn body(cx: &mut Context<AppShell>) -> AnyElement {
    v_flex()
        .items_center()
        .gap_3()
        .px_6()
        .py_7()
        .child(brand(56.))
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(palette::text())
                .child("FerrisPass"),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette::text_muted())
                .child(SharedString::from(format!(
                    "Version {}",
                    crate::app::APP_VERSION
                ))),
        )
        .child(
            div()
                .pt_1()
                .text_xs()
                .text_color(palette::text_muted())
                .child("A native, Rust-built password manager."),
        )
        .child(repo_link(cx))
        .child(
            div()
                .pt_2()
                .text_xs()
                .text_color(palette::text_faint())
                .child("© 2026 FerrisPass · GPL-3.0-or-later"),
        )
        .into_any_element()
}

fn repo_link(cx: &mut Context<AppShell>) -> AnyElement {
    div()
        .id("about-repo-link")
        .text_xs()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(palette::blue())
        .hover(|s| s.text_color(palette::blue_hover()))
        .cursor_pointer()
        .child("View on GitHub")
        .on_click(cx.listener(|_: &mut AppShell, _: &ClickEvent, _, cx| {
            cx.open_url(REPO_URL);
        }))
        .into_any_element()
}

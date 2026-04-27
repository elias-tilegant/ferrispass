use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement as _,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};

use crate::app::actions::{CreateVault, OpenConnect};
use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::brand::brand;

pub fn render(_shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let theme = cx.theme();
    let bg_overlay = format!(
        "background:radial-gradient(ellipse at 30% 20%,{} 0%, transparent 60%)," ,
        css_color(palette::BLUE_SOFT)
    );
    let _ = bg_overlay;

    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.background)
        .child(
            v_flex()
                .w(px(460.))
                .p_10()
                .bg(palette::PANEL)
                .border_1()
                .border_color(palette::BORDER)
                .rounded(px(12.))
                .gap_5()
                .child(brand(48.))
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_xl()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child("Welcome to KeePass RS"),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(palette::TEXT_MUTED)
                                .child(
                                    "A native, Rust-built password manager. Your vault is encrypted locally and synced through your own cloud.",
                                ),
                        ),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(choice_row(
                            "welcome-open",
                            AppIcon::Note,
                            "Open existing database",
                            ".kdbx file from disk",
                            false,
                            cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
                                shell.prompt_for_vault_path(window, cx);
                            }),
                        ))
                        .child(choice_row(
                            "welcome-cloud",
                            AppIcon::Cloud,
                            "Connect OneDrive",
                            "Sync an existing vault from the cloud",
                            true,
                            cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                                window.dispatch_action(Box::new(OpenConnect), cx);
                            }),
                        ))
                        .child(choice_row(
                            "welcome-create",
                            AppIcon::Key,
                            "Create new database",
                            "Start fresh with a new .kdbx vault",
                            false,
                            cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                                window.dispatch_action(Box::new(CreateVault), cx);
                            }),
                        )),
                )
                .child(
                    div()
                        .pt_4()
                        .border_t_1()
                        .border_color(palette::BORDER)
                        .child(
                            h_flex()
                                .items_center()
                                .justify_between()
                                .text_xs()
                                .text_color(palette::TEXT_FAINT)
                                .child("v0.4.2 · KDBX 4.1")
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .items_center()
                                        .child(
                                            gpui_component::Icon::from(AppIcon::Shield)
                                                .with_size(gpui_component::Size::Size(px(12.)))
                                                .text_color(palette::TEXT_FAINT),
                                        )
                                        .child("AES-256 · Argon2id"),
                                ),
                        ),
                ),
        )
        .into_any_element()
}

fn choice_row<F>(
    id: &'static str,
    icon: AppIcon,
    title: &'static str,
    meta: &'static str,
    accent: bool,
    on_click: F,
) -> impl gpui::IntoElement
where
    F: Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
{
    let bg = if accent { palette::BLUE_SOFT } else { palette::SIDEBAR };
    let border = if accent { palette::BLUE_BORDER } else { palette::BORDER };
    let icon_bg = if accent { palette::BLUE } else { palette::PANEL };
    let icon_color = if accent { palette::PANEL } else { palette::TEXT };
    let title_color = if accent { palette::BLUE_HOVER } else { palette::TEXT };

    div()
        .id(id)
        .on_click(on_click)
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .p_3()
                .rounded(px(8.))
                .bg(bg)
                .border_1()
                .border_color(border)
                .child(
                    div()
                        .size(px(32.))
                        .rounded(px(7.))
                        .bg(icon_bg)
                        .text_color(icon_color)
                        .border_1()
                        .border_color(if accent { palette::BLUE } else { palette::BORDER })
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            gpui_component::Icon::from(icon)
                                .with_size(gpui_component::Size::Size(px(15.))),
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
                                .text_color(title_color)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette::TEXT_MUTED)
                                .child(meta),
                        ),
                )
                .child(
                    gpui_component::Icon::from(gpui_component::IconName::ChevronRight)
                        .with_size(gpui_component::Size::Size(px(14.)))
                        .text_color(palette::TEXT_FAINT),
                ),
        )
}

fn css_color(c: gpui::Hsla) -> String {
    let r = (c.l * 255.0) as u8;
    format!("rgb({r},{r},{r})")
}

use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement as _,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, div, hsla, px,
};
use gpui_component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};

use crate::app::actions::CancelUnlock;
use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::buttons::step_indicator;
use crate::ui::widgets::provider_row::{Provider, provider_row};

pub fn render(_shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .p_10()
        .bg(cx.theme().background)
        .child(
            v_flex()
                .w(px(600.))
                .bg(palette::panel())
                .border_1()
                .border_color(palette::border())
                .rounded(px(12.))
                .overflow_hidden()
                .child(
                    v_flex()
                        .gap_4()
                        .px_8()
                        .pt_7()
                        .pb_5()
                        .child(step_indicator(
                            &[(1, "Choose provider"), (2, "Authorize"), (3, "Pick vault")],
                            1,
                            cx,
                        ))
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xl()
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .child("Where should KeePass RS sync your vault?"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(palette::text_muted())
                                        .child(
                                            "Your .kdbx file is encrypted on this device before it ever leaves. The provider only sees ciphertext.",
                                        ),
                                ),
                        ),
                )
                .child(
                    v_flex()
                        .gap_2p5()
                        .px_8()
                        .pb_6()
                        .child(provider_row(Provider {
                            id: "provider-onedrive".into(),
                            name: "OneDrive",
                            meta: "Recommended · Microsoft account",
                            letter: "O",
                            color: palette::blue(),
                            selected: true,
                        }))
                        .child(provider_row(Provider {
                            id: "provider-sharepoint".into(),
                            name: "SharePoint",
                            meta: "Microsoft 365 · team document libraries",
                            letter: "S",
                            color: hsla(0.49, 0.65, 0.30, 1.0),
                            selected: false,
                        }))
                        .child(provider_row(Provider {
                            id: "provider-icloud".into(),
                            name: "iCloud Drive",
                            meta: "Apple ID · seamless on macOS",
                            letter: "i",
                            color: hsla(0.55, 1.0, 0.50, 1.0),
                            selected: false,
                        })),
                )
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .px_8()
                        .py_4()
                        .bg(palette::sidebar())
                        .border_t_1()
                        .border_color(palette::border())
                        .child(
                            gpui_component::Icon::from(AppIcon::Shield)
                                .with_size(gpui_component::Size::Size(px(14.)))
                                .text_color(palette::text_muted()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(palette::text_muted())
                                .child(
                                    "KeePass RS uses the official Microsoft Graph API. We never see your password.",
                                ),
                        )
                        .child(
                            div()
                                .id("connect-cancel")
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
                                .on_click(cx.listener(
                                    |_: &mut AppShell, _: &ClickEvent, window, cx| {
                                        window.dispatch_action(Box::new(CancelUnlock), cx);
                                    },
                                )),
                        )
                        .child(
                            div()
                                .id("connect-continue")
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
                                .gap_2()
                                .child("Continue with OneDrive")
                                .child(
                                    gpui_component::Icon::from(gpui_component::IconName::ArrowRight)
                                        .with_size(gpui_component::Size::Size(px(13.)))
                                        .text_color(palette::panel()),
                                )
                                .on_click(cx.listener(
                                    |_: &mut AppShell, _: &ClickEvent, window, cx| {
                                        window.push_notification(
                                            "OneDrive sync isn't wired up in this build.",
                                            cx,
                                        );
                                        window.dispatch_action(Box::new(CancelUnlock), cx);
                                    },
                                )),
                        ),
                ),
        )
        .into_any_element()
}

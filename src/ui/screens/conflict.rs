use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement as _,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};

use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::atoms::{ChipTone, chip};

pub fn render(_shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let apply_button = div()
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
        .on_click(cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
            window.push_notification("Conflict resolution applied (demo).", cx);
            shell.state().clone().update(cx, |state, cx| {
                let _ = state.close_overlay(cx);
            });
        }));

    v_flex()
        .size_full()
        .bg(cx.theme().background)
        .child(
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
                                .child("Sync conflict on 1 entry"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(palette::text_muted())
                                .child(
                                    "Both this Mac and another device modified GitHub while offline. Pick a version or merge fields.",
                                ),
                        ),
                )
                .child(apply_button),
        )
        .child(
            h_flex()
                .flex_1()
                .min_h(px(0.))
                .gap_3p5()
                .p_6()
                .child(column(
                    "This Mac",
                    "MacBook Pro 16",
                    "Today 14:21",
                    true,
                ))
                .child(column(
                    "OneDrive",
                    "iPhone 15 · KeePass RS 0.4.2",
                    "Today 14:18",
                    false,
                )),
        )
        .into_any_element()
}

struct Field {
    label: &'static str,
    value: &'static str,
    same: bool,
    differs: bool,
    meta: bool,
}

fn fields(side_local: bool) -> Vec<Field> {
    let pw = if side_local {
        "••• (24 chars, rotated)"
    } else {
        "••• (18 chars)"
    };
    let notes = if side_local {
        "Recovery codes in 1Vault\nSSH key fingerprint: SHA256:K9…"
    } else {
        "Recovery codes in 1Vault"
    };
    let modified = if side_local { "2 minutes ago" } else { "5 minutes ago" };

    vec![
        Field { label: "Title", value: "GitHub", same: true, differs: false, meta: false },
        Field { label: "Username", value: "jritter-dev", same: true, differs: false, meta: false },
        Field { label: "Password", value: pw, same: false, differs: true, meta: false },
        Field { label: "URL", value: "github.com", same: true, differs: false, meta: false },
        Field { label: "Notes", value: notes, same: false, differs: true, meta: false },
        Field { label: "Modified", value: modified, same: false, differs: false, meta: true },
    ]
}

fn column(title: &'static str, device: &'static str, time: &'static str, selected: bool) -> AnyElement {
    let header_bg = if selected { palette::blue_soft() } else { palette::sidebar() };
    let border = if selected { palette::blue() } else { palette::border() };
    let highlight_bg = gpui::Hsla {
        h: 0.072464,
        s: 0.851852,
        l: 0.97,
        a: 1.0,
    };

    let mut col = v_flex()
        .flex_1()
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
                        .bg(if selected { palette::blue() } else { palette::panel() })
                        .border_1()
                        .border_color(if selected { palette::blue() } else { palette::border() })
                        .text_color(if selected { palette::panel() } else { palette::text_muted() })
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
                                .child(format!("{device} · {time}")),
                        ),
                )
                .child(if selected {
                    chip("Keeping", ChipTone::Blue)
                } else {
                    chip("Keep this", ChipTone::Gray)
                }),
        );

    let f_list = fields(selected);
    for (i, f) in f_list.iter().enumerate() {
        let last = i == f_list.len() - 1;
        col = col.child(
            v_flex()
                .gap_1()
                .p_3()
                .when(!last, |this| this.border_b_1().border_color(palette::border()))
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
                        .when(f.differs, |this| this.child(chip("Differs", ChipTone::Orange)))
                        .when(f.same, |this| this.child(chip("Same", ChipTone::Gray))),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(if f.meta { palette::text_muted() } else { palette::text() })
                        .font_family(if f.label == "Notes" { "" } else { "JetBrains Mono" })
                        .child(f.value),
                ),
        );
    }

    col.into_any_element()
}

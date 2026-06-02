use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::{Icon, Sizable as _, Size, h_flex};

use crate::app::actions::{InstallUpdate, RestartToUpdate};
use crate::ui::widgets::interaction::Interaction as _;
use crate::ui::{AppShell, icons::AppIcon, palette};
use crate::update::UpdateStatus;

pub fn update_chip(status: &UpdateStatus, cx: &mut Context<AppShell>) -> Option<AnyElement> {
    let (id, label, tone, action): (&str, SharedString, ChipTone, Option<UpdateAction>) =
        match status {
            UpdateStatus::Available(info) => (
                "update-chip-install",
                SharedString::from(format!("Install {}", info.version)),
                ChipTone::Blue,
                Some(UpdateAction::Install),
            ),
            UpdateStatus::Downloading { progress, .. } => {
                let label = if *progress > 0.0 {
                    format!("Downloading… {}%", (progress * 100.0).round() as u32)
                } else {
                    "Downloading…".into()
                };
                (
                    "update-chip-downloading",
                    label.into(),
                    ChipTone::Muted,
                    None,
                )
            }
            UpdateStatus::ReadyToRestart(_) => (
                "update-chip-restart",
                "Restart to Update".into(),
                ChipTone::Blue,
                Some(UpdateAction::Restart),
            ),
            UpdateStatus::Idle | UpdateStatus::Checking | UpdateStatus::Failed(_) => return None,
        };

    let (bg, border, fg) = match tone {
        ChipTone::Blue => (palette::blue(), palette::blue_hover(), palette::panel()),
        ChipTone::Muted => (
            palette::sidebar(),
            palette::border_strong(),
            palette::text_muted(),
        ),
    };

    let chip = h_flex()
        .id(id)
        .h(px(30.))
        .px(px(10.))
        .gap_1p5()
        .items_center()
        .rounded(px(6.))
        .bg(bg)
        .border_1()
        .border_color(border)
        .text_xs()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(fg)
        .child(
            Icon::from(AppIcon::Refresh)
                .with_size(Size::Size(px(13.)))
                .text_color(fg),
        )
        .child(div().whitespace_nowrap().child(label));

    let chip = match action {
        Some(UpdateAction::Install) => chip
            .hover_press(palette::blue_hover())
            .on_click(cx.listener(|_: &mut AppShell, _, window: &mut Window, cx| {
                window.dispatch_action(Box::new(InstallUpdate), cx);
            })),
        Some(UpdateAction::Restart) => chip
            .hover_press(palette::blue_hover())
            .on_click(cx.listener(|_: &mut AppShell, _, window: &mut Window, cx| {
                window.dispatch_action(Box::new(RestartToUpdate), cx);
            })),
        None => chip,
    };

    Some(chip.into_any_element())
}

#[derive(Clone, Copy)]
enum UpdateAction {
    Install,
    Restart,
}

#[derive(Clone, Copy)]
enum ChipTone {
    Blue,
    Muted,
}

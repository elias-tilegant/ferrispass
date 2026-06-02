use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Sizable as _, h_flex,
    input::{Input, InputState},
    v_flex,
};

use crate::app::BiometricAttempt;
use crate::app::actions::{
    CancelUnlock, ForgetBiometric, SubmitBiometricUnlock, SubmitPassword, ToggleBiometricEnrollment,
};
use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::atoms::label;
use crate::ui::widgets::interaction::Interaction as _;

pub fn render(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let state = shell.state().read(cx);
    let prompt = match state.unlock_prompt() {
        Some(p) => p,
        None => return div().into_any_element(),
    };
    let summary = state.summary();
    let subtitle = match (&summary.provider, &summary.synced_at) {
        (Some(provider), Some(synced)) => format!("{provider} · {synced}"),
        (Some(provider), None) => provider.clone(),
        _ => prompt.display_path.clone(),
    };

    // `available` = sensor reachable right now (Touch ID works this
    // instant).
    // `supported` = biometric hardware exists at all (drives the
    // enrolment checkbox — enrolment writes to the keychain
    // without touching the sensor, so a user in clamshell mode
    // can still opt in for next time the lid opens).
    let store = state.biometric_store();
    let biometric_available = store.is_available();
    let biometric_supported = store.is_supported();
    let has_enrollment = state.biometric_for_pending().is_some();
    // The passcode-fallback setting decides whether the OS prompt
    // can also accept the macOS account password. When it's on, the
    // unlock button is useful even with the sensor unreachable
    // (clamshell mode) — the prompt falls back to the password
    // field. So the button shows when biometry is reachable *or*
    // the passcode fallback can carry the attempt.
    let allow_passcode_fallback = shell.settings().biometric_allow_passcode_fallback;
    let show_unlock_button = has_enrollment && (biometric_available || allow_passcode_fallback);
    let attempt = state.biometric_attempt().clone();
    let enrollment_pending = state.pending_biometric_enrollment();
    let biometric_in_flight = matches!(attempt, BiometricAttempt::InFlight { .. });
    let biometric_error = match &attempt {
        BiometricAttempt::Error { message, .. } => Some(message.clone()),
        _ => None,
    };

    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(cx.theme().background)
        .child(
            v_flex()
                .w(px(420.))
                .p_10()
                .bg(palette::panel())
                .border_1()
                .border_color(palette::border())
                .rounded(px(12.))
                .gap_5()
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(
                            div()
                                .size(px(40.))
                                .rounded(px(8.))
                                .bg(palette::blue_soft())
                                .border_1()
                                .border_color(palette::blue_border())
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    gpui_component::Icon::from(AppIcon::Lock)
                                        .with_size(gpui_component::Size::Size(px(18.)))
                                        .text_color(palette::blue()),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_base()
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .child(prompt.file_name.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(palette::text_muted())
                                        .font_family("JetBrains Mono")
                                        .child(subtitle),
                                ),
                        ),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(label("Master password"))
                        .child(input_box(shell.password_input())),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            h_flex().gap_1().child(label("Key file")).child(
                                div()
                                    .text_xs()
                                    .text_color(palette::text_faint())
                                    .child("(optional)"),
                            ),
                        )
                        .child(input_box(shell.keyfile_input())),
                )
                // Conditional Touch-ID enrolment checkbox: shown
                // for any vault that's not yet enrolled, as long
                // as the device *has* Touch ID hardware — even if
                // the sensor isn't reachable in the current
                // physical setup (clamshell mode etc.). Enrolment
                // writes to the keychain without a prompt, so the
                // user can opt in here and the unlock button
                // appears next time the sensor is reachable.
                .when(biometric_supported && !has_enrollment, |this| {
                    this.child(touch_id_enrollment_checkbox(enrollment_pending, cx))
                })
                .when_some(prompt.error.clone(), |this, error| {
                    this.child(div().text_sm().text_color(palette::red()).child(error))
                })
                .when_some(biometric_error, |this, error| {
                    this.child(div().text_sm().text_color(palette::red()).child(error))
                })
                .child(
                    div()
                        .id("unlock-submit")
                        .pressable_dim()
                        .on_click(cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                            window.dispatch_action(Box::new(SubmitPassword), cx);
                        }))
                        .child(primary_button("Unlock vault", AppIcon::Unlock)),
                )
                .when(show_unlock_button, |this| {
                    this.child(touch_id_unlock_button(
                        biometric_in_flight,
                        biometric_available,
                        cx,
                    ))
                })
                .child(
                    div()
                        .pt_4()
                        .border_t_1()
                        .border_color(palette::border())
                        .child(
                            h_flex()
                                .items_center()
                                .justify_between()
                                .text_xs()
                                .text_color(palette::text_faint())
                                .child(footer_left(biometric_supported, has_enrollment, cx))
                                .child(
                                    div()
                                        .id("cancel-unlock")
                                        .text_color(palette::blue())
                                        .hover(|s| s.text_color(palette::blue_hover()))
                                        .pressable()
                                        .child("Cancel")
                                        .on_click(cx.listener(
                                            |_: &mut AppShell, _: &ClickEvent, window, cx| {
                                                window.dispatch_action(Box::new(CancelUnlock), cx);
                                            },
                                        )),
                                ),
                        ),
                ),
        )
        .into_any_element()
}

fn touch_id_enrollment_checkbox(
    checked: bool,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    h_flex()
        .id("biometric-enrol-toggle")
        .gap_2()
        .items_center()
        .text_xs()
        .text_color(palette::text_muted())
        .pressable()
        .on_click(cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
            window.dispatch_action(Box::new(ToggleBiometricEnrollment), cx);
        }))
        .child(
            div()
                .size(px(14.))
                .rounded(px(3.))
                .border_1()
                .border_color(palette::border_strong())
                .flex()
                .items_center()
                .justify_center()
                .when(checked, |this| {
                    this.bg(palette::blue())
                        .border_color(palette::blue())
                        .text_color(palette::panel())
                        .text_xs()
                        .child("✓")
                }),
        )
        .child("Enable Touch ID for this vault")
}

fn touch_id_unlock_button(
    in_flight: bool,
    sensor_available: bool,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    // Honest label: when the sensor is reachable the OS prompt leads
    // with Touch ID, so "Unlock with Touch ID" is accurate. When it
    // isn't (clamshell mode, or a Mac with no Touch ID hardware) the
    // button is only shown because the passcode-fallback setting is
    // on — the prompt will present the macOS account password — so we
    // say that instead of mislabeling a password unlock as biometric.
    let label_text = if in_flight {
        "Waiting…"
    } else if sensor_available {
        "Unlock with Touch ID"
    } else {
        "Unlock with macOS password"
    };
    let icon = if sensor_available {
        AppIcon::Fingerprint
    } else {
        AppIcon::Unlock
    };

    div()
        .id("biometric-unlock")
        .when(!in_flight, |this| {
            this.pressable_dim()
                .on_click(cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                    window.dispatch_action(Box::new(SubmitBiometricUnlock), cx);
                }))
        })
        .child(secondary_button(label_text, icon, in_flight))
}

/// Bottom-left status: hint text describing the current biometric
/// state. Doubles as a "Forget" affordance when enrolled — keeps the
/// footer compact (still one row) while exposing the un-enrol path
/// without needing the Settings page (which lands in a later phase).
///
/// Note: `supported` is used (not `available`) so an enrolled user
/// in clamshell mode still sees and can click "Forget Touch ID" —
/// dropping an enrolment doesn't need the sensor to be reachable.
fn footer_left(
    supported: bool,
    enrolled: bool,
    cx: &mut Context<AppShell>,
) -> impl gpui::IntoElement {
    if enrolled {
        return div()
            .id("biometric-forget")
            .text_color(palette::blue())
            .hover(|s| s.text_color(palette::blue_hover()))
            .pressable()
            .child("Forget Touch ID")
            .on_click(cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                window.dispatch_action(Box::new(ForgetBiometric), cx);
            }))
            .into_any_element();
    }
    if !supported {
        return div().child("Touch ID not available").into_any_element();
    }
    div().child("Touch ID available").into_any_element()
}

fn input_box(state: &gpui::Entity<InputState>) -> impl gpui::IntoElement {
    Input::new(state).cleanable(false)
}

fn primary_button(label: &'static str, icon: AppIcon) -> impl gpui::IntoElement {
    h_flex()
        .h(px(38.))
        .w_full()
        .px_4()
        .rounded(px(6.))
        .gap_2()
        .items_center()
        .justify_center()
        .bg(palette::blue())
        .border_1()
        .border_color(palette::blue_hover())
        .text_color(palette::panel())
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .child(
            gpui_component::Icon::from(icon)
                .with_size(gpui_component::Size::Size(px(14.)))
                .text_color(palette::panel()),
        )
        .child(label)
}

/// Secondary, outlined variant of `primary_button` used for the
/// Touch-ID action. Distinct visual weight signals that password
/// remains the canonical path while Touch ID is the convenience
/// shortcut.
fn secondary_button(label: &'static str, icon: AppIcon, disabled: bool) -> impl gpui::IntoElement {
    let (bg, fg, border) = if disabled {
        (palette::panel(), palette::text_muted(), palette::border())
    } else {
        (palette::panel(), palette::blue(), palette::blue_border())
    };
    h_flex()
        .h(px(36.))
        .w_full()
        .px_4()
        .rounded(px(6.))
        .gap_2()
        .items_center()
        .justify_center()
        .bg(bg)
        .border_1()
        .border_color(border)
        .text_color(fg)
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .child(
            gpui_component::Icon::from(icon)
                .with_size(gpui_component::Size::Size(px(14.)))
                .text_color(fg),
        )
        .child(label)
}

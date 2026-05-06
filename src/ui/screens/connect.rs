//! Cloud-connect overlay. Renders one of four sub-views depending on
//! `state.connect_flow`:
//!
//! 1. `PickProvider` — three provider buttons (SharePoint wired)
//! 2. `SigningIn`    — device code + verification URL + "Open in browser"
//! 3. `Picking`      — search-as-you-type list of the user's `.kdbx` files
//! 4. `Downloading`  — spinner while we fetch the picked file
//! 5. `Failed`       — error + "Back" button to retry from step 1

use gpui::{
    AnyElement, ClickEvent, ClipboardItem, Context, InteractiveElement as _, IntoElement as _,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, div, hsla, px,
};
use gpui_component::{
    ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, input::Input, v_flex,
};

use crate::app::ConnectFlow;
use crate::app::actions::CancelUnlock;
use crate::sync::auth::DeviceCodeChallenge;
use crate::sync::graph::DriveItemHit;
use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::buttons::step_indicator;
use crate::ui::widgets::provider_row::{Provider, provider_row};

pub fn render(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let flow = shell
        .state()
        .read(cx)
        .connect_flow()
        .cloned()
        .unwrap_or(ConnectFlow::PickProvider);

    let (step_index, body) = match &flow {
        ConnectFlow::PickProvider => (1usize, render_pick_provider(cx)),
        ConnectFlow::SigningIn { challenge } => (2, render_signing_in(challenge, cx)),
        ConnectFlow::Picking {
            results,
            query,
            loading,
            error,
            ..
        } => (
            3,
            render_picking(shell, results, query, *loading, error.as_deref(), cx),
        ),
        ConnectFlow::Downloading => (3, render_downloading(cx)),
        ConnectFlow::Failed(msg) => (1, render_failed(msg, cx)),
    };

    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .p_10()
        .bg(cx.theme().background)
        .child(
            v_flex()
                .w(px(680.))
                .max_h_full()
                .min_h(px(0.))
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
                            step_index,
                            cx,
                        ))
                        .child(body),
                )
                .child(footer(cx)),
        )
        .into_any_element()
}

// ---------------- Step 1: provider picker ----------------

fn render_pick_provider(cx: &mut Context<AppShell>) -> AnyElement {
    v_flex()
        .gap_4()
        .child(heading(
            "Where should FerrisPass sync your vault?",
            "Your .kdbx file is encrypted on this device before it ever \
             leaves. The provider only sees ciphertext.",
        ))
        .child(
            v_flex()
                .gap_2p5()
                .child(provider_row_button(
                    Provider {
                        id: "provider-sharepoint".into(),
                        name: "SharePoint",
                        meta: "Microsoft 365 · team document libraries",
                        letter: "S",
                        color: hsla(0.49, 0.65, 0.30, 1.0),
                        selected: false,
                    },
                    true,
                    cx,
                ))
                .child(provider_row_button(
                    Provider {
                        id: "provider-onedrive".into(),
                        name: "OneDrive",
                        meta: "Personal Microsoft account · coming soon",
                        letter: "O",
                        color: palette::blue(),
                        selected: false,
                    },
                    false,
                    cx,
                ))
                .child(provider_row_button(
                    Provider {
                        id: "provider-icloud".into(),
                        name: "iCloud Drive",
                        meta: "Apple ID · coming soon",
                        letter: "i",
                        color: hsla(0.55, 1.0, 0.50, 1.0),
                        selected: false,
                    },
                    false,
                    cx,
                )),
        )
        .into_any_element()
}

fn provider_row_button(
    provider: Provider,
    enabled: bool,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let row = div().id(provider.id.clone()).child(provider_row(provider));
    if enabled {
        row.on_click(
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                // SharePoint button → go straight to device-code sign-in (no
                // URL entry step; user picks the file from the post-auth list).
                shell
                    .state()
                    .clone()
                    .update(cx, |state, cx| state.start_sharepoint_connect(cx));
            }),
        )
        .into_any_element()
    } else {
        row.on_click(cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
            window.push_notification(
                "Only SharePoint is wired in this build. OneDrive and iCloud are coming.",
                cx,
            );
        }))
        .into_any_element()
    }
}

// ---------------- Step 2: device code ----------------

fn render_signing_in(challenge: &DeviceCodeChallenge, cx: &mut Context<AppShell>) -> AnyElement {
    let user_code = challenge.user_code.clone();
    let user_code_for_clipboard = user_code.clone();
    let verification_uri = challenge.verification_uri.clone();
    let verification_uri_for_open = verification_uri.clone();
    v_flex()
        .gap_4()
        .child(heading(
            "Sign in with your Microsoft account",
            "Open the link below, sign in, and enter the one-time code. \
             We'll catch the result automatically — no need to come back \
             and click anything.",
        ))
        .child(
            v_flex()
                .gap_3()
                .p_4()
                .rounded(px(8.))
                .bg(palette::sidebar())
                .border_1()
                .border_color(palette::border())
                .child(field_label("Verification URL"))
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .font_family("JetBrains Mono")
                                .text_sm()
                                .text_color(palette::text())
                                .child(verification_uri.clone()),
                        )
                        .child(
                            div()
                                .id("open-verification")
                                .h(px(28.))
                                .px(px(10.))
                                .rounded(px(5.))
                                .bg(palette::blue())
                                .text_xs()
                                .text_color(palette::panel())
                                .flex()
                                .items_center()
                                .justify_center()
                                .child("Open in browser")
                                .on_click(cx.listener(
                                    move |_: &mut AppShell, _: &ClickEvent, _, _| {
                                        AppShell::open_browser(&verification_uri_for_open);
                                    },
                                )),
                        ),
                )
                .child(field_label("One-time code"))
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .font_family("JetBrains Mono")
                                .text_xl()
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(palette::text())
                                .child(user_code.clone()),
                        )
                        .child(
                            div()
                                .id("copy-user-code")
                                .h(px(28.))
                                .px(px(10.))
                                .rounded(px(5.))
                                .bg(palette::panel())
                                .border_1()
                                .border_color(palette::border_strong())
                                .text_xs()
                                .text_color(palette::text())
                                .flex()
                                .items_center()
                                .justify_center()
                                .child("Copy code")
                                .on_click(cx.listener(
                                    move |_: &mut AppShell, _: &ClickEvent, window, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            user_code_for_clipboard.clone(),
                                        ));
                                        window.push_notification("Code copied to clipboard.", cx);
                                    },
                                )),
                        ),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette::text_muted())
                .child("Waiting for sign-in… we're polling Microsoft for you."),
        )
        .into_any_element()
}

// ---------------- Step 3: pick a file ----------------

fn render_picking(
    shell: &AppShell,
    results: &[DriveItemHit],
    query: &str,
    loading: bool,
    error: Option<&str>,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let query_input = shell.picker_query_input().clone();
    let filtered = if query.is_empty() {
        results.to_vec()
    } else {
        let q = query.to_lowercase();
        results
            .iter()
            .filter(|h| h.name.to_lowercase().contains(&q) || h.path.to_lowercase().contains(&q))
            .cloned()
            .collect()
    };

    let body: AnyElement = if loading {
        // Loading: show a thin progress bar + message. Zero results yet.
        v_flex()
            .gap_3()
            .child(
                div()
                    .h(px(4.))
                    .w_full()
                    .rounded(px(2.))
                    .bg(palette::sidebar())
                    .child(
                        div()
                            .h_full()
                            .w(gpui::relative(0.4))
                            .rounded(px(2.))
                            .bg(palette::blue()),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(palette::text_muted())
                    .child("Searching SharePoint for .kdbx files…"),
            )
            .into_any_element()
    } else if let Some(msg) = error {
        // Network or auth error during search.
        let msg_owned = msg.to_string();
        div()
            .p_3()
            .rounded(px(6.))
            .bg(palette::sidebar())
            .border_1()
            .border_color(palette::orange_border())
            .text_xs()
            .font_family("JetBrains Mono")
            .text_color(palette::text())
            .child(msg_owned)
            .into_any_element()
    } else if results.is_empty() {
        // Search returned nothing.
        div()
            .p_4()
            .rounded(px(8.))
            .bg(palette::sidebar())
            .border_1()
            .border_color(palette::border())
            .text_sm()
            .text_color(palette::text())
            .child(
                "No .kdbx files found in your SharePoint or OneDrive. \
                 Upload a vault via the SharePoint web UI first, then come \
                 back and reconnect.",
            )
            .into_any_element()
    } else if filtered.is_empty() {
        // Filter excluded everything.
        div()
            .p_4()
            .rounded(px(8.))
            .bg(palette::sidebar())
            .border_1()
            .border_color(palette::border())
            .text_sm()
            .text_color(palette::text_muted())
            .child(format!("No matches for \"{query}\"."))
            .into_any_element()
    } else {
        results_list(filtered, cx)
    };

    v_flex()
        .gap_4()
        .min_h(px(0.))
        .child(heading(
            "Pick a .kdbx file",
            "We searched your SharePoint sites and personal OneDrive for \
             KeePass databases. Pick one to download.",
        ))
        .child(Input::new(&query_input))
        .child(body)
        .into_any_element()
}

fn results_list(results: Vec<DriveItemHit>, cx: &mut Context<AppShell>) -> AnyElement {
    let mut col = v_flex()
        .id("picker-scroll")
        .min_h(px(0.))
        .max_h(px(360.))
        .overflow_y_scroll()
        .border_1()
        .border_color(palette::border())
        .rounded(px(8.))
        .bg(palette::panel());

    let total = results.len();
    for (i, hit) in results.into_iter().enumerate() {
        let last = i == total - 1;
        col = col.child(picker_row(hit, last, cx));
    }
    col.into_any_element()
}

fn picker_row(hit: DriveItemHit, last: bool, cx: &mut Context<AppShell>) -> AnyElement {
    use gpui::prelude::FluentBuilder as _;
    let display_path = friendly_path(&hit.path);
    let modified = friendly_modified(&hit.last_modified);
    // Stable id per row — item_id is a Graph identifier that's unique per
    // file, so we use it directly. (gpui interns the SharedString.)
    let row_id = gpui::ElementId::Name(hit.item_id.clone().into());
    let hit_for_click = hit.clone();

    h_flex()
        .id(row_id)
        .gap_3()
        .items_center()
        .px_4()
        .py_3()
        .when(!last, |this| {
            this.border_b_1().border_color(palette::border())
        })
        .hover(|this| this.bg(palette::sidebar()))
        .child(
            div()
                .size(px(28.))
                .rounded(px(6.))
                .bg(palette::orange_soft())
                .flex()
                .items_center()
                .justify_center()
                .child(
                    gpui_component::Icon::from(AppIcon::Key)
                        .with_size(gpui_component::Size::Size(px(13.)))
                        .text_color(palette::orange()),
                ),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w(px(0.))
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .truncate()
                        .child(hit.name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(palette::text_muted())
                        .truncate()
                        .child(display_path),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette::text_faint())
                .child(modified),
        )
        .on_click(
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, window, cx| {
                shell.start_pick_kdbx_file(hit_for_click.clone(), window, cx);
            }),
        )
        .into_any_element()
}

/// Shorten a Graph parentReference path like `/drives/b!xxx/root:/Folder/Sub`
/// into something the user can read at a glance: just the segments after
/// `root:`. Empty when path is missing or root-level.
fn friendly_path(graph_path: &str) -> String {
    if let Some(idx) = graph_path.find("root:") {
        let after = &graph_path[idx + 5..];
        let trimmed = after.trim_start_matches('/');
        if trimmed.is_empty() {
            "(root)".into()
        } else {
            trimmed.to_string()
        }
    } else {
        graph_path.to_string()
    }
}

/// "2 hours ago" / "yesterday" style. Falls back to the raw RFC3339
/// string if we can't parse it.
fn friendly_modified(rfc3339: &str) -> String {
    use chrono::{DateTime, Local, Utc};
    let Ok(parsed) = rfc3339.parse::<DateTime<Utc>>() else {
        return rfc3339.to_string();
    };
    let local: DateTime<Local> = parsed.into();
    let now = Local::now();
    let secs = (now - local).num_seconds().max(0);
    if secs < 3600 {
        format!("{} min ago", (secs / 60).max(1))
    } else if secs < 86_400 {
        format!("{} h ago", secs / 3600)
    } else if secs < 7 * 86_400 {
        format!("{} d ago", secs / 86_400)
    } else {
        local.format("%Y-%m-%d").to_string()
    }
}

// ---------------- Step 3b: downloading ----------------

fn render_downloading(_cx: &mut Context<AppShell>) -> AnyElement {
    v_flex()
        .gap_4()
        .child(heading(
            "Downloading your vault…",
            "Fetching the file, saving a local copy. This usually takes a \
             couple of seconds.",
        ))
        .child(
            div()
                .h(px(4.))
                .w_full()
                .rounded(px(2.))
                .bg(palette::sidebar())
                .child(
                    div()
                        .h_full()
                        .w(gpui::relative(0.4))
                        .rounded(px(2.))
                        .bg(palette::blue()),
                ),
        )
        .into_any_element()
}

// ---------------- Failure ----------------

fn render_failed(msg: &str, cx: &mut Context<AppShell>) -> AnyElement {
    let msg_owned = msg.to_string();
    v_flex()
        .gap_4()
        .child(heading(
            "Connect didn't complete",
            "Something interrupted the sign-in or download. You can retry \
             from the start.",
        ))
        .child(
            div()
                .p_3()
                .rounded(px(6.))
                .bg(palette::sidebar())
                .border_1()
                .border_color(palette::orange_border())
                .text_xs()
                .font_family("JetBrains Mono")
                .text_color(palette::text())
                .child(msg_owned),
        )
        .child(h_flex().gap_3().pt_2().child(secondary_button(
            "Back",
            "back-to-providers",
            cx,
            |state, cx| {
                state.connect_flow_set(ConnectFlow::PickProvider, cx);
            },
        )))
        .into_any_element()
}

// ---------------- shared chrome ----------------

fn heading(title: &'static str, subtitle: &'static str) -> AnyElement {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_xl()
                .font_weight(gpui::FontWeight::BOLD)
                .child(title),
        )
        .child(
            div()
                .text_xs()
                .text_color(palette::text_muted())
                .child(subtitle),
        )
        .into_any_element()
}

fn field_label(label: &'static str) -> AnyElement {
    div()
        .text_xs()
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(palette::text_faint())
        .child(label)
        .into_any_element()
}

fn footer(cx: &mut Context<AppShell>) -> AnyElement {
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
                    "FerrisPass uses the official Microsoft Graph API. We never see your password.",
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
                .on_click(cx.listener(|_: &mut AppShell, _: &ClickEvent, window, cx| {
                    window.dispatch_action(Box::new(CancelUnlock), cx);
                })),
        )
        .into_any_element()
}

fn secondary_button(
    label: &'static str,
    id: &'static str,
    cx: &mut Context<AppShell>,
    on_click: impl Fn(&mut crate::app::AppState, &mut Context<crate::app::AppState>) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .h(px(30.))
        .px(px(14.))
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
        .child(label)
        .on_click(
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                shell
                    .state()
                    .clone()
                    .update(cx, |state, cx| on_click(state, cx));
            }),
        )
        .into_any_element()
}

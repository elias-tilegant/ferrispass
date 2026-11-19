//! The panel over the other apps: over a hidden FerrisPass, the preview of
//! the selected entry, a search without a match, the line that says what
//! failed, and an SAP client written as a spreadsheet leaves it.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{Role, TestAppContext, VisualTestContext};

use super::*;
use crate::app::Overlay;
use crate::platform::activation::{self, fake as hidden};
use crate::ui::ds::components::CopyState;
use crate::ui::vm::panel::PanelError;
use crate::ui::windows::panel::view::{
    PreviewFlags, PreviewValues, error_line, no_match, preview, preview_flags,
};

thread_local! {
    static VOICEOVER: Cell<bool> = const { Cell::new(false) };
    static OTP_TICKS: Cell<usize> = const { Cell::new(0) };
}

/// What the panel hears when it asks whether VoiceOver runs.
pub(crate) fn voiceover_running() -> bool {
    VOICEOVER.get()
}

fn set_voiceover(on: bool) {
    VOICEOVER.set(on);
}

/// Called each time the one-time code ring is drawn again.
pub(crate) fn note_otp_tick() {
    OTP_TICKS.set(OTP_TICKS.get() + 1);
}

/// VoiceOver off and no ticks, as at the start of a test.
pub(crate) fn reset() {
    VOICEOVER.set(false);
    OTP_TICKS.set(0);
}

/// A password the tests look for everywhere it must not be.
const SECRET: &str = "sp-pw-7Qx";

/// A one-time code secret; the code itself changes every 30 seconds.
const OTP: &str = "otpauth://totp/X?secret=JBSWY3DPEHPK3PXP";

/// A web login with a user name, a password and a one-time code.
fn sales() -> Fixture {
    entry("Salesforce")
        .user("petra.engel@example.com")
        .password(SECRET)
        .url("https://login.salesforce.com")
        .field("otp", OTP)
}

fn error_of(app: &Harness, cx: &mut TestAppContext) -> Option<PanelError> {
    app.vm(cx).read_with(cx, |vm, _| vm.error())
}

/// Whether the panel's last frame drew the element marked `selector`.
fn drawn(cx: &mut TestAppContext, panel: AnyWindowHandle, selector: &'static str) -> bool {
    let mut window = VisualTestContext::from_window(panel, cx);
    window.run_until_parked();
    window.debug_bounds(selector).is_some()
}

/// Clicks the middle of the element marked `selector` in the panel.
fn click(cx: &mut TestAppContext, panel: AnyWindowHandle, selector: &'static str) {
    let mut window = VisualTestContext::from_window(panel, cx);
    window.run_until_parked();
    let bounds = window
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    window.simulate_click(bounds.center(), gpui::Modifiers::none());
    cx.run_until_parked();
}

/// `digits` as they could show anywhere: as they are, in groups of three as
/// the ring writes them, and in pairs.
fn spellings(digits: &str) -> Vec<String> {
    let grouped = |size: usize| {
        digits
            .chars()
            .collect::<Vec<_>>()
            .chunks(size)
            .map(|chunk| chunk.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join(" ")
    };
    vec![digits.to_string(), grouped(3), grouped(2)]
}

// ------------------------------------------------------ the hidden app --

/// Over a hidden app the panel unhides it without activating it and orders
/// the main window out, once each, and hides the app again when it closes,
/// however it closes. Over a visible app it does none of the three.
#[gpui::test]
fn the_panel_remembers_whether_the_app_was_hidden_and_hides_it_again(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let none = hidden::Calls::default();

    let panel = app.open_panel(cx);
    activate(cx, panel);
    keys(cx, panel, "escape");
    assert!(!panel_is_open(cx));
    assert_eq!(hidden::calls(), none, "a visible app: nothing extra");

    for close in ["escape", "cmd-w", "shortcut", "click elsewhere", "lock"] {
        hidden::reset();
        hidden::hide_for_test();
        let panel = app.open_panel(cx);
        let opened = hidden::calls();
        assert_eq!(
            (opened.unhides, opened.order_outs, opened.hides),
            (1, 1, 0),
            "{close}: opening over the hidden app"
        );
        assert!(!activation::app_is_hidden(), "{close}: unhidden to show");
        match close {
            "shortcut" => cx.update(toggle),
            "click elsewhere" => {
                activate(cx, panel);
                activate(cx, app.main);
            }
            "lock" => app.state.update(cx, |state, cx| {
                let _ = state.lock_vault(cx);
            }),
            keystroke => {
                activate(cx, panel);
                keys(cx, panel, keystroke);
            }
        }
        cx.run_until_parked();
        assert!(!panel_is_open(cx), "{close}: the panel closed");
        assert_eq!(
            hidden::calls(),
            hidden::Calls {
                unhides: 1,
                order_outs: 1,
                order_fronts: 0,
                hides: 1,
            },
            "{close}: closing hides the app again, once"
        );
        assert!(activation::app_is_hidden(), "{close}: hidden again");
        if close == "lock" {
            break;
        }
    }
}

/// Cmd+O over a hidden app: the user asked for the main window, so the app
/// is not hidden again, and the windows the panel ordered out come back.
#[gpui::test]
fn the_main_window_comes_back_when_the_panel_closes_for_it(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    hidden::hide_for_test();
    let panel = app.open_panel(cx);
    type_into(cx, panel, "webmail");
    keys(cx, panel, "cmd-o");
    assert!(!panel_is_open(cx));
    assert_eq!(
        hidden::calls(),
        hidden::Calls {
            unhides: 1,
            order_outs: 1,
            order_fronts: 1,
            hides: 0,
        }
    );
    assert!(!activation::app_is_hidden());
}

/// The panel unhides, orders windows out and hides the app only through
/// `platform::activation`: AppKit's calls for it appear nowhere else in the
/// source, and the panel's own files never hide the app themselves.
#[test]
fn activation_is_the_only_place_that_unhides_or_orders_windows() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    probe::rust_files(&root.join("src"), &mut files);
    let mut found = Vec::new();
    for file in files.iter().filter(|file| !is_test_source(file)) {
        let code = probe::code_only(&std::fs::read_to_string(file).expect("read source"));
        let in_activation = file.ends_with("src/platform/activation.rs");
        for call in [
            "unhideWithoutActivation",
            "orderOut",
            "orderFront",
            "isHidden",
        ] {
            if code.contains(call) && !in_activation {
                found.push(format!("{}: {call}", file.display()));
            }
        }
    }
    assert!(found.is_empty(), "{found:#?}");
    let activation = probe::code_only(
        &std::fs::read_to_string(root.join("src/platform/activation.rs")).expect("activation"),
    );
    assert!(activation.contains("#![forbid(unsafe_code)]"));
    for (file, code) in panel_sources() {
        assert!(!code.contains("cx.hide("), "{file} hides the app itself");
    }
}

// --------------------------------------------------------- the preview --

/// The preview names the user and shows the password as twelve dots and the
/// code as "••• •••": neither reaches a node of the secret field or the
/// ring, in any spelling, whether VoiceOver runs or not.
#[gpui::test]
fn the_preview_shows_the_user_and_hides_the_secret_and_the_code(cx: &mut TestAppContext) {
    let app = open_app(cx, &[sales()]);
    let panel = app.open_panel(cx);
    type_into(cx, panel, "sales");
    let model = app
        .vm(cx)
        .read_with(cx, |vm, _| vm.preview())
        .expect("a preview once something is typed");
    assert_eq!(model.user.as_deref(), Some("petra.engel@example.com"));
    assert!(model.has_password && model.has_code);
    for assistive in [false, true] {
        let state = app.state.clone();
        let model = model.clone();
        let (texts, user, code) = probe::in_frame(cx, move |window, cx| {
            let (password, otp) = {
                let state = state.read(cx);
                (
                    state.password_for_entry(&model.entry_id),
                    state.totp_for_entry(&model.entry_id),
                )
            };
            let otp = otp.expect("a code");
            let code: String = otp.code.chars().filter(|c| c.is_ascii_digit()).collect();
            let values = PreviewValues {
                password,
                otp: Some(otp),
            };
            let parts = preview(
                &model,
                values,
                assistive,
                |_| CopyState::Idle,
                Rc::new(|_, _, _| {}),
            );
            let mut texts = Vec::new();
            let mut read = |parts: Vec<probe::ComponentPart>| {
                for part in parts {
                    for (name, text) in part.texts() {
                        if let Some(text) = text {
                            texts.push(format!("{} {name}: {text}", part.name));
                        }
                    }
                }
            };
            read(parts.password.expect("the secret field").parts(window, cx));
            let (ring, _) = parts.code.expect("the ring");
            read(ring.parts(window, cx));
            let (user, _) = parts.user.expect("the user name");
            (texts, user.to_string(), code)
        });
        assert_eq!(user, "petra.engel@example.com");
        assert!(!texts.is_empty(), "the parts were read");
        let mut forbidden = spellings(&code);
        forbidden.push(SECRET.to_string());
        forbidden.push("sp-p w-7Q x".to_string());
        for text in &texts {
            for secret in &forbidden {
                assert!(
                    !text.contains(secret.as_str()),
                    "assistive {assistive}: {secret} in {text}"
                );
            }
        }
    }
}

/// The flags the preview hands its secret field and its ring, read where
/// the panel builds them: `assistive` exactly as VoiceOver runs, read when
/// the panel opens and when it becomes key; never revealed, always masked.
/// The source holds the rest: the panel never sets `revealed`, never clears
/// `masked`, and passes the flags on as they are.
#[gpui::test]
fn the_preview_passes_the_flags_the_field_and_the_ring_need(cx: &mut TestAppContext) {
    let app = open_app(cx, &[sales()]);
    let _panel = app.open_panel(cx);
    cx.update(close);
    cx.run_until_parked();
    let vm = app.vm(cx);
    for voiceover in [false, true] {
        set_voiceover(voiceover);
        vm.update(cx, |vm, _| vm.forget());
        let panel = app.open_panel(cx);
        preview_flags();
        type_into(cx, panel, "sales");
        assert_eq!(
            preview_flags(),
            Some(PreviewFlags {
                assistive: voiceover,
                revealed: false,
                masked: true,
            }),
            "VoiceOver {voiceover}"
        );
        cx.update(close);
        cx.run_until_parked();
    }

    // VoiceOver started while the panel was open: the panel hears it when
    // it becomes key.
    set_voiceover(false);
    vm.update(cx, |vm, _| vm.forget());
    let panel = app.open_panel(cx);
    type_into(cx, panel, "sales");
    set_voiceover(true);
    activate(cx, panel);
    app.vm(cx).update(cx, |_, cx| cx.notify());
    cx.run_until_parked();
    assert_eq!(preview_flags().map(|flags| flags.assistive), Some(true));

    let view = source_of("src/ui/windows/panel/view.rs");
    for forbidden in [".revealed(", ".masked(false)", ".on_toggle_reveal("] {
        assert!(!view.contains(forbidden), "the panel calls {forbidden}");
    }
    for (call, argument) in [(".assistive(", "assistive"), (".masked(", "masked")] {
        let calls: Vec<usize> = view.match_indices(call).map(|(at, _)| at).collect();
        assert!(!calls.is_empty(), "{call} is called");
        for at in calls {
            let open = at + call.len() - 1;
            assert_eq!(
                probe::call_arguments(&view, open),
                [argument],
                "{call} gets another value than the flag"
            );
        }
    }
    let squeezed_view = squeezed(&view);
    assert_eq!(
        squeezed_view.matches("self.assistive=").count(),
        0,
        "the flag is set outside the two places that ask VoiceOver"
    );
    assert_eq!(
        squeezed_view
            .matches("assistive:voiceover_running(),")
            .count(),
        1,
        "the panel asks when it opens"
    );
    assert_eq!(
        squeezed_view
            .matches("view.assistive=voiceover_running();")
            .count(),
        1,
        "the panel asks when it becomes key"
    );
    let render = function(&squeezed_view, "fnrender_open(");
    assert!(
        render.contains("self.assistive,"),
        "the preview gets another flag than the one VoiceOver set: {render}"
    );
    let asked = function(&view, "fn voiceover_running(");
    assert!(asked.contains("a11y_prefs::read().voiceover"), "{asked}");
}

/// The preview shows the entry the panel selected, never the main window's
/// selection: a click on its password copies the panel's entry.
#[gpui::test]
fn the_preview_reads_the_entry_of_the_hit_not_the_main_selection(cx: &mut TestAppContext) {
    let app = open_app(
        cx,
        &[
            entry("Alpha").user("main-user").password("sp-main-A"),
            entry("Bravo").user("panel-user").password("sp-panel-B"),
        ],
    );
    let alpha = app.id(cx, "Alpha");
    let bravo = app.id(cx, "Bravo");
    app.state.update(cx, |state, cx| {
        state.select_library(LibrarySelection::Favorites, cx);
        state.select_library(LibrarySelection::AllItems, cx);
        state.select_entry(alpha.clone(), cx);
    });
    let panel = app.open_panel(cx);
    type_into(cx, panel, "bravo");
    let shown = app.vm(cx).read_with(cx, |vm, _| vm.preview());
    assert_eq!(shown.map(|preview| preview.entry_id), Some(bravo));
    click(cx, panel, "fp-secret-value");
    assert_eq!(app.copied(), (1, Some("sp-panel-B".to_string())));
    let main_selection = app.state.read_with(cx, |state, _| {
        state
            .vault_browser()
            .and_then(|browser| browser.selected_entry_id)
    });
    assert_eq!(main_selection, Some(alpha), "the main window's selection");
}

/// Each copy button of the preview copies through the clipboard service
/// with its clear timer, and the panel stays open; the footer says what was
/// copied, and the button shows the copy: a flash after a click, then the
/// tick, then nothing.
#[gpui::test]
fn preview_copies_go_through_the_service_and_keep_the_panel_open(cx: &mut TestAppContext) {
    let app = open_app(cx, &[sales()]);
    let panel = app.open_panel(cx);
    type_into(cx, panel, "sales");
    let vm = app.vm(cx);
    let note = |cx: &mut TestAppContext| vm.read_with(cx, |vm, _| vm.footer().note);

    click(cx, panel, "fp-secret-value");
    assert_eq!(app.copied(), (1, Some(SECRET.to_string())));
    assert!(panel_is_open(cx), "a copy keeps the panel open");
    assert_eq!(note(cx), Some(tr!("quick.footer.copied.password")));
    assert!(
        cx.update(|cx| ClipboardService::handle(cx).read(cx).deadline())
            .is_some(),
        "the copy has a clear timer"
    );
    let state = |cx: &mut TestAppContext| {
        let view = cx.update(|cx| view_for_test(cx)).expect("the view");
        view.read_with(cx, |view, _| view.copied_for_test())
    };
    assert_eq!(state(cx), Some(CopyState::Flash), "a click flashes first");
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(state(cx), Some(CopyState::Check));
    cx.executor()
        .advance_clock(crate::ui::ds::motion::HOLD_COPY_CHECK);
    cx.run_until_parked();
    assert_eq!(state(cx), None);

    click(cx, panel, "fp-preview-copy-user");
    assert_eq!(
        app.copied(),
        (2, Some("petra.engel@example.com".to_string()))
    );
    assert_eq!(note(cx), Some(tr!("quick.footer.copied.user")));

    click(cx, panel, "fp-preview-copy-code");
    let (writes, code) = app.copied();
    assert_eq!(writes, 3);
    let code = code.expect("a code");
    assert!(
        code.len() == 6 && code.chars().all(|c| c.is_ascii_digit()),
        "the code, without the space: {code:?}"
    );
    assert_eq!(note(cx), Some(tr!("quick.footer.copied.one_time_code")));
    assert!(panel_is_open(cx));
}

/// The ring is drawn again ten times a second while it shows, and the
/// clock stops when the preview has no code or shows no entry.
#[gpui::test]
fn the_ring_ticks_only_while_a_code_is_shown(cx: &mut TestAppContext) {
    let app = open_app(cx, &[sales(), entry("Webmail").password("sp-mail")]);
    let ticks_in_a_second = |cx: &mut TestAppContext| {
        OTP_TICKS.set(0);
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        OTP_TICKS.get()
    };
    let panel = app.open_panel(cx);
    assert_eq!(ticks_in_a_second(cx), 0, "no preview from an empty search");
    type_into(cx, panel, "sales");
    let ticks = ticks_in_a_second(cx);
    assert!((9..=11).contains(&ticks), "{ticks} ticks in a second");
    keys(cx, panel, "escape");
    type_into(cx, panel, "webmail");
    assert_eq!(ticks_in_a_second(cx), 0, "an entry without a code");
    keys(cx, panel, "escape");
    assert_eq!(ticks_in_a_second(cx), 0, "no preview");
}

// ------------------------------------------- no match and the error line --

/// A search without a match names the vault searched and offers a new entry
/// with its keys; Unlock only while another vault is unlocked besides the
/// open one. Cmd+N closes the panel and starts a new entry in the main
/// window.
#[gpui::test]
fn no_match_names_the_vault_and_offers_unlock_only_with_a_parked_vault(cx: &mut TestAppContext) {
    let app = open_app(cx, &sample());
    let panel = app.open_panel(cx);
    type_into(cx, panel, "zzz");
    let vm = app.vm(cx);
    assert!(vm.read_with(cx, |vm, _| vm.no_match()));
    let parts = |cx: &mut TestAppContext| {
        vm.read_with(cx, |vm, _| {
            let parts = no_match(vm, &"team-beratung".into());
            (parts.text, parts.unlock.is_some())
        })
    };
    assert_eq!(
        parts(cx),
        (tr!("quick.no_match.text", vault = "team-beratung"), false)
    );
    vm.update(cx, |vm, _| vm.assume_parked_vaults(1));
    assert!(parts(cx).1, "Unlock with a parked vault");

    keys(cx, panel, "cmd-n");
    assert!(!panel_is_open(cx), "Cmd+N closes the panel");
    assert_eq!(
        app.state.read_with(cx, |state, _| state.overlay().clone()),
        Overlay::AddEntry,
        "the main window starts a new entry"
    );
}

/// A website that does not open says so in one line above the search: an
/// alert, in the words of `quick.error.website_failed`, without the entry's
/// title or its host. The panel stays open.
#[gpui::test]
fn a_failed_website_shows_a_carmine_line_without_the_title(cx: &mut TestAppContext) {
    let app = open_app(
        cx,
        &[entry("Nordhafen Portal").url("file://intranet.example/portal")],
    );
    let panel = app.open_panel(cx);
    type_into(cx, panel, "nordhafen portal");
    keys(cx, panel, "enter");
    assert_eq!(cx.opened_url(), None);
    assert!(panel_is_open(cx));
    let error = error_of(&app, cx).expect("the line says what failed");
    assert_eq!(error, PanelError::WebsiteFailed);
    let (role, node) = probe::node(error_line(error));
    assert_eq!(role, Some(Role::Alert));
    assert_eq!(
        node.label(),
        Some(tr!("quick.error.website_failed").as_ref())
    );
    let texts = [node.label(), node.value(), node.description()];
    for text in texts.into_iter().flatten() {
        for secret in ["Nordhafen", "Portal", "intranet.example"] {
            assert!(!text.contains(secret), "{secret} in {text}");
        }
    }
    assert!(
        drawn(cx, panel, "fp-panel-error"),
        "the panel draws the line"
    );
}

/// The line goes with the next key, whatever it is.
#[gpui::test]
fn the_carmine_line_clears_on_the_next_key(cx: &mut TestAppContext) {
    let app = open_app(cx, &[entry("Script portal").url("javascript:alert(1)")]);
    let panel = app.open_panel(cx);
    for next in ["down", "x"] {
        if !app.query(cx).is_empty() {
            keys(cx, panel, "escape");
        }
        type_into(cx, panel, "script portal");
        keys(cx, panel, "enter");
        assert!(error_of(&app, cx).is_some(), "the line shows");
        assert!(drawn(cx, panel, "fp-panel-error"));
        keys(cx, panel, next);
        assert_eq!(error_of(&app, cx), None, "{next} took the line away");
        assert!(!drawn(cx, panel, "fp-panel-error"), "{next}");
    }
}

/// SAP GUI cannot take a password with `&`: Return does not start it, the
/// line above the search says so without the character, the title or the
/// password, the panel stays open and the entry does not count as used. An
/// entry without a password does not start either, and the line says SAP
/// GUI did not start.
#[gpui::test]
fn a_refused_sign_in_shows_a_carmine_line_and_keeps_the_panel_open(cx: &mut TestAppContext) {
    const PASSWORD: &str = "sp&pw-7Qx";
    let app = open_app(
        cx,
        &[
            entry("NHQ 100")
                .user("N.WEISS")
                .password(PASSWORD)
                .field("SAP_HOST", "nhq.example")
                .field("SAP_INSTANCE", "00")
                .field("SAP_CLIENT", "100"),
            entry("NHS 200")
                .user("N.WEISS")
                .field("SAP_HOST", "nhs.example")
                .field("SAP_INSTANCE", "00")
                .field("SAP_CLIENT", "200"),
        ],
    );
    let panel = app.open_panel(cx);
    for (query, failed, said) in [
        (
            "nhq 100",
            PanelError::PasswordRefused,
            tr!("quick.error.password_refused"),
        ),
        (
            "nhs 200",
            PanelError::SignInFailed,
            tr!("quick.error.sign_in_failed"),
        ),
    ] {
        if !app.query(cx).is_empty() {
            keys(cx, panel, "escape");
        }
        type_into(cx, panel, query);
        keys(cx, panel, "enter");
        assert!(launches().is_empty(), "{query}: SAP GUI started");
        assert!(panel_is_open(cx), "{query}: the panel closed");
        assert_eq!(error_of(&app, cx), Some(failed), "{query}");
        let (role, node) = probe::node(error_line(failed));
        assert_eq!(role, Some(Role::Alert));
        assert_eq!(node.label(), Some(said.as_ref()));
        for text in [node.label(), node.value(), node.description()]
            .into_iter()
            .flatten()
        {
            for secret in [PASSWORD, "&", "NHQ", "NHS", "N.WEISS", "nhq.example"] {
                assert!(!text.contains(secret), "{query}: {secret} in {text}");
            }
        }
        assert!(drawn(cx, panel, "fp-panel-error"), "{query}");
        assert!(app.recently_used(cx).is_empty(), "{query}: counted as used");
    }
}

// ------------------------------------------------------ the SAP client --

/// An SAP system whose `SAP_CLIENT` is 1, as a spreadsheet leaves it: SAP
/// GUI gets client 001, so the row names client 001, and "NHP 001" is its
/// exact identifier.
#[gpui::test]
fn a_short_client_shows_and_ranks_as_the_launch_writes_it(cx: &mut TestAppContext) {
    let app = open_app(
        cx,
        &[
            entry("Nordhafen ERP")
                .user("N.WEISS")
                .password("sp-erp")
                .field("SAP_SID", "NHP")
                .field("SAP_HOST", "nhp.example")
                .field("SAP_INSTANCE", "00")
                .field("SAP_CLIENT", "1"),
            entry("NHP 002").password("sp-other"),
        ],
    );
    let panel = app.open_panel(cx);
    type_into(cx, panel, "NHP 001");
    let hit = app
        .vm(cx)
        .read_with(cx, |vm, _| vm.results().first().cloned())
        .expect("a row");
    assert_eq!(app.selected_title(cx).as_deref(), Some("Nordhafen ERP"));
    assert!(hit.exact, "NHP 001 names it exactly");
    let name = hit.parts.accessible_name();
    assert!(
        name.contains(&format!("{} 001", tr!("glossary.sap_client"))),
        "{name}"
    );
    assert!(!name.contains(&format!("{} 1,", tr!("glossary.sap_client"))));
}

/// The footer names the client the launch writes, 001 for a `SAP_CLIENT` of
/// 1, so signing in uses what the footer says.
#[gpui::test]
fn the_footer_names_the_client_the_launch_writes(cx: &mut TestAppContext) {
    let app = open_app(
        cx,
        &[entry("Nordhafen ERP")
            .user("N.WEISS")
            .password("sp-erp")
            .field("SAP_SID", "NHP")
            .field("SAP_HOST", "nhp.example")
            .field("SAP_INSTANCE", "00")
            .field("SAP_CLIENT", " 1 ")],
    );
    let panel = app.open_panel(cx);
    type_into(cx, panel, "nordhafen");
    let words = app
        .vm(cx)
        .read_with(cx, |vm, _| vm.footer().main.map(|(_, words)| words));
    assert_eq!(
        words,
        Some(tr!(
            "quick.footer.sap_login",
            sid = "NHP",
            client = "001",
            user = "N.WEISS"
        ))
    );
}

/// Two entries for NHP, one with `SAP_CLIENT` 1 and one with 001, stand
/// for the same system, and the footer counts them.
#[gpui::test]
fn one_and_001_are_twins(cx: &mut TestAppContext) {
    let app = open_app(
        cx,
        &[
            entry("Nordhafen ERP")
                .user("N.WEISS")
                .password("sp-north")
                .field("SAP_SID", "NHP")
                .field("SAP_HOST", "nhp-north.example")
                .field("SAP_INSTANCE", "00")
                .field("SAP_CLIENT", "1"),
            entry("Nordhafen ERP old")
                .user("K.BRAUN")
                .password("sp-south")
                .field("SAP_SID", "NHP")
                .field("SAP_HOST", "nhp-south.example")
                .field("SAP_INSTANCE", "00")
                .field("SAP_CLIENT", "001"),
        ],
    );
    let panel = app.open_panel(cx);
    type_into(cx, panel, "nordhafen");
    let twins: Vec<usize> = app.vm(cx).read_with(cx, |vm, _| {
        vm.results().iter().map(|hit| hit.twins).collect()
    });
    assert_eq!(twins, [2, 2]);
}

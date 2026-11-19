//! The panel over the other apps: over a hidden FerrisPass, the preview of
//! the selected entry, a search without a match, the line that says what
//! failed, an SAP client written as a spreadsheet leaves it, and the
//! footer's menu.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{Focusable as _, Role, TestAppContext, VisualTestContext};

use super::*;
use crate::app::Overlay;
use crate::platform::activation::{self, fake as hidden};
use crate::ui::ds::components::CopyState;
use crate::ui::ds::components::Shortcut;
use crate::ui::vm::panel::PanelError;
use crate::ui::windows::panel::view::{
    PreviewFlags, PreviewParts, PreviewValues, error_line, no_match, preview, preview_flags,
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

/// The password of the web login, which the tests look for everywhere it
/// must not be.
const SENTINEL: &str = "sp-pw-7Qx";

/// A one-time code secret; the code itself changes every 30 seconds.
const OTP: &str = "otpauth://totp/X?secret=JBSWY3DPEHPK3PXP";

/// A web login with a user name, a password and a one-time code.
fn sales() -> Fixture {
    entry("Salesforce")
        .user("petra.engel@example.com")
        .password(SENTINEL)
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

fn in_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether `code` names `word` as a whole word.
fn names(code: &str, word: &str) -> bool {
    code.match_indices(word).any(|(at, _)| {
        !code[..at].ends_with(in_word) && !code[at + word.len()..].starts_with(in_word)
    })
}

/// Where `keyword` starts in `code` and is not the end of a longer word.
fn starts_of(code: &str, keyword: &str) -> Vec<usize> {
    code.match_indices(keyword)
        .filter(|(at, _)| !code[..*at].ends_with(in_word))
        .map(|(at, _)| at)
        .collect()
}

/// The name that starts `code`.
fn leading_name(code: &str) -> String {
    code.trim_start()
        .chars()
        .take_while(|c| in_word(*c))
        .collect()
}

/// The types of the panel's files that are or hold a copy button:
/// `IconButton`, each name it is brought in under with `as`, and each
/// struct, enum, union or alias whose definition names one of those, until
/// no new one turns up.
fn button_carriers(sources: &[(String, String)]) -> Vec<String> {
    let mut carriers = vec!["IconButton".to_string()];
    loop {
        let known = carriers.len();
        for (_, code) in sources {
            for keyword in ["struct ", "enum ", "union ", "type "] {
                for at in starts_of(code, keyword) {
                    let rest = &code[at + keyword.len()..];
                    let name = leading_name(rest);
                    let definition = match (rest.find(';'), rest.find('{')) {
                        (Some(end), Some(open)) if end < open => &rest[..end],
                        (Some(end), None) => &rest[..end],
                        (_, Some(_)) => probe::braced(rest),
                        (None, None) => rest,
                    };
                    if !name.is_empty()
                        && !carriers.contains(&name)
                        && carriers.iter().any(|carrier| names(definition, carrier))
                    {
                        carriers.push(name);
                    }
                }
            }
            for carrier in carriers.clone() {
                for at in starts_of(code, &format!("{carrier} as ")) {
                    let name = leading_name(&code[at + carrier.len() + " as ".len()..]);
                    if !name.is_empty() && !carriers.contains(&name) {
                        carriers.push(name);
                    }
                }
            }
        }
        if carriers.len() == known {
            return carriers;
        }
    }
}

/// Where the signature that starts `code` ends: the offset of the brace
/// that opens its body, or `None` for a declaration without one. Brackets
/// and parentheses are skipped, so `[u8; 4]` ends nothing.
fn body_start(code: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (at, c) in code.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            '{' if depth == 0 => return Some(at),
            ';' if depth == 0 => return None,
            _ => {}
        }
    }
    None
}

/// Every function and `impl` block of the panel's files a copy button can
/// pass through: a `fn` whose signature names one of `carriers`, and an
/// `impl` whose header does. Each comes with a label, its file and its
/// header, and its body.
fn on_the_buttons_way<'a>(
    sources: &'a [(String, String)],
    carriers: &[String],
) -> Vec<(String, &'a str)> {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/");
    let carries = |text: &str| carriers.iter().any(|carrier| names(text, carrier));
    let mut way = Vec::new();
    for (file, code) in sources {
        let file = file.strip_prefix(root).unwrap_or(file);
        for at in starts_of(code, "fn ") {
            let rest = &code[at..];
            let Some(open) = body_start(rest) else {
                continue;
            };
            let header = &rest[..open];
            if carries(header) {
                let name = header.split('(').next().unwrap_or(header).trim();
                way.push((format!("{file}: {name}("), probe::braced(&rest[open..])));
            }
        }
        for at in starts_of(code, "impl") {
            let line = code[..at].rfind('\n').map_or(0, |end| end + 1);
            let before = code[line..at].trim();
            let item = before.is_empty() || before == "unsafe";
            let rest = &code[at..];
            let follows = rest["impl".len()..].starts_with(|c: char| c.is_whitespace() || c == '<');
            let Some(open) = rest.find('{').filter(|_| item && follows) else {
                continue;
            };
            let header = &rest[..open];
            if carries(header) {
                let header = header.split_whitespace().collect::<Vec<_>>().join(" ");
                way.push((format!("{file}: {header}"), probe::braced(&rest[open..])));
            }
        }
    }
    way
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
/// code as "••• •••": neither reaches the title, the user line or a node of
/// the secret field or the ring, in any spelling, whether VoiceOver runs or
/// not. Every part `preview` hands out is read by name, so a part added to
/// `PreviewParts` does not compile until it is read here; the badge shows
/// the vault and nothing of the entry.
///
/// The two copy buttons write their node only once the design system has
/// turned them into GPUI's, which the panel's tests cannot reach, so they
/// are read in the source of the panel's files, `src/ui/windows` and
/// `src/ui/vm`. Those build exactly two `IconButton`s, both in `preview`,
/// each with a literal id and a name from `tr!` with a literal key. No
/// function there whose signature names `IconButton`, a name it is brought
/// in under, or a struct, enum or alias there that holds one, such as
/// `PreviewParts`, and no `impl` block for one of those, has a description,
/// an accessibility id, a tooltip or an `aria_` property in its body;
/// `preview` and `assemble_preview` are among them. `render_open` hands
/// what `preview` builds straight to `assemble_preview`.
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
            let PreviewParts {
                title,
                badge: _,
                user,
                password,
                code: ring,
            } = preview(
                &model,
                values,
                assistive,
                |_| CopyState::Idle,
                Rc::new(|_, _, _| {}),
            );
            let mut texts = vec![format!("title: {title}")];
            let mut read = |parts: Vec<probe::ComponentPart>| {
                for part in parts {
                    for (name, text) in part.texts() {
                        if let Some(text) = text {
                            texts.push(format!("{} {name}: {text}", part.name));
                        }
                    }
                }
            };
            read(password.expect("the secret field").parts(window, cx));
            // The copy buttons are read in the source, below.
            let (ring, _copy_code) = ring.expect("the ring");
            read(ring.parts(window, cx));
            let (user, _copy_user) = user.expect("the user line");
            texts.push(format!("user line: {user}"));
            (texts, user.to_string(), code)
        });
        assert_eq!(user, "petra.engel@example.com");
        assert!(!texts.is_empty(), "the parts were read");
        let mut forbidden = spellings(&code);
        forbidden.push(SENTINEL.to_string());
        forbidden.push("sp-p w-7Q x".to_string());
        for text in &texts {
            for sentinel in &forbidden {
                assert!(
                    !text.contains(sentinel.as_str()),
                    "assistive {assistive}: {sentinel} in {text}"
                );
            }
        }
    }

    // The copy buttons, in the source.
    let sources = panel_sources();
    let carriers = button_carriers(&sources);
    let way = on_the_buttons_way(&sources, &carriers);
    let read: Vec<&str> = way.iter().map(|(label, _)| label.as_str()).collect();
    for start in ["view.rs: fn preview(", "view.rs: fn assemble_preview("] {
        assert!(
            read.iter().any(|label| label.ends_with(start)),
            "the scan reads {start}: {read:?}"
        );
    }
    let mut handed = Vec::new();
    for (label, body) in &way {
        for call in [
            ".description(",
            "accessibility_",
            "tooltip",
            "aria_",
            "a11y_",
        ] {
            if body.contains(call) {
                handed.push(format!("{label} hands a button {call}"));
            }
        }
    }
    assert!(handed.is_empty(), "{handed:#?}");
    let built: usize = sources
        .iter()
        .flat_map(|(_, code)| {
            carriers
                .iter()
                .map(move |carrier| code.matches(&format!("{carrier}::new(")).count())
        })
        .sum();
    let view = source_of("src/ui/windows/panel/view.rs");
    let body = function(&view, "fn preview(");
    let buttons: Vec<Vec<&str>> = body
        .match_indices("IconButton::new(")
        .map(|(at, call)| probe::call_arguments(body, at + call.len() - 1))
        .collect();
    assert_eq!(
        (built, buttons.len()),
        (2, 2),
        "the two copy buttons, both built in preview: {buttons:?}"
    );
    let literal = |text: &str, prefix: &str, suffix: &str, allowed: &str| {
        text.strip_prefix(prefix)
            .and_then(|text| text.strip_suffix(suffix))
            .is_some_and(|inner| {
                !inner.is_empty()
                    && inner
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || allowed.contains(c))
            })
    };
    for arguments in &buttons {
        let &[id, _glyph, name] = arguments.as_slice() else {
            panic!("a copy button built otherwise: {arguments:?}");
        };
        assert!(
            literal(id, "\"", "\"", "-"),
            "a copy button's id is no literal: {id}"
        );
        assert!(
            literal(name, "tr!(\"", "\")", "._"),
            "a copy button is named otherwise than from tr! with a literal key: {name}"
        );
    }
    let render = squeezed(function(&view, "fn render_open("));
    let (_, after) = render
        .split_once("letparts=preview(")
        .expect("render_open builds the preview");
    assert!(
        after.matches("parts").count() == 1 && after.contains(".child(assemble_preview(parts,"),
        "render_open changes what preview builds before it is put in place: {after}"
    );
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
    assert_eq!(app.copied(), (1, Some(SENTINEL.to_string())));
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
        for word in ["Nordhafen", "Portal", "intranet.example"] {
            assert!(!text.contains(word), "{word} in {text}");
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
    const REFUSED: &str = "sp&pw-7Qx";
    let app = open_app(
        cx,
        &[
            entry("NHQ 100")
                .user("N.WEISS")
                .password(REFUSED)
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
            for word in [REFUSED, "&", "NHQ", "NHS", "N.WEISS", "nhq.example"] {
                assert!(!text.contains(word), "{query}: {word} in {text}");
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

// ------------------------------------------------------- the footer menu --

/// The menu of a web login lists its main action, its copies, showing it in
/// FerrisPass and the app's commands, in the order of the command table;
/// with nothing selected only the app's commands, and while locked only the
/// settings and closing. While Return waits for a production system to be
/// picked, the main action is left out.
#[gpui::test]
fn the_more_menu_lists_the_panel_commands_enabled_for_the_selection(cx: &mut TestAppContext) {
    let mut entries = sample();
    entries[1].starred = true;
    entries[2].starred = false;
    let app = open_app(cx, &entries);
    let panel = app.open_panel(cx);
    let menu = |cx: &mut TestAppContext| app.vm(cx).read_with(cx, |vm, _| vm.menu());
    let app_commands = [
        CommandId::APP_LOCK_ALL,
        CommandId::APP_SETTINGS,
        CommandId::PANEL_CLOSE,
    ];

    // The empty search starts on NHP 100, a production system.
    assert_eq!(app.selected_title(cx).as_deref(), Some("NHP 100"));
    let waiting = menu(cx);
    assert!(
        !waiting.contains(&CommandId::ENTRY_SAP_LOGIN),
        "{waiting:?}"
    );
    assert!(waiting.contains(&CommandId::ENTRY_COPY_PASSWORD));

    type_into(cx, panel, "webmail");
    let mut expected = vec![
        CommandId::ENTRY_OPEN_WEBSITE,
        CommandId::ENTRY_COPY_PASSWORD,
        CommandId::ENTRY_COPY_USERNAME,
        CommandId::ENTRY_COPY_URL,
        CommandId::ENTRY_OPEN_URL,
        CommandId::ENTRY_SHOW_IN_APP,
    ];
    expected.extend(app_commands);
    assert_eq!(menu(cx), expected);

    keys(cx, panel, "escape");
    type_into(cx, panel, "zzz");
    assert_eq!(menu(cx), app_commands, "nothing selected");

    // Locked, nothing can be locked and no entry is offered.
    assert_eq!(
        PanelVm::default().menu(),
        [CommandId::APP_SETTINGS, CommandId::PANEL_CLOSE],
        "the locked panel"
    );
}

/// Every row of the menu is the pair `Shortcut::for_command` makes from the
/// command table: the command's first keys and its word. No row is written
/// by hand.
#[gpui::test]
fn every_menu_row_is_a_shortcut_pair_from_the_command_table(cx: &mut TestAppContext) {
    let app = open_app(cx, &[sales()]);
    let panel = app.open_panel(cx);
    type_into(cx, panel, "sales");
    let commands = app.vm(cx).read_with(cx, |vm, _| vm.menu());
    assert!(commands.len() > 3, "{commands:?}");
    for command in commands {
        let spec = crate::app::registry::command(command).expect("a row in the table");
        let row = crate::ui::windows::panel::view::menu_row(command);
        let table = Shortcut::for_command(command);
        assert_eq!(row.label(), &(spec.label)(), "{}", command.as_str());
        assert_eq!(row.keys_text(), table.keys_text(), "{}", command.as_str());
    }
    let view = squeezed(&source_of("src/ui/windows/panel/view.rs"));
    let row = function(&view, "fnmenu_row(");
    assert_eq!(row, "{Shortcut::for_command(command)}");
    let menu = function(&view, "fnfooter_menu(");
    assert!(menu.contains("letpair=menu_row(command);"), "{menu}");
    assert!(menu.contains(".child(pair)"), "{menu}");
    assert!(!menu.contains("from_keystroke"), "{menu}");
}

/// The footer's menu button opens a popover over the footer and says that
/// the menu is open; the search keeps the keyboard. A click on a row runs
/// its command, Escape closes the menu before it clears the search.
#[gpui::test]
fn menu_open_is_set_while_the_popover_shows(cx: &mut TestAppContext) {
    let app = open_app(cx, &[sales()]);
    let panel = app.open_panel(cx);
    type_into(cx, panel, "sales");
    let vm = app.vm(cx);
    assert!(!drawn(cx, panel, "fp-panel-menu"));
    cx.update(super::super::toggle_menu);
    cx.run_until_parked();
    assert!(vm.read_with(cx, |vm, _| vm.menu_open()));
    assert!(drawn(cx, panel, "fp-panel-menu"), "the popover shows");
    let expanded = probe::in_frame(cx, {
        let vm = vm.clone();
        move |window, cx| {
            let model = vm.read(cx).footer();
            let parts = crate::ui::ds::components::PanelFooter::parts(
                || footer(&model).menu_open(true),
                window,
                cx,
            );
            parts
                .into_iter()
                .find(|part| part.name == "menu")
                .and_then(|part| part.node.is_expanded())
        }
    });
    assert_eq!(expanded, Some(true));
    let view = squeezed(&source_of("src/ui/windows/panel/view.rs"));
    let render = function(&view, "fnrender(&mutself");
    assert!(
        render.contains("(error,menu_open,commands)=")
            && render.contains("vm.menu_open()")
            && render.contains(".child(footer.menu_open(menu_open))"),
        "render does not tell the footer that the menu is open: {render}"
    );
    let search_focused = panel
        .update(cx, |_, window, cx| {
            let view = view_for_test(cx).expect("the view");
            let search = view.read(cx).search.clone();
            search.read(cx).focus_handle(cx).is_focused(window)
        })
        .expect("the panel");
    assert!(search_focused, "the search keeps the keyboard");

    keys(cx, panel, "escape");
    assert!(
        !vm.read_with(cx, |vm, _| vm.menu_open()),
        "Escape closes it"
    );
    assert_eq!(app.query(cx), "sales", "and leaves the search");
    assert!(!drawn(cx, panel, "fp-panel-menu"));

    // A row runs its command: copy the user name.
    cx.update(super::super::toggle_menu);
    cx.run_until_parked();
    let rows = vm.read_with(cx, |vm, _| vm.menu());
    let index = rows
        .iter()
        .position(|command| *command == CommandId::ENTRY_COPY_USERNAME)
        .expect("copy the user name");
    let selector: &'static str = Box::leak(format!("fp-panel-menu-row-{index}").into_boxed_str());
    click(cx, panel, selector);
    assert_eq!(
        app.copied(),
        (1, Some("petra.engel@example.com".to_string()))
    );
    assert!(!vm.read_with(cx, |vm, _| vm.menu_open()), "the menu closed");
    assert!(panel_is_open(cx));
}

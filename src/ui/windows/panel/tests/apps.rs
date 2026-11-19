//! The panel over the other apps: over a hidden FerrisPass.

use gpui::TestAppContext;

use super::*;
use crate::platform::activation::{self, fake as hidden};

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

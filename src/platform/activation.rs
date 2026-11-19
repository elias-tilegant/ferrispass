//! Showing the quick access panel while FerrisPass is hidden.
//!
//! macOS shows no window of a hidden app. Opening the panel over a hidden
//! FerrisPass, after Cmd+H or after Cmd+W hid it, unhides the whole app, and
//! the main window comes back on screen with the panel. So when the app is
//! hidden, the panel takes these steps, in this order:
//!
//! 1. [`app_is_hidden`] says whether it is; only then do the next steps run.
//! 2. [`unhide_without_activation`] unhides the app without making it the
//!    active app, so the app the user works in stays in front.
//! 3. [`order_out_app_windows`] orders out again every window the unhide
//!    brought back: the windows of the app that are on screen, hide with the
//!    app and are not in the Dock. In FerrisPass that is the main window. It
//!    runs before the panel's own window exists, so the panel is never one
//!    of them.
//! 4. When the panel closes, [`hide_app`] hides the app again. When it closes
//!    because the user asked for the main window (Cmd+O, Unlock, New, the
//!    menu's Settings), the app is not hidden again: [`order_front`] puts
//!    back the windows step 3 took away, and the panel activates the app as
//!    it always does for that.
//!
//! A visible, minimized or windowless app needs none of this: there the panel
//! opens and closes as it always does.
//!
//! Every call here is a safe AppKit call through objc2-app-kit, and each one
//! first makes sure it runs on the main thread (`MainThreadMarker::new()`);
//! off the main thread it does nothing and [`app_is_hidden`] says no. No
//! window handle or pointer comes from GPUI: the windows are the ones
//! `NSApplication` lists, each held only for the call and released after it.
//! The AppKit types come with the objc2-app-kit features GPUI and the GPUI Kit
//! turn on for their own use of the crate.
//!
//! Tests put a fake in place of AppKit for the whole test build
//! (`fake::hide_for_test`): the fake counts the calls, and hiding the app,
//! which the test platform cannot do, only marks it hidden.

#![forbid(unsafe_code)]

use gpui::App;

/// The windows [`order_out_app_windows`] took off the screen, by their
/// window numbers, so that [`order_front`] can put them back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeptOut(Vec<isize>);

/// Whether FerrisPass is hidden, as after Cmd+H. `false` off the main thread
/// and on other platforms.
pub fn app_is_hidden() -> bool {
    system::app_is_hidden()
}

/// Unhides FerrisPass without making it the active app.
pub fn unhide_without_activation() {
    system::unhide_without_activation();
}

/// Orders out every window of the app that is on screen, hides with the app
/// and is not minimized, and says which.
pub fn order_out_app_windows() -> KeptOut {
    KeptOut(system::order_out_app_windows())
}

/// Puts the windows of `kept` back on screen, in front.
pub fn order_front(kept: &KeptOut) {
    system::order_front(&kept.0);
}

/// Hides FerrisPass again, as Cmd+H does.
pub fn hide_app(cx: &mut App) {
    system::hide_app(cx);
}

/// AppKit, on the main thread.
#[cfg(all(target_os = "macos", not(test)))]
mod system {
    use gpui::App;
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;

    pub(super) fn app_is_hidden() -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        NSApplication::sharedApplication(mtm).isHidden()
    }

    pub(super) fn unhide_without_activation() {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        NSApplication::sharedApplication(mtm).unhideWithoutActivation();
    }

    pub(super) fn order_out_app_windows() -> Vec<isize> {
        let Some(mtm) = MainThreadMarker::new() else {
            return Vec::new();
        };
        let mut kept = Vec::new();
        for window in NSApplication::sharedApplication(mtm).windows().iter() {
            if window.isVisible() && window.canHide() && !window.isMiniaturized() {
                kept.push(window.windowNumber());
                window.orderOut(None);
            }
        }
        kept
    }

    pub(super) fn order_front(kept: &[isize]) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        for window in NSApplication::sharedApplication(mtm).windows().iter() {
            if kept.contains(&window.windowNumber()) {
                window.orderFront(None);
            }
        }
    }

    pub(super) fn hide_app(cx: &mut App) {
        cx.hide();
    }
}

/// Elsewhere an app is never hidden this way, and there is nothing to do.
#[cfg(all(not(target_os = "macos"), not(test)))]
mod system {
    use gpui::App;

    pub(super) fn app_is_hidden() -> bool {
        false
    }

    pub(super) fn unhide_without_activation() {}

    pub(super) fn order_out_app_windows() -> Vec<isize> {
        Vec::new()
    }

    pub(super) fn order_front(_: &[isize]) {}

    pub(super) fn hide_app(_: &mut App) {}
}

#[cfg(test)]
use fake as system;

/// The stand-in for AppKit in the test build. Its state belongs to the
/// thread, so each test sees only its own calls.
#[cfg(test)]
pub(crate) mod fake {
    use std::cell::Cell;

    use gpui::App;

    /// How often each step ran.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub(crate) struct Calls {
        pub(crate) unhides: usize,
        pub(crate) order_outs: usize,
        pub(crate) order_fronts: usize,
        pub(crate) hides: usize,
    }

    /// The number the fake gives the main window.
    pub(crate) const MAIN_WINDOW: isize = 1;

    thread_local! {
        static HIDDEN: Cell<bool> = const { Cell::new(false) };
        static CALLS: Cell<Calls> = const {
            Cell::new(Calls { unhides: 0, order_outs: 0, order_fronts: 0, hides: 0 })
        };
    }

    /// A visible app and no calls, as at the start of a test.
    pub(crate) fn reset() {
        HIDDEN.set(false);
        CALLS.set(Calls::default());
    }

    /// The app is hidden now, as after Cmd+H.
    pub(crate) fn hide_for_test() {
        HIDDEN.set(true);
    }

    pub(crate) fn calls() -> Calls {
        CALLS.get()
    }

    fn count(step: impl FnOnce(&mut Calls)) {
        let mut calls = CALLS.get();
        step(&mut calls);
        CALLS.set(calls);
    }

    pub(super) fn app_is_hidden() -> bool {
        HIDDEN.get()
    }

    pub(super) fn unhide_without_activation() {
        HIDDEN.set(false);
        count(|calls| calls.unhides += 1);
    }

    pub(super) fn order_out_app_windows() -> Vec<isize> {
        count(|calls| calls.order_outs += 1);
        vec![MAIN_WINDOW]
    }

    pub(super) fn order_front(_: &[isize]) {
        count(|calls| calls.order_fronts += 1);
    }

    pub(super) fn hide_app(_: &mut App) {
        HIDDEN.set(true);
        count(|calls| calls.hides += 1);
    }
}

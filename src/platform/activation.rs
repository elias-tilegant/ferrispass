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
//! 4. When the panel has closed, [`order_front`] puts back the windows step
//!    3 took away, and [`hide_app`] hides the app again right after. An
//!    unhide brings back only the windows that were on screen when the app
//!    was hidden, so a window still ordered out at the hide would stay off
//!    the screen when the user brings the app back with a click in the Dock
//!    or Cmd+Tab, and the next panel over the hidden app would find it
//!    missing too. When the panel closes because the user asked for the
//!    main window (Cmd+O, Unlock, New, the menu's Settings), the app is not
//!    hidden again: [`order_front`] puts back the windows, and the panel
//!    activates the app as it always does for that.
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
//! (`fake::hide_for_test`): the fake notes the calls in the order they come,
//! and in place of hiding the app, which the test platform cannot do, it
//! keeps AppKit's rule for which windows a hide takes along and an unhide
//! brings back.

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
///
/// It notes each call in the order it comes, and keeps AppKit's rule for a
/// hidden app as far as the panel meets it: hiding takes the app's windows
/// that are on the screen off it, and unhiding brings back exactly those,
/// not a window that was already ordered out when the app was hidden. The
/// app has one window, the main window, on the screen until a test hides it.
#[cfg(test)]
pub(crate) mod fake {
    use std::cell::{Cell, RefCell};

    use gpui::App;

    /// One call into `activation`, as the fake noted it.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum Step {
        Unhide,
        OrderOut,
        OrderFront,
        /// Hiding the app, with the number of windows GPUI listed then.
        Hide {
            windows: usize,
        },
    }

    /// The number the fake gives the main window.
    pub(crate) const MAIN_WINDOW: isize = 1;

    thread_local! {
        static HIDDEN: Cell<bool> = const { Cell::new(false) };
        /// The app's windows on the screen.
        static ON_SCREEN: RefCell<Vec<isize>> = RefCell::new(vec![MAIN_WINDOW]);
        /// The windows the last hide took off the screen.
        static BACK_ON_UNHIDE: RefCell<Vec<isize>> = const { RefCell::new(Vec::new()) };
        static STEPS: RefCell<Vec<Step>> = const { RefCell::new(Vec::new()) };
    }

    /// A visible app with its main window on the screen, and no calls, as
    /// at the start of a test.
    pub(crate) fn reset() {
        HIDDEN.set(false);
        ON_SCREEN.set(vec![MAIN_WINDOW]);
        BACK_ON_UNHIDE.set(Vec::new());
        STEPS.set(Vec::new());
    }

    /// The user hides the app, as Cmd+H does.
    pub(crate) fn hide_for_test() {
        hide();
    }

    /// The user brings the hidden app back, as a click in the Dock or
    /// Cmd+Tab does.
    pub(crate) fn unhide_for_test() {
        unhide();
    }

    /// The calls so far, in the order they came.
    pub(crate) fn steps() -> Vec<Step> {
        STEPS.with_borrow(Clone::clone)
    }

    /// The app's windows on the screen; none while the app is hidden.
    pub(crate) fn on_screen() -> Vec<isize> {
        ON_SCREEN.with_borrow(Clone::clone)
    }

    fn note(step: Step) {
        STEPS.with_borrow_mut(|steps| steps.push(step));
    }

    /// AppKit's hide: the windows on the screen go off it, and are the ones
    /// the next unhide brings back.
    fn hide() {
        if !HIDDEN.replace(true) {
            BACK_ON_UNHIDE.set(ON_SCREEN.take());
        }
    }

    fn unhide() {
        if HIDDEN.replace(false) {
            let back = BACK_ON_UNHIDE.take();
            put_on_screen(&back);
        }
    }

    fn put_on_screen(windows: &[isize]) {
        ON_SCREEN.with_borrow_mut(|shown| {
            for window in windows {
                if !shown.contains(window) {
                    shown.push(*window);
                }
            }
        });
    }

    pub(super) fn app_is_hidden() -> bool {
        HIDDEN.get()
    }

    pub(super) fn unhide_without_activation() {
        note(Step::Unhide);
        unhide();
    }

    pub(super) fn order_out_app_windows() -> Vec<isize> {
        note(Step::OrderOut);
        ON_SCREEN.take()
    }

    pub(super) fn order_front(kept: &[isize]) {
        note(Step::OrderFront);
        put_on_screen(kept);
    }

    pub(super) fn hide_app(cx: &mut App) {
        note(Step::Hide {
            windows: cx.windows().len(),
        });
        hide();
    }
}

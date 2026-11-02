//! The auto-lock reads the app-wide idle clock. Input in the window moves
//! it, the TOTP tick and the auto-lock check don't, and the pointer counts
//! only while the window is in front.
use super::*;
use crate::app::services::activity;
use crate::app::services::clipboard::ClipboardService;
use crate::app::{LibrarySelection, VaultStatus};
use crate::clipboard::{AccessBehavior, fake::FakePasteboard};
use crate::keepass::VaultDocument;
use gpui::{
    Modifiers, MouseButton, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase,
    VisualTestContext, WindowHandle, point,
};
use gpui_component::Root;
use std::cell::Cell;
use std::rc::Rc;

const VAULT: &str = "/tmp/activity-clock.kdbx";
const TEN_MINUTES: Duration = Duration::from_secs(600);

struct Harness {
    state: Entity<AppState>,
    shell: Entity<AppShell>,
    window: WindowHandle<Root>,
}

/// An open vault with one TOTP entry, selected, so the TOTP tick runs, and
/// the auto-lock set to `auto_lock_secs`. The window is not in front.
fn open_vault(cx: &mut TestAppContext, auto_lock_secs: Option<u64>) -> Harness {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::app::actions::init(cx);
        ClipboardService::install_for_test(
            cx,
            Box::new(FakePasteboard::new(AccessBehavior::Default)),
        );
    });
    let mut database = ::keepass::db::Database::new();
    {
        let mut root = database.root_mut();
        let mut entry = root.add_entry();
        entry.set_unprotected(::keepass::db::fields::TITLE, "Two factor");
        entry.set_protected(
            ::keepass::db::fields::OTP,
            "otpauth://totp/X?secret=JBSWY3DPEHPK3PXP",
        );
    }
    let snapshot = crate::keepass::repository::snapshot_from_database(&database);
    let state = cx.new(|_| {
        let mut state = AppState::default();
        state.open_for_test(
            PathBuf::from(VAULT),
            VaultDocument::new(database, snapshot, "pw".to_string(), None),
        );
        state
    });
    let mut shell = None;
    let window = cx.add_window(|window, cx| {
        let view = cx.new(|cx| {
            let mut shell = AppShell::new(state.clone(), window, cx);
            shell.settings.auto_lock_secs = auto_lock_secs;
            shell
        });
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    let shell = shell.expect("shell view");
    // Showing the TOTP entries selects the only one. It is the first state
    // change, so it also starts the auto-lock task.
    state.update(cx, |state, cx| {
        state.select_library(LibrarySelection::TotpEnabled, cx)
    });
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| {
        assert!(shell.totp_tick.is_some(), "the TOTP tick has to run");
        assert_eq!(shell.auto_lock_task.is_some(), auto_lock_secs.is_some());
    });
    Harness {
        state,
        shell,
        window,
    }
}

fn is_open(cx: &mut TestAppContext, harness: &Harness) -> bool {
    harness.state.read_with(cx, |state, _| {
        matches!(state.vault_status(), VaultStatus::Open { .. })
    })
}

fn advance(cx: &mut TestAppContext, seconds: u64) {
    for _ in 0..seconds {
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
    }
}

fn move_pointer(cx: &mut TestAppContext, harness: &Harness) {
    let mut window = VisualTestContext::from_window(harness.window.into(), cx);
    window.simulate_mouse_move(point(px(40.), px(40.)), None, Modifiers::none());
    window.simulate_mouse_move(point(px(60.), px(80.)), None, Modifiers::none());
}

#[gpui::test]
fn key_press_in_the_window_resets_the_clock(cx: &mut TestAppContext) {
    let harness = open_vault(cx, Some(240));
    activity::reset(SystemTime::now() - TEN_MINUTES);

    cx.simulate_keystrokes(harness.window.into(), "x");
    cx.run_until_parked();

    let idle = activity::idle_for(SystemTime::now());
    assert!(idle < Duration::from_secs(60), "a key press left {idle:?}");
}

#[gpui::test]
fn background_tick_does_not_reset_the_clock(cx: &mut TestAppContext) {
    // The auto-lock check runs too, with a threshold far off.
    let harness = open_vault(cx, Some(24 * 60 * 60));
    let notifications = Rc::new(Cell::new(0));
    cx.update({
        let notifications = notifications.clone();
        |cx| {
            cx.observe(&harness.state, move |_, _| {
                notifications.set(notifications.get() + 1);
            })
            .detach();
        }
    });
    let last_input = SystemTime::now() - TEN_MINUTES;
    activity::reset(last_input);

    advance(cx, 30);

    assert!(
        notifications.get() >= 30,
        "the TOTP tick ran {} times in 30 seconds",
        notifications.get()
    );
    let now = SystemTime::now();
    assert_eq!(
        activity::idle_for(now),
        now.duration_since(last_input).unwrap(),
        "a background tick moved the idle clock"
    );
    assert!(is_open(cx, &harness));
}

#[gpui::test]
fn pointer_counts_only_while_the_window_is_in_front(cx: &mut TestAppContext) {
    let harness = open_vault(cx, Some(240));
    let last_input = SystemTime::now() - TEN_MINUTES;
    activity::reset(last_input);

    // Another app is in front; the pointer only passes over the window.
    let in_front = AnyWindowHandle::from(harness.window)
        .update(cx, |_, window, _| window.is_window_active())
        .unwrap();
    assert!(!in_front);
    move_pointer(cx, &harness);
    let now = SystemTime::now();
    assert_eq!(
        activity::idle_for(now),
        now.duration_since(last_input).unwrap(),
        "the pointer over a window behind another app moved the clock"
    );

    AnyWindowHandle::from(harness.window)
        .update(cx, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
    move_pointer(cx, &harness);
    let idle = activity::idle_for(SystemTime::now());
    assert!(
        idle < Duration::from_secs(60),
        "the pointer over the window in front left {idle:?}"
    );
}

#[gpui::test]
fn auto_lock_locks_once_the_clock_passes_the_threshold(cx: &mut TestAppContext) {
    let harness = open_vault(cx, Some(240));

    activity::reset(SystemTime::now() - Duration::from_secs(180));
    advance(cx, AUTO_LOCK_TICK_SECS);
    assert!(is_open(cx, &harness), "locked after three idle minutes");

    activity::reset(SystemTime::now() - Duration::from_secs(241));
    advance(cx, AUTO_LOCK_TICK_SECS);
    assert!(!is_open(cx, &harness), "still open after four idle minutes");
    harness
        .shell
        .read_with(cx, |shell, _| assert!(shell.totp_tick.is_none()));
}

#[gpui::test]
fn auto_lock_starts_counting_when_the_vault_opens(cx: &mut TestAppContext) {
    // Nothing touched the clock before the vault opened, so without a new
    // baseline it reads as idle for decades.
    let harness = open_vault(cx, Some(240));
    advance(cx, 2 * AUTO_LOCK_TICK_SECS);
    assert!(is_open(cx, &harness), "locked on the first check");
}

fn assert_clock_moved(what: &str) {
    let idle = activity::idle_for(SystemTime::now());
    assert!(idle < Duration::from_secs(60), "{what} left {idle:?}");
}

#[gpui::test]
fn shortcut_in_the_window_resets_the_clock(cx: &mut TestAppContext) {
    let harness = open_vault(cx, Some(240));
    activity::reset(SystemTime::now() - TEN_MINUTES);

    // Cmd+F is bound to an action. The action takes the key before any
    // element's key listener sees it.
    cx.simulate_keystrokes(harness.window.into(), "cmd-f");
    cx.run_until_parked();

    assert_clock_moved("a shortcut");
}

#[gpui::test]
fn click_in_the_window_resets_the_clock(cx: &mut TestAppContext) {
    let harness = open_vault(cx, Some(240));
    activity::reset(SystemTime::now() - TEN_MINUTES);

    let mut window = VisualTestContext::from_window(harness.window.into(), cx);
    window.simulate_mouse_down(
        point(px(40.), px(40.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    window.simulate_mouse_up(
        point(px(40.), px(40.)),
        MouseButton::Left,
        Modifiers::none(),
    );

    assert_clock_moved("a click without moving the pointer");
}

#[gpui::test]
fn scrolling_in_the_window_resets_the_clock(cx: &mut TestAppContext) {
    let harness = open_vault(cx, Some(240));
    activity::reset(SystemTime::now() - TEN_MINUTES);

    let mut window = VisualTestContext::from_window(harness.window.into(), cx);
    window.simulate_event(ScrollWheelEvent {
        position: point(px(300.), px(200.)),
        delta: ScrollDelta::Lines(point(0., 3.)),
        modifiers: Modifiers::none(),
        touch_phase: TouchPhase::Moved,
    });

    assert_clock_moved("scrolling");
}

#[gpui::test]
fn pointer_over_a_dialog_resets_the_clock(cx: &mut TestAppContext) {
    let harness = open_vault(cx, Some(240));
    AnyWindowHandle::from(harness.window)
        .update(cx, |_, window, _| window.activate_window())
        .unwrap();
    // The dialog blocks the pointer for everything below it, the window's
    // root element included.
    cx.dispatch_action(harness.window.into(), OpenAbout);
    cx.run_until_parked();
    assert!(
        harness
            .state
            .read_with(cx, |state, _| matches!(state.overlay(), Overlay::About))
    );
    activity::reset(SystemTime::now() - TEN_MINUTES);

    move_pointer(cx, &harness);

    assert_clock_moved("the pointer over a dialog");
}

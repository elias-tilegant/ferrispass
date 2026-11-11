//! The auto-lock reads the app-wide idle clock. Input in the window moves
//! it, the TOTP tick and the auto-lock check don't, and the pointer counts
//! only while the window is in front. With the Mac idle setting on, the
//! Mac's idle time decides instead, and the app's clock with the Mac
//! threshold when macOS cannot report it. With it off, the Mac's idle time
//! plays no part.
use super::*;
use crate::app::services::activity;
use crate::app::services::clipboard::ClipboardService;
use crate::app::{LibrarySelection, VaultStatus};
use crate::clipboard::{AccessBehavior, fake::FakePasteboard};
use crate::keepass::VaultDocument;
use crate::platform::system_idle;
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
    open_vault_with(cx, auto_lock_secs, None)
}

/// The same, with the Mac idle setting at `mac_idle_lock_secs`. Both are set
/// here, whatever the settings file on this Mac says.
fn open_vault_with(
    cx: &mut TestAppContext,
    auto_lock_secs: Option<u64>,
    mac_idle_lock_secs: Option<u64>,
) -> Harness {
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
            shell.settings.mac_idle_lock_secs = mac_idle_lock_secs;
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
        assert_eq!(
            shell.auto_lock_task.is_some(),
            auto_lock_secs.is_some() || mac_idle_lock_secs.is_some(),
            "the auto-lock task runs exactly when one of the two settings is on"
        );
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

const AUTO_LOCK: Option<u64> = Some(240);
const MAC_IDLE_LOCK: Option<u64> = Some(15 * 60);
const FIVE_MINUTES: Duration = Duration::from_secs(5 * 60);
const FIFTEEN_MINUTES: Duration = Duration::from_secs(15 * 60);

/// From here on the Mac reports `idle` as its idle time.
fn mac_idle_for(cx: &mut TestAppContext, idle: Duration) {
    cx.update(|cx| system_idle::fake::install_idle(cx, idle));
}

/// From here on the app's own clock reads as idle for `idle`.
fn app_idle_for(idle: Duration) {
    activity::reset(SystemTime::now() - idle);
}

#[gpui::test]
fn auto_lock_without_mac_idle_ignores_the_mac(cx: &mut TestAppContext) {
    // Someone at work in another app, with the Mac idle setting off: the
    // Mac's idle time plays no part, FerrisPass's own clock decides alone.
    // Without the fake the tick could not tell the two clocks apart, since a
    // test build answers `Unsupported` and the fallback reads the app clock.
    mac_idle_for(cx, Duration::ZERO);
    let harness = open_vault_with(cx, AUTO_LOCK, None);

    app_idle_for(Duration::from_secs(180));
    advance(cx, AUTO_LOCK_TICK_SECS);
    assert!(is_open(cx, &harness), "locked after three idle minutes");

    app_idle_for(Duration::from_secs(241));
    advance(cx, AUTO_LOCK_TICK_SECS);
    assert!(
        !is_open(cx, &harness),
        "input in another app held off the lock while the Mac idle setting was off"
    );
}

#[gpui::test]
fn auto_lock_with_mac_idle_ignores_the_app_clock(cx: &mut TestAppContext) {
    // Someone at work in another app: the Mac is busy, FerrisPass is not.
    mac_idle_for(cx, Duration::ZERO);
    let harness = open_vault_with(cx, AUTO_LOCK, MAC_IDLE_LOCK);

    // Past FerrisPass's own four minutes.
    app_idle_for(FIVE_MINUTES);
    advance(cx, AUTO_LOCK_TICK_SECS);
    assert!(
        is_open(cx, &harness),
        "locked by the app's own threshold while the Mac was in use"
    );

    // Past the Mac threshold too, on the app's clock.
    app_idle_for(FIFTEEN_MINUTES + FIVE_MINUTES);
    advance(cx, AUTO_LOCK_TICK_SECS);
    assert!(
        is_open(cx, &harness),
        "locked by the app's clock while the Mac was in use"
    );
}

#[gpui::test]
fn auto_lock_with_mac_idle_locks_when_the_mac_is_idle(cx: &mut TestAppContext) {
    mac_idle_for(cx, FIFTEEN_MINUTES - Duration::from_secs(1));
    let harness = open_vault_with(cx, AUTO_LOCK, MAC_IDLE_LOCK);
    advance(cx, AUTO_LOCK_TICK_SECS);
    assert!(
        is_open(cx, &harness),
        "locked before the Mac was idle for fifteen minutes"
    );

    mac_idle_for(cx, FIFTEEN_MINUTES);
    advance(cx, AUTO_LOCK_TICK_SECS);
    assert!(
        !is_open(cx, &harness),
        "still open after the Mac was idle for fifteen minutes"
    );
}

#[gpui::test]
fn auto_lock_with_mac_idle_falls_back_when_macos_cannot_say(cx: &mut TestAppContext) {
    // A reading that is not a number of seconds, which `idle_for` turns into
    // an error.
    cx.update(|cx| system_idle::fake::install(cx, Ok(f64::NAN)));
    let harness = open_vault_with(cx, AUTO_LOCK, MAC_IDLE_LOCK);

    // Past the app's own four minutes, short of the Mac threshold, which is
    // the one that applies.
    app_idle_for(FIVE_MINUTES);
    advance(cx, AUTO_LOCK_TICK_SECS);
    assert!(
        is_open(cx, &harness),
        "locked by the app's own threshold while the Mac idle setting was on"
    );

    app_idle_for(FIFTEEN_MINUTES);
    advance(cx, AUTO_LOCK_TICK_SECS);
    assert!(
        !is_open(cx, &harness),
        "an unreadable Mac idle time postponed the lock"
    );
}

#[gpui::test]
fn the_auto_lock_task_runs_with_only_the_alternative_set(cx: &mut TestAppContext) {
    mac_idle_for(cx, Duration::ZERO);
    // `open_vault_with` fails if the task does not start.
    let harness = open_vault_with(cx, None, MAC_IDLE_LOCK);
    advance(cx, AUTO_LOCK_TICK_SECS);
    assert!(is_open(cx, &harness), "locked while the Mac was in use");

    mac_idle_for(cx, FIFTEEN_MINUTES);
    advance(cx, AUTO_LOCK_TICK_SECS);
    assert!(
        !is_open(cx, &harness),
        "the Mac idle setting alone did not lock"
    );
}

/// Sleep, the lock screen and the screen saver lock at once, whatever the
/// idle clocks say. The event comes from a fake in place of macOS's
/// notifications, through the task the shell starts when it is built.
#[cfg(target_os = "macos")]
#[gpui::test]
fn a_session_lock_event_still_locks_with_the_alternative(cx: &mut TestAppContext) {
    let session = cx.update(super::tasks::fake_session_lock::FakeSessionLock::install);
    mac_idle_for(cx, Duration::ZERO);
    let harness = open_vault_with(cx, AUTO_LOCK, MAC_IDLE_LOCK);
    advance(cx, AUTO_LOCK_TICK_SECS);
    assert!(is_open(cx, &harness));

    session.lock();
    cx.run_until_parked();
    assert!(
        !is_open(cx, &harness),
        "a screen lock left the vault open while the Mac idle setting was on"
    );
}

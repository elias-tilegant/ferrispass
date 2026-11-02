//! The master password field holds secure event input while it has focus,
//! and every way of leaving it lets go: blur, another app in front, a lock
//! and a successful unlock. A fake stands in for Carbon.
use super::*;
use crate::app::VaultStatus;
use crate::app::services::clipboard::ClipboardService;
use crate::clipboard::{AccessBehavior, fake::FakePasteboard};
use crate::domain::{VaultGroup, VaultSnapshot};
use crate::keepass::VaultDocument;
use crate::platform::secure_input::OsStatus;
use crate::platform::secure_input::fake::{Fake, FakeApi};
use gpui::{TestAppContext, VisualTestContext, WindowHandle};
use gpui_component::Root;
use std::cell::RefCell;
use std::rc::Rc;

const VAULT: &str = "/tmp/secure-input-field.kdbx";

struct Harness {
    state: Entity<AppState>,
    shell: Entity<AppShell>,
    window: WindowHandle<Root>,
    fake: Fake,
}

/// The unlock screen for a vault that is not on disk, in the active window,
/// with `api` in place of Carbon. Nothing has focus yet.
fn unlock_screen(cx: &mut TestAppContext, api: FakeApi) -> Harness {
    let fake = cx.update(|cx| {
        gpui_component::init(cx);
        crate::app::actions::init(cx);
        ClipboardService::install_for_test(
            cx,
            Box::new(FakePasteboard::new(AccessBehavior::Default)),
        );
        secure_input::fake::install(cx, api)
    });
    let state = cx.new(|cx| {
        let mut state = AppState::default();
        state.request_password(PathBuf::from(VAULT), cx);
        state
    });
    let mut shell = None;
    let window = cx.add_window(|window, cx| {
        let view = cx.new(|cx| AppShell::new(state.clone(), window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    let harness = Harness {
        state,
        shell: shell.expect("shell view"),
        window,
        fake,
    };
    activate(cx, &harness);
    assert!(
        harness.state.read_with(cx, |state, _| matches!(
            state.vault_status(),
            VaultStatus::AwaitingPassword { .. }
        )),
        "the test needs the unlock screen"
    );
    harness
}

/// Runs `f` on the shell inside its window and returns before the next frame
/// is drawn, so no focus change has been reported yet.
fn in_window<R>(
    cx: &mut TestAppContext,
    harness: &Harness,
    f: impl FnOnce(&mut AppShell, &mut Window, &mut Context<AppShell>) -> R,
) -> R {
    let shell = harness.shell.clone();
    AnyWindowHandle::from(harness.window)
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| f(shell, window, cx))
        })
        .unwrap()
}

fn focus_password(cx: &mut TestAppContext, harness: &Harness) {
    in_window(cx, harness, |shell, window, cx| {
        shell
            .password_input
            .update(cx, |input, cx| input.focus(window, cx));
    });
    cx.run_until_parked();
}

/// Tab to the key file field, below the password on the unlock screen.
fn focus_keyfile(cx: &mut TestAppContext, harness: &Harness) {
    in_window(cx, harness, |shell, window, cx| {
        shell
            .keyfile_input
            .update(cx, |input, cx| input.focus(window, cx));
    });
    cx.run_until_parked();
}

fn activate(cx: &mut TestAppContext, harness: &Harness) {
    AnyWindowHandle::from(harness.window)
        .update(cx, |_, window, _| window.activate_window())
        .unwrap();
    cx.run_until_parked();
}

fn deactivate(cx: &mut TestAppContext, harness: &Harness) {
    VisualTestContext::from_window(harness.window.into(), cx).deactivate_window();
    cx.run_until_parked();
}

fn shell_holds(cx: &mut TestAppContext, harness: &Harness) -> bool {
    harness
        .shell
        .read_with(cx, |shell, _| shell.secure_input.is_some())
}

/// Notifications on the window. Their text cannot be read back from a
/// headless window, but their number can.
fn notices(cx: &mut TestAppContext, harness: &Harness) -> usize {
    AnyWindowHandle::from(harness.window)
        .update(cx, |_, window, cx| window.notifications(cx).len())
        .unwrap()
}

#[gpui::test]
fn password_focus_holds_and_blur_releases(cx: &mut TestAppContext) {
    let harness = unlock_screen(cx, FakeApi::new());
    assert!(!harness.fake.held(), "nothing has focus yet");

    focus_password(cx, &harness);
    assert!(
        harness.fake.held(),
        "focusing the master password field did not hold secure input"
    );
    assert!(shell_holds(cx, &harness));
    assert_eq!(harness.fake.api.enables(), 1);

    focus_keyfile(cx, &harness);
    assert!(
        !harness.fake.held(),
        "leaving the password field left secure input on"
    );
    assert!(!shell_holds(cx, &harness));
    assert_eq!(harness.fake.api.disables(), 1);

    focus_password(cx, &harness);
    assert!(harness.fake.held());
    assert_eq!(harness.fake.api.enables(), 2);
}

#[gpui::test]
fn window_deactivation_releases_and_activation_holds_again(cx: &mut TestAppContext) {
    let harness = unlock_screen(cx, FakeApi::new());
    focus_password(cx, &harness);
    assert!(harness.fake.held());

    // What is held at the moment the window reports the change, before the
    // next frame. GPUI reports the focus leaving the field as well, but only
    // when that frame is drawn.
    let seen = Rc::new(RefCell::new(Vec::new()));
    let _watch = {
        let seen = seen.clone();
        let fake = harness.fake;
        in_window(cx, &harness, move |_, window, cx| {
            cx.observe_window_activation(window, move |_, window, _| {
                seen.borrow_mut()
                    .push((window.is_window_active(), fake.held()));
            })
        })
    };

    deactivate(cx, &harness);
    assert_eq!(
        seen.borrow().last(),
        Some(&(false, false)),
        "secure input was still held when another app came to the front"
    );
    assert!(!harness.fake.held());
    assert!(!shell_holds(cx, &harness));
    assert_eq!(harness.fake.api.disables(), 1);

    activate(cx, &harness);
    assert_eq!(
        seen.borrow().last(),
        Some(&(true, true)),
        "the field still has focus, so coming back has to hold again"
    );
    assert!(harness.fake.held());
    assert!(shell_holds(cx, &harness));
    // One Enable for coming back, however many ways GPUI reports it.
    assert_eq!(harness.fake.api.enables(), 2);
    assert_eq!(harness.fake.api.disables(), 1);
}

#[gpui::test]
fn lock_releases_secure_input(cx: &mut TestAppContext) {
    let harness = unlock_screen(cx, FakeApi::new());
    focus_password(cx, &harness);
    // A second holder, as the menu bar panel will be: the lock has to
    // release every guard, not only the field's.
    let other = cx.update(|cx| secure_input::hold(cx)).expect("hold");
    assert_eq!(harness.fake.api.enables(), 1);

    // Checked inside the same update, while the field still has focus and
    // before any frame could report it leaving.
    let held_after_lock = in_window(cx, &harness, |shell, window, cx| {
        shell.lock_vault(window, cx);
        secure_input::held(cx)
    });
    assert!(!held_after_lock, "locking left secure input on");
    cx.run_until_parked();
    assert!(!shell_holds(cx, &harness));
    harness.state.read_with(cx, |state, _| {
        assert!(matches!(state.vault_status(), VaultStatus::Empty));
    });
    assert_eq!(harness.fake.api.disables(), 1);

    drop(other);
    assert_eq!(
        harness.fake.api.disables(),
        1,
        "a guard from before the lock disabled again"
    );
    assert!(!harness.fake.held());
}

#[gpui::test]
fn unlock_leaves_nothing_held(cx: &mut TestAppContext) {
    let harness = unlock_screen(cx, FakeApi::new());
    focus_password(cx, &harness);
    assert!(harness.fake.held());

    // How a successful unlock ends: the vault is open, and the unlock
    // screen goes away with the field that had focus.
    harness.state.update(cx, |state, cx| {
        state.open_for_test(
            PathBuf::from(VAULT),
            VaultDocument::new(
                ::keepass::db::Database::new(),
                VaultSnapshot::new(VaultGroup::default()),
                "pw".to_string(),
                None,
            ),
        );
        cx.notify();
    });
    cx.run_until_parked();

    harness.state.read_with(cx, |state, _| {
        assert!(matches!(state.vault_status(), VaultStatus::Open { .. }));
    });
    assert!(!harness.fake.held(), "the open vault left secure input on");
    assert!(!shell_holds(cx, &harness));
    assert_eq!(harness.fake.api.enables(), 1);
    assert_eq!(harness.fake.api.disables(), 1);
}

#[gpui::test]
fn failed_hold_is_reported_not_swallowed(cx: &mut TestAppContext) {
    let harness = unlock_screen(cx, FakeApi::refusing(HoldError::Os(OsStatus(-50))));
    let before = notices(cx, &harness);

    focus_password(cx, &harness);
    assert!(!shell_holds(cx, &harness));
    assert!(!harness.fake.held());
    assert_eq!(
        notices(cx, &harness),
        before + 1,
        "a refused hold has to be shown to the user"
    );
    assert!(
        harness
            .fake
            .notes()
            .iter()
            .any(|line| line.contains("OSStatus -50")),
        "the refusal was not logged: {:?}",
        harness.fake.notes()
    );
}

#[gpui::test]
fn unsupported_hold_is_logged_once_and_never_shown(cx: &mut TestAppContext) {
    let harness = unlock_screen(cx, FakeApi::refusing(HoldError::Unsupported));
    // What `app::run` does at launch.
    cx.update(secure_input::install_hooks);
    assert_eq!(
        harness.fake.notes(),
        ["secure event input is not available on this platform"]
    );
    let before = notices(cx, &harness);

    focus_password(cx, &harness);
    focus_keyfile(cx, &harness);
    focus_password(cx, &harness);
    deactivate(cx, &harness);
    activate(cx, &harness);

    assert!(!shell_holds(cx, &harness));
    assert!(!harness.fake.held());
    assert_eq!(
        notices(cx, &harness),
        before,
        "there is nothing to tell the user at every focus"
    );
    assert_eq!(
        harness.fake.notes().len(),
        1,
        "logged more than the line at launch: {:?}",
        harness.fake.notes()
    );
}

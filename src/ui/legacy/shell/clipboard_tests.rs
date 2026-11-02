//! The main window copies through the app-wide clipboard service, and every
//! way of locking ends with that copy cleared.
use super::*;
use crate::app::VaultStatus;
use crate::app::services::clipboard::ClipboardService;
use crate::clipboard::{AccessBehavior, fake::FakePasteboard};
use crate::domain::{VaultGroup, VaultSnapshot};
use crate::keepass::VaultDocument;
use gpui::{TestAppContext, WindowHandle};
use gpui_component::Root;

fn open_document() -> VaultDocument {
    VaultDocument::new(
        ::keepass::db::Database::new(),
        VaultSnapshot::new(VaultGroup::default()),
        "pw".to_string(),
        None,
    )
}

fn copy_password(
    cx: &mut TestAppContext,
    window: WindowHandle<Root>,
    shell: &Entity<AppShell>,
    password: &str,
) {
    AnyWindowHandle::from(window)
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| {
                shell.copy_with_auto_clear(
                    Zeroizing::new(password.to_string()),
                    "Password",
                    window,
                    cx,
                );
            });
        })
        .unwrap();
    cx.run_until_parked();
}

#[gpui::test]
fn lock_clears_clipboard_ownership(cx: &mut TestAppContext) {
    let pasteboard = FakePasteboard::new(AccessBehavior::Default);
    let service = cx.update(|cx| {
        gpui_component::init(cx);
        crate::app::actions::init(cx);
        ClipboardService::install_for_test(cx, Box::new(pasteboard.clone()))
    });
    let path = PathBuf::from("/tmp/clipboard-lock.kdbx");
    let state = cx.new(|_| {
        let mut state = AppState::default();
        state.open_for_test(path.clone(), open_document());
        state
    });
    let mut shell = None;
    let window = cx.add_window(|window, cx| {
        let view = cx.new(|cx| AppShell::new(state.clone(), window, cx));
        shell = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    let shell = shell.expect("shell view");

    copy_password(cx, window, &shell, "hunter2");
    assert_eq!(pasteboard.ledger().text.as_deref(), Some("hunter2"));
    assert!(
        service.read_with(cx, |service, _| service.has_pending()),
        "the copy has to go through the clipboard service"
    );

    // Cmd+L and the Lock button.
    AnyWindowHandle::from(window)
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| shell.lock_vault(window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    state.read_with(cx, |state, _| {
        assert!(matches!(state.vault_status(), VaultStatus::Empty));
    });
    assert!(
        !service.read_with(cx, |service, _| service.has_pending()),
        "locking left the clipboard copy owned"
    );
    assert_eq!(
        pasteboard.ledger().text,
        None,
        "locking left the password on the clipboard"
    );

    // Closing the window, the idle lock and switching vaults lock through
    // `AppState` alone; the shell's state observer clears from there.
    state.update(cx, |state, cx| {
        state.open_for_test(path.clone(), open_document());
        cx.notify();
    });
    cx.run_until_parked();
    copy_password(cx, window, &shell, "hunter3");
    assert_eq!(pasteboard.ledger().text.as_deref(), Some("hunter3"));
    state.update(cx, |state, cx| {
        let _ = state.lock_vault(cx);
    });
    cx.run_until_parked();
    assert!(!service.read_with(cx, |service, _| service.has_pending()));
    assert_eq!(
        pasteboard.ledger().text,
        None,
        "a lock through the app state left the password on the clipboard"
    );
}

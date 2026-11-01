//! What an automatic lock does to an open entry editor, written down before
//! this file is split up. A refactoring has to keep this green without
//! editing it.
use super::*;
use crate::app::VaultStatus;
use crate::domain::{VaultGroup, VaultSnapshot};
use crate::keepass::VaultDocument;
use gpui::TestAppContext;
use gpui_component::Root;
use std::cell::RefCell;

thread_local! {
    static LAST_ERROR_NOTICE: RefCell<Option<SharedString>> = const { RefCell::new(None) };
}

/// Called by `notify_error` in test builds with the text it is about to
/// show. A notification's text cannot be read back from a headless
/// window, so the last one is kept here as well.
pub(super) fn record_error_notice(message: SharedString) -> SharedString {
    LAST_ERROR_NOTICE.with(|last| *last.borrow_mut() = Some(message.clone()));
    message
}

fn take_error_notice() -> Option<SharedString> {
    LAST_ERROR_NOTICE.with(|last| last.borrow_mut().take())
}

/// An unsaved draft cannot hold off the idle or screen lock, so the lock
/// throws it away. The user has to be told, not left with an empty editor.
#[gpui::test]
fn automatic_lock_discards_the_draft_and_says_so(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::app::actions::init(cx);
    });
    let state = cx.new(|_| {
        let mut state = AppState::default();
        state.open_for_test(
            PathBuf::from("/tmp/characterize-draft.kdbx"),
            VaultDocument::new(
                ::keepass::db::Database::new(),
                VaultSnapshot::new(VaultGroup::default()),
                "pw".to_string(),
                None,
            ),
        );
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
    take_error_notice();

    cx.dispatch_action(window.into(), NewEntry);
    cx.run_until_parked();
    cx.simulate_input(window.into(), "Bank");
    cx.run_until_parked();
    assert!(
        shell.read_with(cx, |shell, cx| shell.entry_form_is_dirty(cx)),
        "typing a title has to leave an unsaved draft"
    );

    // The way the idle timer does it: through the window, without holding
    // the root view, which the notification has to read.
    AnyWindowHandle::from(window)
        .update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| shell.lock_vault_automatically(window, cx));
        })
        .unwrap();
    cx.run_until_parked();

    shell.read_with(cx, |shell, cx| {
        assert!(!shell.entry_form_is_dirty(cx));
        assert_eq!(shell.new_entry_title_input.read(cx).value(), "");
    });
    state.read_with(cx, |state, _| {
        assert!(matches!(state.vault_status(), VaultStatus::Empty));
        assert!(!state.has_any_unlocked());
    });
    let notice = take_error_notice().expect("the user has to be told the draft is gone");
    assert!(
        notice.contains("unsaved changes were discarded"),
        "unexpected notice: {notice}"
    );
}

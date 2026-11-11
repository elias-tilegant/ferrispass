//! The main window copies through the app-wide clipboard service, and every
//! way of locking ends with that copy cleared. Copying a password or a
//! username also marks the entry as recently used; copying the URL does not.
use super::*;
use crate::app::VaultStatus;
use crate::app::services::clipboard::ClipboardService;
use crate::clipboard::{AccessBehavior, fake::FakePasteboard};
use crate::domain::{VaultGroup, VaultSnapshot};
use crate::keepass::VaultDocument;
use crate::keepass::repository::snapshot_from_database;
use ::keepass::db::fields;
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

const USERNAME: &str = "alice";
const PASSWORD: &str = "correct horse";
const URL: &str = "https://mail.example";

/// A vault with one entry that has a username, a password and a URL.
fn document_with_a_login() -> VaultDocument {
    let mut database = ::keepass::db::Database::new();
    {
        let mut root = database.root_mut();
        let mut entry = root.add_entry();
        entry.set_unprotected(fields::TITLE, "Webmail");
        entry.set_unprotected(fields::USERNAME, USERNAME);
        entry.set_protected(fields::PASSWORD, PASSWORD);
        entry.set_unprotected(fields::URL, URL);
    }
    let snapshot = snapshot_from_database(&database);
    VaultDocument::new(database, snapshot, "pw".to_string(), None)
}

struct OpenLogin {
    pasteboard: FakePasteboard,
    state: Entity<AppState>,
    window: WindowHandle<Root>,
    shell: Entity<AppShell>,
    entry_id: String,
}

/// The main window with that vault open and its one entry picked in the
/// list, the way a click on the row picks it.
fn open_login(cx: &mut TestAppContext) -> OpenLogin {
    let pasteboard = FakePasteboard::new(AccessBehavior::Default);
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::app::actions::init(cx);
        ClipboardService::install_for_test(cx, Box::new(pasteboard.clone()));
    });
    let state = cx.new(|_| {
        let mut state = AppState::default();
        state.open_for_test(
            PathBuf::from("/tmp/clipboard-recently-used.kdbx"),
            document_with_a_login(),
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

    let entry_id = state.update(cx, |state, cx| {
        let VaultStatus::Open { document, .. } = state.vault_status() else {
            panic!("the vault has to be open");
        };
        let snapshot = document.snapshot();
        let root = snapshot.root.id.clone();
        let entry = snapshot.live_entries()[0].id.clone();
        state.select_group(root, cx);
        state.select_entry(entry.clone(), cx);
        entry
    });
    cx.run_until_parked();
    assert!(
        recently_used(cx, &state).is_empty(),
        "nothing is used before the first copy"
    );

    OpenLogin {
        pasteboard,
        state,
        window,
        shell,
        entry_id,
    }
}

/// What the Copy Username, Copy Password and Copy URL commands call.
fn copy_value(cx: &mut TestAppContext, login: &OpenLogin, kind: CopyValueKind) {
    AnyWindowHandle::from(login.window)
        .update(cx, |_, window, cx| {
            login.shell.update(cx, |shell, cx| {
                shell.copy_selected_value(kind, window, cx);
            });
        })
        .unwrap();
    cx.run_until_parked();
}

/// The entries the "Recently used" list is built from.
fn recently_used(cx: &mut TestAppContext, state: &Entity<AppState>) -> Vec<String> {
    state.read_with(cx, |state, _| {
        let VaultStatus::Open { last_used, .. } = state.vault_status() else {
            panic!("the vault has to stay open");
        };
        last_used.keys().cloned().collect()
    })
}

#[gpui::test]
fn copying_the_password_marks_the_entry_as_recently_used(cx: &mut TestAppContext) {
    let login = open_login(cx);

    copy_value(cx, &login, CopyValueKind::Password);

    assert_eq!(login.pasteboard.ledger().text.as_deref(), Some(PASSWORD));
    assert_eq!(
        recently_used(cx, &login.state),
        vec![login.entry_id.clone()]
    );
}

#[gpui::test]
fn copying_the_username_marks_the_entry_as_recently_used(cx: &mut TestAppContext) {
    let login = open_login(cx);

    copy_value(cx, &login, CopyValueKind::Username);

    assert_eq!(login.pasteboard.ledger().text.as_deref(), Some(USERNAME));
    assert_eq!(
        recently_used(cx, &login.state),
        vec![login.entry_id.clone()]
    );
}

/// Copying a link to pass it on is not signing in with the entry.
#[gpui::test]
fn copying_the_url_does_not_mark_the_entry(cx: &mut TestAppContext) {
    let login = open_login(cx);

    copy_value(cx, &login, CopyValueKind::Url);

    assert_eq!(
        login.pasteboard.ledger().text.as_deref(),
        Some(URL),
        "the URL has to be copied for this to show anything"
    );
    assert_eq!(recently_used(cx, &login.state), Vec::<String>::new());
}

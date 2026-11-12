use gpui::{AnyWindowHandle, App, KeyBinding, Menu, MenuItem, SharedString, Window, actions};
use gpui_component::WindowExt as _;
use std::sync::atomic::{AtomicBool, Ordering};

pub const APP_CONTEXT: &str = "FerrisPass";

/// The key context of the quick access panel's root.
pub const PANEL_CONTEXT: &str = "Panel";

/// The panel's search field. The GPUI Kit input binds ↑, ↓ and Cmd+C in its
/// own context and does not pass them on, so the panel binds them one level
/// further in; a binding on the panel's root never sees them while the field
/// has focus, and the field always has it. Cmd+W goes here too, see
/// `bind_panel_keys`.
const PANEL_FIELD_CONTEXT: &str = "Panel > Input";

actions!(
    ferrispass,
    [
        OpenVault,
        OpenVaultSwitcher,
        OpenAddVault,
        AddSharePointVault,
        SubmitPassword,
        /// Triggered by the "Unlock with Touch ID" button on the Unlock
        /// screen. Runs the OS biometric prompt off-thread and feeds the
        /// retrieved password directly into the existing open flow.
        SubmitBiometricUnlock,
        /// Toggle bound to the "Enable Touch ID for this vault"
        /// checkbox on the Unlock screen. Pure state flip; the actual
        /// enrolment happens after the password unlock succeeds.
        ToggleBiometricEnrollment,
        /// Drop the Touch ID enrolment for the currently-pending vault
        /// (both the keychain item and the registry entry). Wired
        /// from the Unlock screen when the OS reports the ACL was
        /// invalidated, and from the Settings page in a future phase.
        ForgetBiometric,
        CancelUnlock,
        LockVault,
        /// Minimize the window. Bound to the macOS-standard Cmd+M, which the
        /// app did not answer at all before.
        MinimizeWindow,
        /// Bring the window back after Cmd+W closed it. Also what the Dock
        /// icon does.
        ShowWindow,
        FocusSearch,
        CopyUsername,
        CopyUrl,
        CopyPassword,
        OpenConnect,
        /// Re-authenticate an *existing* synced vault whose refresh token
        /// expired. Unlike `OpenConnect` (which runs the full provider →
        /// device-code → file-picker flow and writes a brand-new local
        /// copy), this reuses the active vault's on-disk `SyncConfig` and
        /// only swaps in a fresh access/refresh token - no new file, no
        /// duplicate binding. Dispatched by the "Reconnect" button on the
        /// Sync settings card and the sidebar's expired-sign-in affordance.
        OpenReconnect,
        OpenSettings,
        OpenSyncSettings,
        InstallUpdate,
        RestartToUpdate,
        OpenWhatsNew,
        OpenAbout,
        SyncNow,
        DownloadFavicons,
        NewEntry,
        CreateVault,
        ToggleTheme,
        CloseWindow,
        Quit,
        SaveVault,
        EditEntry,
        DeleteEntry,
        /// Auto-type credentials into the foreground window. Dispatched
        /// by the global hotkey listener when the user presses the
        /// configured combo from any app. The handler matches the
        /// foreground window to a vault entry by URL hostname and
        /// types `{USERNAME}{TAB}{PASSWORD}{ENTER}` (or the user's
        /// custom sequence).
        PerformAutoType,
        /// Auto-type credentials for the *currently-selected* entry,
        /// after a short countdown that lets the user switch to the
        /// target window. Bound to ⌘⇧T inside FerrisPass - distinct
        /// from `PerformAutoType` (which runs from a global hotkey
        /// and infers the entry from the foreground).
        PerformAutoTypeForSelected,
        /// Open the quick access panel, from the Window menu. The global
        /// shortcut opens it without an action (`services::hotkeys`).
        OpenQuickAccess,
        /// Up in the quick access panel's list. Stops at the first row.
        PanelMoveUp,
        /// Down in the quick access panel's list. Stops at the last row.
        PanelMoveDown,
        /// Return in the quick access panel: the main action of the
        /// selected entry, unless it waits for a production system to be
        /// picked.
        PanelPrimary,
        /// Copy the password of the entry selected in the panel.
        PanelCopyPassword,
        /// Copy the user name of the entry selected in the panel.
        PanelCopyUsername,
        /// Copy the one-time code of the entry selected in the panel.
        PanelCopyCode,
        /// Close the panel and show the entry selected in it in the main
        /// window.
        PanelShowInApp,
    ]
);

/// Close the quick access panel. Cmd+W closes it at once; Escape first
/// clears a search that holds text and closes on the next press.
#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = ferrispass, no_json)]
pub struct ClosePanel {
    pub clear_search_first: bool,
}

/// Select the row with this number, 1 to 9, in the quick access panel.
#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = ferrispass, no_json)]
pub struct PanelSelect {
    pub number: u8,
}

/// The main window, for the app-level handlers that act on a window.
///
/// They act on this one and never on whichever window is key: with the quick
/// access panel in front, that is the panel, and Cmd+Q from there must not
/// store the panel's size as the window's, a notice must not land in the
/// panel, and Cmd+M must not minimize it. Set when the main window opens.
pub(crate) struct MainWindow(pub(crate) AnyWindowHandle);

impl gpui::Global for MainWindow {}

/// The main window, if it was opened.
pub(crate) fn main_window(cx: &App) -> Option<AnyWindowHandle> {
    cx.try_global::<MainWindow>().map(|main| main.0)
}

/// Shows `message` in `window`, the main window when one is in hand, or else
/// in the main window. Without a main window the message is dropped.
fn notify_main_window(window: Option<&mut Window>, message: impl Into<SharedString>, cx: &mut App) {
    let message = message.into();
    if let Some(window) = window {
        window.push_notification(message, cx);
    } else if let Some(main) = main_window(cx) {
        let _ = main.update(cx, |_root, window, cx| {
            window.push_notification(message, cx);
        });
    }
}

/// Open the "New group" modal targeting the database root. Dispatched by
/// the `+` button next to the "Groups" section heading.
#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = ferrispass, no_json)]
pub struct NewGroup;

/// Open the "New subgroup" modal targeting a specific parent group.
/// Dispatched from the right-click context menu on a group row.
#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = ferrispass, no_json)]
pub struct NewSubgroup {
    pub parent_group_id: String,
}

/// Open the "Rename group" modal for a specific group. Suffixed with `Op`
/// to keep the action name out of the way of `Overlay::RenameGroup`.
#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = ferrispass, no_json)]
pub struct RenameGroupOp {
    pub group_id: String,
}

/// Soft-delete a group: move the subtree to the Recycle Bin. Dispatched
/// from the group row context menu.
#[derive(Clone, PartialEq, Default, Debug, gpui::Action)]
#[action(namespace = ferrispass, no_json)]
pub struct DeleteGroup {
    pub group_id: String,
}

const SAVE_IN_PROGRESS_MESSAGE: &str =
    "FerrisPass is still saving vault changes. Wait for the save to finish.";

/// Set by the first refused quit against a stalled save. A save that has
/// settled into `Failed` never completes on its own, so vetoing forever left
/// the user with no way to quit short of killing the process. The second
/// attempt goes through and loses those changes, which is the user's call to
/// make once they have been told.
static QUIT_ARMED_OVER_STALLED_SAVE: AtomicBool = AtomicBool::new(false);

fn block_lifecycle_action_while_saving(window: Option<&mut Window>, cx: &mut App) -> bool {
    if !super::state::has_unpersisted_vault_saves() {
        QUIT_ARMED_OVER_STALLED_SAVE.store(false, Ordering::Release);
        return false;
    }

    let message = match super::state::stalled_save_vault() {
        Some(vault) => {
            if QUIT_ARMED_OVER_STALLED_SAVE.swap(true, Ordering::AcqRel) {
                // Told once, asked again: let them out.
                QUIT_ARMED_OVER_STALLED_SAVE.store(false, Ordering::Release);
                return false;
            }
            format!(
                "Saving \"{vault}\" failed and is not retrying. Try Save again, or repeat this \
                 to quit and lose those changes."
            )
        }
        None => SAVE_IN_PROGRESS_MESSAGE.to_string(),
    };

    notify_main_window(window, message, cx);
    true
}

/// The save veto only covers changes that already reached the vault. A
/// half-written entry lives in the editor's inputs, and locking or hiding the
/// window clears it, so quitting and closing have to ask the editor too.
///
/// Returns `true` when the caller must stand down. The first attempt arms the
/// discard banner in the editor and refuses; repeating the gesture goes
/// through, the same two-step Escape has always used.
fn block_lifecycle_action_over_dirty_editor(window: Option<&mut Window>, cx: &mut App) -> bool {
    if super::with_shared_shell(cx, |shell, cx| shell.entry_draft_blocks_close(cx)) != Some(true) {
        return false;
    }
    let message = "This entry has unsaved changes. Save it, or repeat this to discard it.";
    notify_main_window(window, message, cx);
    true
}

fn request_quit(cx: &mut App) {
    if block_lifecycle_action_while_saving(None, cx) {
        return;
    }
    if block_lifecycle_action_over_dirty_editor(None, cx) {
        return;
    }
    quit(cx);
}

/// Quit once nothing holds it back.
///
/// Wipe cleartext launch payloads and our own clipboard copy before the
/// platform starts tearing down windows. GPUI schedules the macOS terminate
/// call asynchronously; the clipboard's quit hook runs at its end and is the
/// second chance, not the only one.
pub(super) fn quit(cx: &mut App) {
    crate::launch::sweeper::purge_all();
    super::services::clipboard::clear_owned(cx);
    // Secure event input as well; its quit hook is the second chance too.
    crate::platform::secure_input::release_all(cx);
    cx.quit();
}

/// Restart into an installed update once nothing holds it back. GPUI quits
/// this process before the new one starts, so the cleanup is the same as
/// [`quit`]'s.
pub(super) fn restart(cx: &mut App) {
    crate::launch::sweeper::purge_all();
    super::services::clipboard::clear_owned(cx);
    crate::platform::secure_input::release_all(cx);
    cx.restart();
}

/// Store where the user left the window, so the next launch opens there
/// instead of centring a fixed default on whatever display is primary.
fn remember_window_geometry(window: &Window) {
    #[cfg(test)]
    stored_geometry::note(window.window_handle());
    let bounds = window.bounds();
    super::settings::persist_window_bounds(super::settings::WindowBoundsSetting {
        x: f32::from(bounds.origin.x),
        y: f32::from(bounds.origin.y),
        width: f32::from(bounds.size.width),
        height: f32::from(bounds.size.height),
    });
}

/// Closing the window locks the vault and leaves the app running, the way
/// every other Mac password manager behaves: the Dock icon stays and clicking
/// it brings the window back at the unlock screen. Quitting is Cmd+Q.
///
/// The window is hidden, not destroyed. The auto-sync timer lives on the
/// window's view; tearing it down would silently switch off a feature the
/// user configured. The global shortcuts run at app level
/// (`services::hotkeys`), so they keep working behind the hidden window.
///
/// Native window-close callbacks run while that window is already on GPUI's
/// update stack, so notify it directly instead of resolving `active_window`.
pub(crate) fn request_window_close(window: &mut Window, cx: &mut App) -> bool {
    if block_lifecycle_action_while_saving(Some(window), cx) {
        return false;
    }
    if block_lifecycle_action_over_dirty_editor(Some(window), cx) {
        return false;
    }
    remember_window_geometry(window);
    // Locking is the point: a window that is not on screen must not leave a
    // decrypted vault and a live clipboard behind it.
    lock_all_vaults(cx);
    crate::launch::sweeper::purge_all();
    cx.hide();
    // Always veto the platform close. The window stays alive behind the hide.
    false
}

/// Lock every unlocked vault before the window goes away.
fn lock_all_vaults(cx: &mut App) {
    super::with_shared_state(cx, |state, cx| {
        let _ = state.lock_vault(cx);
    });
}

/// The windows whose geometry was stored, on this thread, for the tests:
/// storing it writes the settings file, which tests running side by side
/// share.
#[cfg(test)]
pub(crate) mod stored_geometry {
    use std::cell::RefCell;

    use gpui::AnyWindowHandle;

    thread_local! {
        static STORED: RefCell<Vec<AnyWindowHandle>> = const { RefCell::new(Vec::new()) };
    }

    pub(super) fn note(window: AnyWindowHandle) {
        STORED.with(|stored| stored.borrow_mut().push(window));
    }

    /// The windows noted since the last call, oldest first.
    pub(crate) fn take() -> Vec<AnyWindowHandle> {
        STORED.with(|stored| std::mem::take(&mut *stored.borrow_mut()))
    }
}

/// Cmd+Q and the Quit menu item: remember where the main window is, then
/// quit unless a save or an unsaved entry holds it back. The geometry is the
/// main window's, whichever window is in front.
pub(crate) fn quit_from_anywhere(cx: &mut App) {
    if let Some(main) = main_window(cx) {
        let _ = main.update(cx, |_root, window, _cx| remember_window_geometry(window));
    }
    request_quit(cx);
}

/// The Close Window menu item and Cmd+W outside the panel: lock and hide,
/// for the main window and only while it is the key window. With the quick
/// access panel in front, Cmd+W is the panel's (`ClosePanel`), and this does
/// nothing.
pub(crate) fn close_main_window(cx: &mut App) {
    if let Some(main) = main_window(cx) {
        let _ = main.update(cx, |_root, window, cx| {
            if window.is_window_active() {
                request_window_close(window, cx);
            }
        });
    }
}

/// Cmd+M: minimize the main window while it is the key window. With the quick
/// access panel in front it does nothing; the panel is not minimized either.
pub(crate) fn minimize_main_window(cx: &mut App) {
    if let Some(main) = main_window(cx) {
        let _ = main.update(cx, |_root, window, _cx| {
            if window.is_window_active() {
                window.minimize_window();
            }
        });
    }
}

pub fn init(cx: &mut App) {
    // App-global Quit handler. Wired here (not on AppShell) so the action fires
    // independently of whatever view currently holds focus.
    cx.on_action(|_: &Quit, cx: &mut App| quit_from_anywhere(cx));
    cx.on_action(|_: &CloseWindow, cx: &mut App| close_main_window(cx));
    cx.on_action(|_: &MinimizeWindow, cx: &mut App| minimize_main_window(cx));

    cx.on_action(|_: &ShowWindow, cx: &mut App| {
        super::show_main_window(cx);
    });

    cx.on_action(|_: &OpenQuickAccess, cx: &mut App| {
        crate::ui::windows::panel::open(cx);
    });

    cx.on_action(|_: &RestartToUpdate, cx: &mut App| {
        if block_lifecycle_action_while_saving(None, cx) {
            return;
        }
        if block_lifecycle_action_over_dirty_editor(None, cx) {
            return;
        }
        restart(cx);
    });

    cx.bind_keys([
        // ⌘O opens the vault switcher (recents + filter + Browse…). The
        // raw file-dialog action `OpenVault` is still wired so the
        // switcher's "Browse other vault…" row, the Welcome screen, and
        // the Unlock screen's fallback can dispatch it directly.
        KeyBinding::new("cmd-o", OpenVaultSwitcher, Some(APP_CONTEXT)),
        KeyBinding::new("enter", SubmitPassword, Some(APP_CONTEXT)),
        KeyBinding::new("escape", CancelUnlock, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-l", LockVault, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-f", FocusSearch, Some(APP_CONTEXT)),
        // The standard copy gesture copies the password of the currently
        // selected vault entry. Text inputs keep their own, more-specific
        // native copy handling when they have focus. `cmd-shift-p` used to be
        // a second binding for the same action, which left the menu free to
        // display either one.
        KeyBinding::new("cmd-c", CopyPassword, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-shift-u", CopyUsername, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-shift-l", CopyUrl, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-,", OpenSettings, Some(APP_CONTEXT)),
        // Power-shortcut: jump straight to the Sync tab. Same overlay,
        // pre-selected tab.
        KeyBinding::new("cmd-shift-,", OpenSyncSettings, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-n", NewEntry, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-shift-d", ToggleTheme, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-s", SaveVault, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-e", EditEntry, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-backspace", DeleteEntry, Some(APP_CONTEXT)),
        // ⌘⇧T triggers an in-app auto-type for the currently-selected
        // entry with a 3-second countdown so the user has time to
        // switch to the target window. The global hotkey (configured
        // in Settings, default ⌃⌥⌘V) is the more common entry point
        // and works from any app - `PerformAutoType` is dispatched by
        // `AutoTypeService` directly, no KeyBinding here.
        KeyBinding::new("cmd-shift-t", PerformAutoTypeForSelected, Some(APP_CONTEXT)),
        // No context filter - cmd-q should always quit, even if focus is in
        // some weird state (e.g. inside a modal or before the shell is wired).
        KeyBinding::new("cmd-m", MinimizeWindow, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
    ]);
    bind_panel_keys(cx);

    install_app_menus(cx);
}

/// The quick access panel's keys. They come after the GPUI Kit's, which
/// `gpui_component::init` registers first, and after the app-wide ones, so
/// that where two bindings rank the same the panel's wins.
///
/// GPUI ranks a binding without a context as deep as the focused element,
/// above any binding on an element around it. The app-wide `cmd-w` would
/// therefore beat a binding on the panel's root, so the panel binds Cmd+W
/// on the search field, which always has the focus: there the two rank the
/// same and the later one, the panel's, wins. Cmd+W and Escape close the
/// panel and nothing else. Return and Escape go to the field first, which
/// passes them on to the panel's root.
fn bind_panel_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(
            "escape",
            ClosePanel {
                clear_search_first: true,
            },
            Some(PANEL_CONTEXT),
        ),
        KeyBinding::new(
            "cmd-w",
            ClosePanel {
                clear_search_first: false,
            },
            Some(PANEL_FIELD_CONTEXT),
        ),
        KeyBinding::new("enter", PanelPrimary, Some(PANEL_CONTEXT)),
        KeyBinding::new("cmd-shift-c", PanelCopyUsername, Some(PANEL_CONTEXT)),
        KeyBinding::new("alt-cmd-c", PanelCopyCode, Some(PANEL_CONTEXT)),
        KeyBinding::new("cmd-o", PanelShowInApp, Some(PANEL_CONTEXT)),
        KeyBinding::new("up", PanelMoveUp, Some(PANEL_FIELD_CONTEXT)),
        KeyBinding::new("down", PanelMoveDown, Some(PANEL_FIELD_CONTEXT)),
        // Cmd+C copies the password even with text marked in the field:
        // nothing typed into the search is worth more than the password.
        KeyBinding::new("cmd-c", PanelCopyPassword, Some(PANEL_FIELD_CONTEXT)),
    ]);
    cx.bind_keys((1..=9u8).map(|number| {
        KeyBinding::new(
            &format!("cmd-{number}"),
            PanelSelect { number },
            Some(PANEL_FIELD_CONTEXT),
        )
    }));
}

/// Register the application-level menu bar. On macOS this populates the
/// standard `FerrisPass` menu shown next to the Apple logo (About,
/// Preferences, Quit, …). On Linux and Windows the GPUI platform layer
/// treats `set_menus` as a no-op, so calling it unconditionally is safe
/// and the same action dispatches still work - the items are simply not
/// rendered in a system menu bar.
fn install_app_menus(cx: &mut App) {
    cx.set_menus([
        Menu::new("FerrisPass").items([
            MenuItem::action("About FerrisPass", OpenAbout),
            MenuItem::separator(),
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::action("Lock Vault", LockVault),
            MenuItem::separator(),
            MenuItem::action("Quit FerrisPass", Quit),
        ]),
        Menu::new("File").items([
            // No "New Vault…" here: the flow is not built, and a menu item
            // that answers with a toast is a menu that lies.
            MenuItem::action("Open Vault…", OpenVaultSwitcher),
            MenuItem::separator(),
            MenuItem::action("Save Vault", SaveVault),
            MenuItem::separator(),
            MenuItem::action("Close Window", CloseWindow),
        ]),
        // The standard editing commands work in every text field through
        // gpui-component's own bindings, but without menu items macOS shows
        // no shortcuts for them and VoiceOver cannot reach them at all.
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", gpui_component::input::Undo, gpui::OsAction::Undo),
            MenuItem::os_action("Redo", gpui_component::input::Redo, gpui::OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", gpui_component::input::Cut, gpui::OsAction::Cut),
            MenuItem::os_action("Copy", gpui_component::input::Copy, gpui::OsAction::Copy),
            MenuItem::os_action("Paste", gpui_component::input::Paste, gpui::OsAction::Paste),
            MenuItem::os_action(
                "Select All",
                gpui_component::input::SelectAll,
                gpui::OsAction::SelectAll,
            ),
            MenuItem::separator(),
            MenuItem::action("New Entry", NewEntry),
            MenuItem::action("Edit Entry", EditEntry),
            MenuItem::action("Delete Entry", DeleteEntry),
            MenuItem::separator(),
            MenuItem::action("Copy Username", CopyUsername),
            MenuItem::action("Copy Password", CopyPassword),
            MenuItem::action("Copy URL", CopyUrl),
            MenuItem::separator(),
            MenuItem::action("Auto-Type Selected Entry", PerformAutoTypeForSelected),
        ]),
        Menu::new("View").items([
            MenuItem::action("Find in Vault", FocusSearch),
            MenuItem::action("Toggle Theme", ToggleTheme),
            MenuItem::separator(),
            MenuItem::action("Sync Settings…", OpenSyncSettings),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", MinimizeWindow),
            MenuItem::separator(),
            MenuItem::action("Show FerrisPass", ShowWindow),
            MenuItem::action("Quick Access", OpenQuickAccess),
        ]),
        Menu::new("Help").items([MenuItem::action("What's New", OpenWhatsNew)]),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::services::clipboard::ClipboardService;
    use crate::clipboard::{AccessBehavior, fake::FakePasteboard};
    use crate::platform::secure_input::{self, fake::FakeApi};
    use gpui::TestAppContext;

    /// Cmd+Q, the Quit menu item and "Restart to update" let go of secure
    /// event input before macOS is asked to quit, the way they clear the
    /// clipboard. The test platform's quit does nothing, so only that direct
    /// release can have run here.
    #[gpui::test]
    fn quitting_releases_secure_input(cx: &mut TestAppContext) {
        let fake = cx.update(|cx| {
            ClipboardService::install_for_test(
                cx,
                Box::new(FakePasteboard::new(AccessBehavior::Default)),
            );
            secure_input::fake::install(cx, FakeApi::new())
        });

        let guard = cx.update(|cx| secure_input::hold(cx)).expect("hold");
        cx.update(quit);
        assert!(!fake.held(), "quitting left secure event input on");
        drop(guard);

        let guard = cx.update(|cx| secure_input::hold(cx)).expect("hold");
        cx.update(restart);
        assert!(
            !fake.held(),
            "restarting to update left secure event input on"
        );
        drop(guard);

        assert_eq!(fake.api.enables(), 2);
        assert_eq!(fake.api.disables(), 2);
    }
}

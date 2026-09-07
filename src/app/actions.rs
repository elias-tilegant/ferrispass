use gpui::{App, KeyBinding, Menu, MenuItem, Window, actions};
use gpui_component::WindowExt as _;
use std::sync::atomic::{AtomicBool, Ordering};

pub const APP_CONTEXT: &str = "FerrisPass";

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
    ]
);

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

    if let Some(window) = window {
        window.push_notification(message, cx);
    } else if let Some(window) = cx.active_window() {
        let _ = window.update(cx, |_root, window, cx| {
            window.push_notification(message, cx);
        });
    }
    true
}

fn request_quit(cx: &mut App) {
    if block_lifecycle_action_while_saving(None, cx) {
        return;
    }

    // Wipe cleartext launch payloads before the platform starts tearing down
    // windows. GPUI schedules the macOS terminate call asynchronously.
    crate::launch::sweeper::purge_all();
    cx.quit();
}

/// Store where the user left the window, so the next launch opens there
/// instead of centring a fixed default on whatever display is primary.
fn remember_window_geometry(window: &Window) {
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
/// The window is hidden, not destroyed. The Auto-Type hotkey listener and the
/// auto-sync timer live on the window's view; tearing it down would silently
/// switch off a feature the user configured, and a global hotkey that stops
/// working after Cmd+W is worse than no Cmd+W behaviour at all.
///
/// Native window-close callbacks run while that window is already on GPUI's
/// update stack, so notify it directly instead of resolving `active_window`.
pub(crate) fn request_window_close(window: &mut Window, cx: &mut App) -> bool {
    if block_lifecycle_action_while_saving(Some(window), cx) {
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

pub fn init(cx: &mut App) {
    // App-global Quit handler. Wired here (not on AppShell) so the action fires
    // independently of whatever view currently holds focus.
    cx.on_action(|_: &Quit, cx: &mut App| {
        if let Some(handle) = cx.active_window() {
            let _ = handle.update(cx, |_root, window, _cx| remember_window_geometry(window));
        }
        request_quit(cx);
    });

    cx.on_action(|_: &CloseWindow, cx: &mut App| {
        if let Some(handle) = cx.active_window() {
            let _ = handle.update(cx, |_root, window, cx| {
                request_window_close(window, cx);
            });
        }
    });

    cx.on_action(|_: &MinimizeWindow, cx: &mut App| {
        if let Some(handle) = cx.active_window() {
            let _ = handle.update(cx, |_root, window, _cx| window.minimize_window());
        }
    });

    cx.on_action(|_: &ShowWindow, cx: &mut App| {
        super::show_main_window(cx);
    });

    cx.on_action(|_: &RestartToUpdate, cx: &mut App| {
        if block_lifecycle_action_while_saving(None, cx) {
            return;
        }
        crate::launch::sweeper::purge_all();
        cx.restart();
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

    install_app_menus(cx);
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
        ]),
        Menu::new("Help").items([MenuItem::action("What's New", OpenWhatsNew)]),
    ]);
}

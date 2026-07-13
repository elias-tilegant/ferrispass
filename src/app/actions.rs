use gpui::{App, KeyBinding, Menu, MenuItem, actions};
use gpui_component::WindowExt as _;

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
        FocusSearch,
        CopyUsername,
        CopyUrl,
        CopyPassword,
        OpenConnect,
        /// Re-authenticate an *existing* synced vault whose refresh token
        /// expired. Unlike `OpenConnect` (which runs the full provider →
        /// device-code → file-picker flow and writes a brand-new local
        /// copy), this reuses the active vault's on-disk `SyncConfig` and
        /// only swaps in a fresh access/refresh token — no new file, no
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
        OpenConflictDemo,
        CreateVault,
        ToggleTheme,
        Quit,
        SaveVault,
        EditEntry,
        DeleteEntry,
        /// Open the currently-selected entry in its native external app
        /// (e.g. SAP GUI for entries with a `SAP_CONN` custom field).
        /// No default keybinding in v0.3 — the detail-panel button is
        /// the only entry point until we know which shortcut won't
        /// collide with future "Open in browser" / "Open in terminal"
        /// flavours.
        LaunchEntry,
        /// Auto-type credentials into the foreground window. Dispatched
        /// by the global hotkey listener when the user presses the
        /// configured combo from any app. The handler matches the
        /// foreground window to a vault entry by URL hostname and
        /// types `{USERNAME}{TAB}{PASSWORD}{ENTER}` (or the user's
        /// custom sequence).
        PerformAutoType,
        /// Auto-type credentials for the *currently-selected* entry,
        /// after a short countdown that lets the user switch to the
        /// target window. Bound to ⌘⇧T inside FerrisPass — distinct
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

fn block_lifecycle_action_while_saving(cx: &mut App) -> bool {
    if !super::state::has_unpersisted_vault_saves() {
        return false;
    }

    if let Some(window) = cx.active_window() {
        let _ = window.update(cx, |_root, window, cx| {
            window.push_notification(
                "FerrisPass is still saving vault changes. Wait for the save to finish, or retry Save if it failed.",
                cx,
            );
        });
    }
    true
}

pub fn init(cx: &mut App) {
    // App-global Quit handler. Wired here (not on AppShell) so the action fires
    // independently of whatever view currently holds focus.
    cx.on_action(|_: &Quit, cx: &mut App| {
        if block_lifecycle_action_while_saving(cx) {
            return;
        }
        // Wipe the launch tempdir before we hand control back to the
        // OS — a Quit-mid-launch otherwise leaves cleartext payload
        // files lying around. The startup sweep would catch them on
        // the next run, but "delete on the way out" closes the window
        // tighter and avoids the user wondering about stray files
        // between sessions.
        crate::launch::sweeper::purge_all();
        cx.quit();
    });

    cx.on_action(|_: &RestartToUpdate, cx: &mut App| {
        if block_lifecycle_action_while_saving(cx) {
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
        KeyBinding::new("cmd-shift-u", CopyUsername, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-shift-l", CopyUrl, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-shift-p", CopyPassword, Some(APP_CONTEXT)),
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
        // and works from any app — `PerformAutoType` is dispatched by
        // `AutoTypeService` directly, no KeyBinding here.
        KeyBinding::new("cmd-shift-t", PerformAutoTypeForSelected, Some(APP_CONTEXT)),
        // No context filter — cmd-q should always quit, even if focus is in
        // some weird state (e.g. inside a modal or before the shell is wired).
        KeyBinding::new("cmd-q", Quit, None),
    ]);

    install_app_menus(cx);
}

/// Register the application-level menu bar. On macOS this populates the
/// standard `FerrisPass` menu shown next to the Apple logo (About,
/// Preferences, Quit, …). On Linux and Windows the GPUI platform layer
/// treats `set_menus` as a no-op, so calling it unconditionally is safe
/// and the same action dispatches still work — the items are simply not
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
            MenuItem::action("New Vault…", CreateVault),
            MenuItem::action("Open Vault…", OpenVaultSwitcher),
            MenuItem::separator(),
            MenuItem::action("Save Vault", SaveVault),
        ]),
        Menu::new("Edit").items([
            MenuItem::action("New Entry", NewEntry),
            MenuItem::action("Edit Entry", EditEntry),
            MenuItem::action("Delete Entry", DeleteEntry),
            MenuItem::separator(),
            MenuItem::action("Copy Username", CopyUsername),
            MenuItem::action("Copy Password", CopyPassword),
            MenuItem::action("Copy URL", CopyUrl),
        ]),
        Menu::new("View").items([
            MenuItem::action("Find in Vault", FocusSearch),
            MenuItem::action("Toggle Theme", ToggleTheme),
        ]),
        Menu::new("Help").items([MenuItem::action("What's New", OpenWhatsNew)]),
    ]);
}

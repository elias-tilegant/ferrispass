use gpui::{App, KeyBinding, actions};

pub const APP_CONTEXT: &str = "FerrisPass";

actions!(
    ferrispass,
    [
        OpenVault,
        OpenVaultSwitcher,
        SubmitPassword,
        CancelUnlock,
        LockVault,
        FocusSearch,
        CopyUsername,
        CopyUrl,
        CopyPassword,
        OpenConnect,
        OpenSettings,
        OpenSyncSettings,
        InstallUpdate,
        RestartToUpdate,
        OpenWhatsNew,
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

pub fn init(cx: &mut App) {
    // App-global Quit handler. Wired here (not on AppShell) so the action fires
    // independently of whatever view currently holds focus.
    cx.on_action(|_: &Quit, cx: &mut App| {
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
}

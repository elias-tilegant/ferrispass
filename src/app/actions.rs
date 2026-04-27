use gpui::{App, KeyBinding, actions};

pub const APP_CONTEXT: &str = "StcKeepass";

actions!(
    stc_keepass,
    [
        OpenVault,
        SubmitPassword,
        CancelUnlock,
        LockVault,
        FocusSearch,
        CopyUsername,
        CopyUrl,
        CopyPassword,
        OpenConnect,
        OpenSyncSettings,
        NewEntry,
        OpenConflictDemo,
        CreateVault,
        ToggleTheme,
        Quit,
    ]
);

pub fn init(cx: &mut App) {
    // App-global Quit handler. Wired here (not on AppShell) so the action fires
    // independently of whatever view currently holds focus.
    cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

    cx.bind_keys([
        KeyBinding::new("cmd-o", OpenVault, Some(APP_CONTEXT)),
        KeyBinding::new("enter", SubmitPassword, Some(APP_CONTEXT)),
        KeyBinding::new("escape", CancelUnlock, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-l", LockVault, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-f", FocusSearch, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-shift-u", CopyUsername, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-shift-l", CopyUrl, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-shift-p", CopyPassword, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-,", OpenSyncSettings, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-n", NewEntry, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-shift-d", ToggleTheme, Some(APP_CONTEXT)),
        // No context filter — cmd-q should always quit, even if focus is in
        // some weird state (e.g. inside a modal or before the shell is wired).
        KeyBinding::new("cmd-q", Quit, None),
    ]);
}

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
    ]
);

pub fn init(cx: &mut App) {
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
    ]);
}

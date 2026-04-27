use gpui::{App, KeyBinding, actions};

pub const APP_CONTEXT: &str = "StcKeepass";

actions!(
    stc_keepass,
    [OpenVault, SubmitPassword, CancelUnlock, LockVault]
);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-o", OpenVault, Some(APP_CONTEXT)),
        KeyBinding::new("enter", SubmitPassword, Some(APP_CONTEXT)),
        KeyBinding::new("escape", CancelUnlock, Some(APP_CONTEXT)),
        KeyBinding::new("cmd-l", LockVault, Some(APP_CONTEXT)),
    ]);
}

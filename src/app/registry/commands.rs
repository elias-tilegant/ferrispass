//! The table. A new command gets a name below, a row in `COMMANDS` and a
//! label in `locales/action.yml`; the tests check that the three agree.

use super::{
    CommandCtx, CommandId, CommandSpec, Enablement, SelectedKind, Selection, Surface, TargetKind,
    UrlKind,
};
use crate::tr;

impl CommandId {
    pub const ENTRY_PRIMARY: CommandId = CommandId("entry.primary");
    pub const ENTRY_SAP_LOGIN: CommandId = CommandId("entry.sap_login");
    pub const ENTRY_OPEN_WEBSITE: CommandId = CommandId("entry.open_website");
    pub const ENTRY_COPY_PASSWORD: CommandId = CommandId("entry.copy_password");
    pub const ENTRY_COPY_USERNAME: CommandId = CommandId("entry.copy_username");
    pub const ENTRY_COPY_CODE: CommandId = CommandId("entry.copy_code");
    pub const ENTRY_COPY_URL: CommandId = CommandId("entry.copy_url");
    pub const ENTRY_OPEN_URL: CommandId = CommandId("entry.open_url");
    pub const ENTRY_SHOW_IN_APP: CommandId = CommandId("entry.show_in_app");
    pub const APP_QUICK_ACCESS: CommandId = CommandId("app.quick_access");
    pub const APP_OPEN: CommandId = CommandId("app.open");
    pub const APP_LOCK_ALL: CommandId = CommandId("app.lock_all");
    pub const APP_SETTINGS: CommandId = CommandId("app.settings");
    pub const APP_QUIT: CommandId = CommandId("app.quit");
    pub const PANEL_CLOSE: CommandId = CommandId("panel.close");
}

pub(super) static COMMANDS: &[CommandSpec] = &[
    // Return runs the main action of the selected entry, which depends on
    // its kind. Its own word is for lists of shortcuts; with an entry in
    // view, primary_label gives the main action's word instead.
    CommandSpec {
        id: CommandId::ENTRY_PRIMARY,
        label_key: "action.primary.label",
        label: || tr!("action.primary.label"),
        keys: &["enter"],
        surfaces: &[Surface::Panel],
        target: TargetKind::Entry,
        enabled: primary,
        touches_secret: true,
    },
    // The main actions of SAP systems and web logins. They have no keys and
    // no surface of their own: Return reaches them through entry.primary.
    // A remote access entry's main action is copying its password.
    CommandSpec {
        id: CommandId::ENTRY_SAP_LOGIN,
        label_key: "action.sap_login.label",
        label: || tr!("action.sap_login.label"),
        keys: &[],
        surfaces: &[],
        target: TargetKind::Entry,
        enabled: sap_system,
        touches_secret: true,
    },
    CommandSpec {
        id: CommandId::ENTRY_OPEN_WEBSITE,
        label_key: "action.open_website.label",
        label: || tr!("action.open_website.label"),
        keys: &[],
        surfaces: &[],
        target: TargetKind::Entry,
        enabled: web_login,
        touches_secret: false,
    },
    CommandSpec {
        id: CommandId::ENTRY_COPY_PASSWORD,
        label_key: "action.copy_password.label",
        label: || tr!("action.copy_password.label"),
        keys: &["cmd-c"],
        surfaces: &[Surface::Panel],
        target: TargetKind::Entry,
        enabled: any_entry,
        touches_secret: true,
    },
    CommandSpec {
        id: CommandId::ENTRY_COPY_USERNAME,
        label_key: "action.copy_username.label",
        label: || tr!("action.copy_username.label"),
        keys: &["cmd-shift-c"],
        surfaces: &[Surface::Panel],
        target: TargetKind::Entry,
        enabled: with_username,
        touches_secret: false,
    },
    CommandSpec {
        id: CommandId::ENTRY_COPY_CODE,
        label_key: "action.copy_code.label",
        label: || tr!("action.copy_code.label"),
        keys: &["alt-cmd-c"],
        surfaces: &[Surface::Panel],
        target: TargetKind::Entry,
        enabled: with_one_time_code,
        touches_secret: true,
    },
    CommandSpec {
        id: CommandId::ENTRY_COPY_URL,
        label_key: "action.copy_url.label",
        label: || tr!("action.copy_url.label"),
        keys: &["cmd-shift-l"],
        surfaces: &[Surface::Panel],
        target: TargetKind::Entry,
        enabled: with_address,
        touches_secret: false,
    },
    CommandSpec {
        id: CommandId::ENTRY_OPEN_URL,
        label_key: "action.open_url.label",
        label: || tr!("action.open_url.label"),
        keys: &["alt-enter"],
        surfaces: &[Surface::Panel],
        target: TargetKind::Entry,
        enabled: with_web_address,
        touches_secret: false,
    },
    CommandSpec {
        id: CommandId::ENTRY_SHOW_IN_APP,
        label_key: "action.show_in_app.label",
        label: || tr!("action.show_in_app.label"),
        keys: &["cmd-o"],
        surfaces: &[Surface::Panel],
        target: TargetKind::Entry,
        enabled: any_entry,
        touches_secret: false,
    },
    // The menu bar item's menu will list the commands it offers in this order.
    CommandSpec {
        id: CommandId::APP_QUICK_ACCESS,
        label_key: "action.quick_access.label",
        label: || tr!("action.quick_access.label"),
        keys: &[],
        surfaces: &[Surface::StatusMenu],
        target: TargetKind::None,
        enabled: always,
        touches_secret: false,
    },
    CommandSpec {
        id: CommandId::APP_OPEN,
        label_key: "action.open.label",
        label: || tr!("action.open.label"),
        keys: &[],
        surfaces: &[Surface::StatusMenu],
        target: TargetKind::None,
        enabled: always,
        touches_secret: false,
    },
    // Hidden rather than disabled while everything is locked: there is
    // nothing to lock, and nothing to explain.
    CommandSpec {
        id: CommandId::APP_LOCK_ALL,
        label_key: "action.lock_all.label",
        label: || tr!("action.lock_all.label"),
        keys: &["cmd-l"],
        surfaces: &[Surface::Panel, Surface::StatusMenu],
        target: TargetKind::None,
        enabled: unlocked,
        touches_secret: false,
    },
    CommandSpec {
        id: CommandId::APP_SETTINGS,
        label_key: "action.settings.label",
        label: || tr!("action.settings.label"),
        keys: &["cmd-,"],
        surfaces: &[Surface::Panel, Surface::StatusMenu],
        target: TargetKind::None,
        enabled: always,
        touches_secret: false,
    },
    CommandSpec {
        id: CommandId::APP_QUIT,
        label_key: "action.quit.label",
        label: || tr!("action.quit.label"),
        keys: &["cmd-q"],
        surfaces: &[Surface::StatusMenu],
        target: TargetKind::None,
        enabled: always,
        touches_secret: false,
    },
    CommandSpec {
        id: CommandId::PANEL_CLOSE,
        label_key: "action.close.label",
        label: || tr!("action.close.label"),
        keys: &["cmd-w", "escape"],
        surfaces: &[Surface::Panel],
        target: TargetKind::Window,
        enabled: always,
        touches_secret: false,
    },
];

/// The reason Return does nothing on a production system from an empty
/// search. Quick Access shows it in its footer.
const SELECT_PRODUCTION_FIRST: &str = "quick.footer.prd_select_first";

/// From an empty search, Return never acts on a production system the user
/// has not picked. Typing in the search lifts the rule, and so does picking
/// the row with the arrow keys or Cmd+1 to Cmd+9. A production system that
/// is selected because it came first, or because it is the only match,
/// stays behind it.
fn primary(ctx: &CommandCtx) -> Enablement {
    let Some(selection) = entry(ctx) else {
        return Enablement::Hidden;
    };
    if ctx.query_empty && selection.kind.is_production() && !selection.explicit {
        Enablement::Disabled(SELECT_PRODUCTION_FIRST)
    } else {
        Enablement::Enabled
    }
}

/// The selected entry, while the app is unlocked. Every command on an entry
/// starts here, so none of them is offered while the app is locked.
fn entry(ctx: &CommandCtx) -> Option<&Selection> {
    if ctx.locked {
        None
    } else {
        ctx.selection.as_ref()
    }
}

fn shown_if(applies: bool) -> Enablement {
    if applies {
        Enablement::Enabled
    } else {
        Enablement::Hidden
    }
}

fn sap_system(ctx: &CommandCtx) -> Enablement {
    shown_if(entry(ctx).is_some_and(|selection| matches!(selection.kind, SelectedKind::Sap { .. })))
}

fn web_login(ctx: &CommandCtx) -> Enablement {
    shown_if(entry(ctx).is_some_and(|selection| selection.kind == SelectedKind::Web))
}

fn any_entry(ctx: &CommandCtx) -> Enablement {
    shown_if(entry(ctx).is_some())
}

fn with_username(ctx: &CommandCtx) -> Enablement {
    shown_if(entry(ctx).is_some_and(|selection| selection.has_username))
}

fn with_one_time_code(ctx: &CommandCtx) -> Enablement {
    shown_if(entry(ctx).is_some_and(|selection| selection.has_otp))
}

fn with_address(ctx: &CommandCtx) -> Enablement {
    shown_if(entry(ctx).is_some_and(|selection| selection.url != UrlKind::Empty))
}

/// Only http and https addresses are opened.
fn with_web_address(ctx: &CommandCtx) -> Enablement {
    shown_if(entry(ctx).is_some_and(|selection| selection.url == UrlKind::Web))
}

fn unlocked(ctx: &CommandCtx) -> Enablement {
    shown_if(!ctx.locked)
}

fn always(_: &CommandCtx) -> Enablement {
    Enablement::Enabled
}

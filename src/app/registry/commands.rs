//! The table. A new command gets a name below, a row in `COMMANDS` and a
//! label in `locales/action.yml`; the tests check that the three agree.

use super::{CommandId, CommandSpec, Surface, TargetKind};
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
    // its kind. Its own word is for lists of shortcuts; a list of what the
    // selected entry offers names the main action itself.
    CommandSpec {
        id: CommandId::ENTRY_PRIMARY,
        label_key: "action.primary.label",
        label: || tr!("action.primary.label"),
        keys: &["enter"],
        surfaces: &[Surface::Panel],
        target: TargetKind::Entry,
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
        touches_secret: true,
    },
    CommandSpec {
        id: CommandId::ENTRY_OPEN_WEBSITE,
        label_key: "action.open_website.label",
        label: || tr!("action.open_website.label"),
        keys: &[],
        surfaces: &[],
        target: TargetKind::Entry,
        touches_secret: false,
    },
    CommandSpec {
        id: CommandId::ENTRY_COPY_PASSWORD,
        label_key: "action.copy_password.label",
        label: || tr!("action.copy_password.label"),
        keys: &["cmd-c"],
        surfaces: &[Surface::Panel],
        target: TargetKind::Entry,
        touches_secret: true,
    },
    CommandSpec {
        id: CommandId::ENTRY_COPY_USERNAME,
        label_key: "action.copy_username.label",
        label: || tr!("action.copy_username.label"),
        keys: &["cmd-shift-c"],
        surfaces: &[Surface::Panel],
        target: TargetKind::Entry,
        touches_secret: false,
    },
    CommandSpec {
        id: CommandId::ENTRY_COPY_CODE,
        label_key: "action.copy_code.label",
        label: || tr!("action.copy_code.label"),
        keys: &["alt-cmd-c"],
        surfaces: &[Surface::Panel],
        target: TargetKind::Entry,
        touches_secret: true,
    },
    CommandSpec {
        id: CommandId::ENTRY_COPY_URL,
        label_key: "action.copy_url.label",
        label: || tr!("action.copy_url.label"),
        keys: &["cmd-shift-l"],
        surfaces: &[Surface::Panel],
        target: TargetKind::Entry,
        touches_secret: false,
    },
    CommandSpec {
        id: CommandId::ENTRY_OPEN_URL,
        label_key: "action.open_url.label",
        label: || tr!("action.open_url.label"),
        keys: &["alt-enter"],
        surfaces: &[Surface::Panel],
        target: TargetKind::Entry,
        touches_secret: false,
    },
    CommandSpec {
        id: CommandId::ENTRY_SHOW_IN_APP,
        label_key: "action.show_in_app.label",
        label: || tr!("action.show_in_app.label"),
        keys: &["cmd-o"],
        surfaces: &[Surface::Panel],
        target: TargetKind::Entry,
        touches_secret: false,
    },
    // The menu bar item's menu lists the commands it offers in this order.
    CommandSpec {
        id: CommandId::APP_QUICK_ACCESS,
        label_key: "action.quick_access.label",
        label: || tr!("action.quick_access.label"),
        keys: &[],
        surfaces: &[Surface::StatusMenu],
        target: TargetKind::None,
        touches_secret: false,
    },
    CommandSpec {
        id: CommandId::APP_OPEN,
        label_key: "action.open.label",
        label: || tr!("action.open.label"),
        keys: &[],
        surfaces: &[Surface::StatusMenu],
        target: TargetKind::None,
        touches_secret: false,
    },
    CommandSpec {
        id: CommandId::APP_LOCK_ALL,
        label_key: "action.lock_all.label",
        label: || tr!("action.lock_all.label"),
        keys: &["cmd-l"],
        surfaces: &[Surface::Panel, Surface::StatusMenu],
        target: TargetKind::None,
        touches_secret: false,
    },
    CommandSpec {
        id: CommandId::APP_SETTINGS,
        label_key: "action.settings.label",
        label: || tr!("action.settings.label"),
        keys: &["cmd-,"],
        surfaces: &[Surface::Panel, Surface::StatusMenu],
        target: TargetKind::None,
        touches_secret: false,
    },
    CommandSpec {
        id: CommandId::APP_QUIT,
        label_key: "action.quit.label",
        label: || tr!("action.quit.label"),
        keys: &["cmd-q"],
        surfaces: &[Surface::StatusMenu],
        target: TargetKind::None,
        touches_secret: false,
    },
    CommandSpec {
        id: CommandId::PANEL_CLOSE,
        label_key: "action.close.label",
        label: || tr!("action.close.label"),
        keys: &["cmd-w", "escape"],
        surfaces: &[Surface::Panel],
        target: TargetKind::Window,
        touches_secret: false,
    },
];

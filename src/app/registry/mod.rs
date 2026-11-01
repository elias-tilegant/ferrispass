//! The commands the new surfaces offer, in one table.
//!
//! Quick Access and the menu bar item's menu take their commands from here,
//! and the main window's menus and the command palette will too: what a
//! command is called, its keys, where it appears, what it acts on and when
//! it is available. Every surface shows the same word for a command,
//! `action.<verb>.label` in `locales/action.yml`, so a command never goes by
//! two names.
//!
//! Return runs the main action of the selected entry, which depends on its
//! kind ([`primary_command`]), and takes that action's word
//! ([`primary_label`]). From an empty search it never acts on a
//! production system the user has not picked; that rule is the availability
//! of `entry.primary` and is written down nowhere else.
//!
//! Nothing here binds a key or builds a menu yet. The surfaces do that as
//! they are built; until then the old main window keeps its own bindings and
//! menus in `src/app/actions.rs`.

#![forbid(unsafe_code)]

mod commands;
#[cfg(test)]
mod tests;

use gpui::SharedString;
use url::{ParseError, Url};

use crate::tr;

/// A command's stable name, `<area>.<verb>`: `entry.copy_password`,
/// `app.quit`. Its label is `action.<verb>.label`. The names are constants
/// on this type, such as [`CommandId::ENTRY_COPY_PASSWORD`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CommandId(&'static str);

impl CommandId {
    pub fn as_str(self) -> &'static str {
        self.0
    }
}

/// Where a command can appear.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Surface {
    /// Quick Access, the panel that opens over any app, with the list of
    /// all commands for the selected entry and the `[...]` menu in its
    /// footer.
    Panel,
    /// The right-click menu of the menu bar item.
    StatusMenu,
}

/// What a command acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    /// The app as a whole.
    None,
    /// The selected entry.
    Entry,
    /// The window the command is given in.
    Window,
}

/// One command.
#[derive(Debug)]
pub struct CommandSpec {
    pub id: CommandId,
    /// The key `label` reads, kept as text so tests can check it.
    pub label_key: &'static str,
    /// The command's word in the UI language.
    pub label: fn() -> SharedString,
    /// The shortcuts, as GPUI key strings (`cmd-shift-c`), written the way
    /// `gpui::Keystroke` writes them back. How a key looks on a keycap is up
    /// to whoever draws it.
    pub keys: &'static [&'static str],
    /// Where the command appears, in no particular order. A command without
    /// a surface is reached through another one.
    pub surfaces: &'static [Surface],
    pub target: TargetKind,
    /// Whether the command can be used in this situation.
    pub enabled: fn(&CommandCtx) -> Enablement,
    /// Whether the command reads a password or a one-time code, or hands
    /// one on.
    pub touches_secret: bool,
}

/// Whether a command can be used right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enablement {
    Enabled,
    /// Shown, but it does nothing. The text is the key of the reason a
    /// surface shows instead, such as `quick.footer.prd_select_first`.
    Disabled(&'static str),
    /// Not shown, because it does not apply: the app is locked, nothing is
    /// selected, or the entry lacks what the command needs.
    Hidden,
}

/// What a command's availability depends on, as the surface asking sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandCtx {
    /// Every vault is locked. No command on an entry is offered then.
    pub locked: bool,
    /// The search field is empty, as it is when Quick Access opens.
    pub query_empty: bool,
    /// The selected entry, if any.
    pub selection: Option<Selection>,
}

/// The selected entry, as far as the commands care.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub kind: SelectedKind,
    /// The user picked this row, with the arrow keys, Cmd+1 to Cmd+9 or a
    /// click. A row that is selected because it came first, or because it
    /// is the only match, is not picked.
    pub explicit: bool,
    pub url: UrlKind,
    pub has_otp: bool,
    pub has_username: bool,
}

/// What the selected entry is, as far as its main action goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectedKind {
    /// An SAP system, `production` when it has the production role.
    Sap {
        production: bool,
    },
    Remote,
    Web,
}

impl SelectedKind {
    pub fn is_production(self) -> bool {
        matches!(self, SelectedKind::Sap { production: true })
    }
}

/// What the selected entry's address field holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UrlKind {
    /// Nothing, or only spaces.
    Empty,
    /// An http or https address with a host. An address without a scheme,
    /// such as `github.com/login`, counts too: it opens as https, as in the
    /// main window.
    Web,
    /// Anything else: `ftp:`, `javascript:`, `file:` and other schemes, and
    /// text that is no address. It can be copied but is never opened. URL
    /// rules read `localhost:8080` as the scheme `localhost`, so a host and
    /// port without `https://` lands here as well.
    Other,
}

impl UrlKind {
    pub fn of(url: &str) -> UrlKind {
        let url = url.trim();
        if url.is_empty() {
            return UrlKind::Empty;
        }
        let parsed = match Url::parse(url) {
            Err(ParseError::RelativeUrlWithoutBase) => Url::parse(&format!("https://{url}")),
            parsed => parsed,
        };
        match parsed {
            Ok(parsed)
                if matches!(parsed.scheme(), "http" | "https") && parsed.host().is_some() =>
            {
                UrlKind::Web
            }
            _ => UrlKind::Other,
        }
    }
}

/// Every command, in the order a list of them shows them.
pub fn commands() -> &'static [CommandSpec] {
    commands::COMMANDS
}

/// The command with this name.
pub fn command(id: CommandId) -> Option<&'static CommandSpec> {
    commands().iter().find(|spec| spec.id == id)
}

/// The command Return runs on an entry of this kind: signing in to an SAP
/// system, opening a web login's website, copying a remote access entry's
/// password. Whether Return may run it right now is `entry.primary`'s
/// availability, which holds the rule for production systems.
pub fn primary_command(kind: &SelectedKind) -> CommandId {
    match kind {
        SelectedKind::Sap { .. } => CommandId::ENTRY_SAP_LOGIN,
        SelectedKind::Remote => CommandId::ENTRY_COPY_PASSWORD,
        SelectedKind::Web => CommandId::ENTRY_OPEN_WEBSITE,
    }
}

/// Return's word for an entry of this kind: the word of its main action,
/// Sign in, Open website or Copy password. Wherever an entry is in view,
/// Return goes by this word. `entry.primary`'s own word is for lists of
/// shortcuts, where there is no entry to name an action for.
pub fn primary_label(kind: &SelectedKind) -> SharedString {
    // Every main action is a row in the table; the tests make sure of it.
    command(primary_command(kind))
        .map_or_else(|| tr!("action.primary.label"), |spec| (spec.label)())
}

/// The commands `surface` shows in this situation, in table order, each with
/// its availability. Hidden commands are left out.
pub fn for_surface(
    surface: Surface,
    ctx: &CommandCtx,
) -> impl Iterator<Item = (&'static CommandSpec, Enablement)> + use<> {
    let ctx = *ctx;
    commands()
        .iter()
        .filter(move |spec| spec.surfaces.contains(&surface))
        .filter_map(move |spec| match (spec.enabled)(&ctx) {
            Enablement::Hidden => None,
            enablement => Some((spec, enablement)),
        })
}

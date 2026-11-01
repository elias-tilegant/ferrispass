//! The commands the new surfaces offer, in one table.
//!
//! Quick Access and the menu bar item's menu take their commands from here,
//! and the main window's menus and the command palette will too: what a
//! command is called, its keys, where it appears and what it acts on. Every
//! surface shows the same word for a command, `action.<verb>.label` in
//! `locales/action.yml`, so a command never goes by two names.
//!
//! Nothing here binds a key or builds a menu yet. The surfaces do that as
//! they are built; until then the old main window keeps its own bindings and
//! menus in `src/app/actions.rs`.

#![forbid(unsafe_code)]

mod commands;
#[cfg(test)]
mod tests;

use gpui::SharedString;

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
    /// Whether the command reads a password or a one-time code, or hands
    /// one on.
    pub touches_secret: bool,
}

/// Every command, in the order a list of them shows them.
pub fn commands() -> &'static [CommandSpec] {
    commands::COMMANDS
}

/// The command with this name.
pub fn command(id: CommandId) -> Option<&'static CommandSpec> {
    commands().iter().find(|spec| spec.id == id)
}

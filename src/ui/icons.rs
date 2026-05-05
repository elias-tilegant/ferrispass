//! Project-local icon glyphs that aren't in `gpui_component::IconName`.
//!
//! Render with the existing `gpui_component::Icon` machinery so they pick up theme color.

use gpui::SharedString;
use gpui_component::IconNamed;

#[derive(Debug, Clone, Copy)]
pub enum AppIcon {
    Lock,
    Unlock,
    Cloud,
    Refresh,
    Key,
    Shield,
    Dot,
    Sync,
    Note,
}

impl IconNamed for AppIcon {
    fn path(self) -> SharedString {
        match self {
            AppIcon::Lock => "icons/app/lock.svg",
            AppIcon::Unlock => "icons/app/unlock.svg",
            AppIcon::Cloud => "icons/app/cloud.svg",
            AppIcon::Refresh => "icons/app/refresh.svg",
            AppIcon::Key => "icons/app/key.svg",
            AppIcon::Shield => "icons/app/shield.svg",
            AppIcon::Dot => "icons/app/dot.svg",
            AppIcon::Sync => "icons/app/sync.svg",
            AppIcon::Note => "icons/app/note.svg",
        }
        .into()
    }
}

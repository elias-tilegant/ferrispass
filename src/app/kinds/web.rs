//! Web logins: every entry that is neither an SAP system nor remote access.

use crate::domain::VaultEntry;

use super::Source;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebLogin {
    /// Whether the entry has a URL at all. Which URLs may be opened is
    /// decided where they are opened, not here.
    pub has_url: bool,
    pub source: Source,
}

impl WebLogin {
    pub(super) fn read(entry: &VaultEntry, source: Source) -> Self {
        Self {
            has_url: !entry.url.trim().is_empty(),
            source,
        }
    }
}

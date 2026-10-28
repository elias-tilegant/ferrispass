//! Remote access: the VPN or remote desktop client someone signs in to before
//! they can reach anything else.

use crate::domain::VaultEntry;

use super::{Source, field};

/// Names the program, e.g. `GlobalProtect`. Known names are matched in any
/// case; anything else is kept as written.
pub const KEY_REMOTE_CLIENT: &str = "REMOTE_CLIENT";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteProgram {
    GlobalProtect,
    Cisco,
    Forti,
    Citrix,
    Workspot,
    /// A program without a name of its own here, spelled as the
    /// `REMOTE_CLIENT` field spells it.
    Other(String),
}

impl RemoteProgram {
    /// One of the known programs, by its name in any case. FortiClient counts
    /// as Forti.
    fn known(name: &str) -> Option<Self> {
        [
            ("globalprotect", Self::GlobalProtect),
            ("cisco", Self::Cisco),
            ("forti", Self::Forti),
            ("forticlient", Self::Forti),
            ("citrix", Self::Citrix),
            ("workspot", Self::Workspot),
        ]
        .into_iter()
        .find(|(known, _)| name.eq_ignore_ascii_case(known))
        .map(|(_, program)| program)
    }

    /// Reads a `REMOTE_CLIENT` value, which the caller has trimmed: the whole
    /// value has to be a known name, otherwise it is kept as [`Self::Other`].
    pub fn from_client_field(value: &str) -> Self {
        Self::known(value).unwrap_or_else(|| Self::Other(value.to_string()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteAccess {
    /// `None` when nothing names the program, e.g. an entry titled
    /// "Nordhafen VPN".
    pub program: Option<RemoteProgram>,
    pub source: Source,
}

impl RemoteAccess {
    /// The program comes from `REMOTE_CLIENT`.
    pub(super) fn read(entry: &VaultEntry, source: Source) -> Self {
        Self {
            program: field(entry, KEY_REMOTE_CLIENT).map(RemoteProgram::from_client_field),
            source,
        }
    }
}

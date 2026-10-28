//! Remote access: the VPN or remote desktop client someone signs in to before
//! they can reach anything else.

use crate::domain::VaultEntry;

use super::{Source, field, is_word_char};

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
    /// The program comes from `REMOTE_CLIENT`, else from the first program
    /// the title, the URL or the tags name.
    pub(super) fn read(entry: &VaultEntry, source: Source) -> Self {
        Self {
            program: field(entry, KEY_REMOTE_CLIENT)
                .map(RemoteProgram::from_client_field)
                .or_else(|| named_program(entry)),
            source,
        }
    }
}

/// Remote access recognised by a word in the title, the URL or a tag: VPN or
/// the name of a known program. VPN alone leaves the program open.
pub(super) fn guess(entry: &VaultEntry) -> Option<RemoteAccess> {
    let program = named_program(entry);
    let found = program.is_some() || words(entry).any(|word| word.eq_ignore_ascii_case("vpn"));
    found.then_some(RemoteAccess {
        program,
        source: Source::Heuristic,
    })
}

/// The first known program named as a word, looking at the title, then the
/// URL, then the tags.
fn named_program(entry: &VaultEntry) -> Option<RemoteProgram> {
    words(entry).find_map(RemoteProgram::known)
}

fn words(entry: &VaultEntry) -> impl Iterator<Item = &str> {
    [entry.title.as_str(), entry.url.as_str()]
        .into_iter()
        .chain(entry.tags.iter().map(String::as_str))
        .flat_map(|text| text.split(|c: char| !is_word_char(c)))
        .filter(|word| !word.is_empty())
}

//! What an entry is: an SAP system, remote access or a web login.
//!
//! The kind is worked out from what the entry already holds (custom fields,
//! title, tags and URL) every time it is needed. Nothing is written back:
//! these functions take shared references and return plain data, so an entry
//! looks exactly the same to KeePassXC and to older versions of FerrisPass
//! whatever kind it is shown as. Nothing here knows about the UI either.
//!
//! ## Rules
//!
//! The first rule that matches decides:
//!
//! 1. A type marker, when the caller has one. Entries carry no marker yet, so
//!    every caller passes `None` for now. `sap`, `remote` and `web` are
//!    understood in any case; any other marker is ignored and the rules below
//!    decide.
//! 2. `SAP_HOST` and `SAP_INSTANCE` both set, or a valid `SAP_SID`: an SAP
//!    system, [`Source::Fields`].
//! 3. `REMOTE_CLIENT` set: remote access, [`Source::Fields`], with the
//!    program the field names.
//! 4. A title that starts with a SID and a three-digit client, like
//!    "NHP 100" (see [`title_pattern`]): an SAP system, [`Source::Title`].
//! 5. The title, the URL or a tag contains one of the words VPN,
//!    GlobalProtect, Cisco, Forti, FortiClient, Citrix or Workspot, in any
//!    case: remote access, [`Source::Heuristic`]. A word is a run of letters,
//!    digits and underscores, so "vpn.example.com" counts and "myvpn" does
//!    not.
//! 6. Anything else is a web login, [`Source::Fallback`].
//!
//! A field that holds nothing but whitespace counts as empty in every rule.
//! The order matters where rules overlap: "NHP 100" tagged VPN is an SAP
//! system, and an entry with both `SAP_SID` and `REMOTE_CLIENT` is too.

mod remote;
mod sap;
mod web;

#[cfg(test)]
mod tests;

pub use remote::{KEY_REMOTE_CLIENT, RemoteAccess, RemoteProgram};
pub use sap::{KEY_LINE, KEY_ROLE, KEY_SID, SapRole, SapSystem, Sid, role_from_sid, title_pattern};
pub use web::WebLogin;

use crate::domain::{VaultEntry, VaultSnapshot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Sap(SapSystem),
    Remote(RemoteAccess),
    Web(WebLogin),
}

impl EntryKind {
    pub fn source(&self) -> Source {
        match self {
            Self::Sap(sap) => sap.source,
            Self::Remote(remote) => remote.source,
            Self::Web(web) => web.source,
        }
    }

    /// True only for an SAP system in the production role.
    pub fn is_production(&self) -> bool {
        matches!(self, Self::Sap(sap) if sap.is_production())
    }
}

/// Which rule decided the kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The type marker the caller passed.
    Marker,
    /// Custom fields that belong to the kind (`SAP_*`, `REMOTE_CLIENT`).
    Fields,
    /// A title of the form "SID client".
    Title,
    /// A telling word in the title, the URL or a tag.
    Heuristic,
    /// Nothing matched.
    Fallback,
}

impl Source {
    /// Whether the kind is a guess from the title, the URL or the tags rather
    /// than something the entry states. The UI shows such a kind as
    /// recognised, so nobody mistakes it for a choice they made.
    pub fn is_recognised(self) -> bool {
        matches!(self, Self::Title | Self::Heuristic)
    }
}

/// Works out the kind of `entry`, by the rules in the module docs. `marker` is
/// the entry's type marker, `None` until entries carry one.
pub fn classify(entry: &VaultEntry, marker: Option<&str>) -> EntryKind {
    if let Some(kind) = marker.and_then(|marker| from_marker(entry, marker)) {
        return kind;
    }
    if sap::has_sap_fields(entry) {
        return EntryKind::Sap(SapSystem::read(entry, Source::Fields));
    }
    if field(entry, KEY_REMOTE_CLIENT).is_some() {
        return EntryKind::Remote(RemoteAccess::read(entry, Source::Fields));
    }
    if title_pattern(&entry.title).is_some() {
        return EntryKind::Sap(SapSystem::read(entry, Source::Title));
    }
    if let Some(remote) = remote::guess(entry) {
        return EntryKind::Remote(remote);
    }
    EntryKind::Web(WebLogin::read(entry, Source::Fallback))
}

/// The kind of every entry in the snapshot, the Recycle Bin included, in the
/// order [`VaultSnapshot::entries_recursive`] lists them.
pub fn classify_all(snapshot: &VaultSnapshot) -> Vec<(String, EntryKind)> {
    snapshot
        .entries_recursive()
        .into_iter()
        .map(|entry| (entry.id.clone(), classify(entry, None)))
        .collect()
}

fn from_marker(entry: &VaultEntry, marker: &str) -> Option<EntryKind> {
    let marker = marker.trim();
    if marker.eq_ignore_ascii_case("sap") {
        Some(EntryKind::Sap(SapSystem::read(entry, Source::Marker)))
    } else if marker.eq_ignore_ascii_case("remote") {
        Some(EntryKind::Remote(RemoteAccess::read(entry, Source::Marker)))
    } else if marker.eq_ignore_ascii_case("web") {
        Some(EntryKind::Web(WebLogin::read(entry, Source::Marker)))
    } else {
        None
    }
}

/// The value of the custom field `key`, trimmed. `None` when the entry has no
/// such field or the value is only whitespace.
///
/// Takes the first field with that key, as the SAP launcher does. An entry
/// read from a vault has each key once: keepass-rs keeps an entry's fields in
/// a map.
fn field<'a>(entry: &'a VaultEntry, key: &str) -> Option<&'a str> {
    entry
        .custom_fields
        .iter()
        .find(|field| field.key == key)
        .map(|field| field.value.trim())
        .filter(|value| !value.is_empty())
}

/// Letters, digits and the underscore: what `\w` matches in a regular
/// expression, and so what a word boundary is measured against.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

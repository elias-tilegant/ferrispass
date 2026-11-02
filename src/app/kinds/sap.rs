//! SAP systems: the SID, the client and the role an entry stands for.
//!
//! The connection keys the SAP GUI launcher already reads (`SAP_HOST`,
//! `SAP_INSTANCE`, `SAP_CLIENT`, ...) are reused from there. This module adds
//! three keys of its own: `SAP_SID`, `SAP_ROLE` and `SAP_LINE`. Recognising an
//! entry as an SAP system does not make it launchable; the launcher keeps its
//! own check for a host and an instance.

use crate::domain::VaultEntry;
use crate::launch::sap::{KEY_CLIENT, KEY_HOST, KEY_INSTANCE, KEY_LANG, KEY_USER};

use super::{Source, field, is_word_char};

/// The three-character system ID, e.g. `NHP`.
pub const KEY_SID: &str = "SAP_SID";
/// `DEV`, `QAS`, `PRD` or `SBX`, in any case. Overrides the role the last
/// letter of the SID suggests.
pub const KEY_ROLE: &str = "SAP_ROLE";
/// Free text naming the system line the SID belongs to, e.g. `EWM`.
pub const KEY_LINE: &str = "SAP_LINE";

/// An SAP system ID: three characters, an uppercase ASCII letter followed by
/// two uppercase ASCII letters or digits. Lowercase is not accepted, so a
/// title like "nhp 100" stays an ordinary title.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Sid(String);

impl Sid {
    /// `value` must be the SID and nothing else; callers trim it first.
    pub fn parse(value: &str) -> Option<Self> {
        let mut chars = value.chars();
        let first = chars.next()?;
        let second = chars.next()?;
        let third = chars.next()?;
        let valid = chars.next().is_none()
            && first.is_ascii_uppercase()
            && [second, third]
                .iter()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
        valid.then(|| Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Where a system sits in the landscape. Production is the one to be careful
/// with, see [`SapSystem::is_production`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SapRole {
    Dev,
    Qas,
    Prd,
    Sbx,
}

impl SapRole {
    /// Reads a `SAP_ROLE` value. Only the four names count, in any case;
    /// anything else is no role.
    pub fn parse(value: &str) -> Option<Self> {
        [
            ("DEV", Self::Dev),
            ("QAS", Self::Qas),
            ("PRD", Self::Prd),
            ("SBX", Self::Sbx),
        ]
        .into_iter()
        .find(|(name, _)| value.eq_ignore_ascii_case(name))
        .map(|(_, role)| role)
    }
}

/// The role the naming convention puts in the last letter of the SID: D for
/// development, Q for quality assurance, P for production, S for sandbox.
/// Any other last character, a digit included, says nothing.
pub fn role_from_sid(sid: &Sid) -> Option<SapRole> {
    match sid.as_str().chars().last()? {
        'D' => Some(SapRole::Dev),
        'Q' => Some(SapRole::Qas),
        'P' => Some(SapRole::Prd),
        'S' => Some(SapRole::Sbx),
        _ => None,
    }
}

/// Reads a title of the form "SID client", like "NHP 100" or
/// "BRQ 200 quality": a SID at the very start, whitespace, then exactly three
/// ASCII digits that are not followed by a letter, digit or underscore. As a
/// regular expression: `^([A-Z][A-Z0-9]{2})\s+(\d{3})\b`.
pub fn title_pattern(title: &str) -> Option<(Sid, String)> {
    let sid_end = title.char_indices().nth(3).map_or(title.len(), |(i, _)| i);
    let sid = Sid::parse(&title[..sid_end])?;
    let rest = &title[sid_end..];
    let client_start = rest.trim_start_matches(char::is_whitespace);
    if client_start.len() == rest.len() {
        return None;
    }
    let client = client_start.get(..3)?;
    if !client.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if client_start[3..].chars().next().is_some_and(is_word_char) {
        return None;
    }
    Some((sid, client.to_string()))
}

/// Whether the fields alone make this an SAP system: a host and an instance,
/// or a valid SID.
pub(super) fn has_sap_fields(entry: &VaultEntry) -> bool {
    let connection = field(entry, KEY_HOST).is_some() && field(entry, KEY_INSTANCE).is_some();
    connection || field(entry, KEY_SID).and_then(Sid::parse).is_some()
}

/// What the entry says about the SAP system it stands for. Every field is
/// optional: an entry recognised by its title alone has a SID and a client
/// and nothing else.
#[derive(Clone, PartialEq, Eq)]
pub struct SapSystem {
    pub sid: Option<Sid>,
    pub client: Option<String>,
    pub role: Option<SapRole>,
    pub line: Option<String>,
    pub host: Option<String>,
    pub instance: Option<String>,
    /// The `SAP_USER` override, or the entry's own username without one;
    /// the same user the launcher logs in with.
    pub user: Option<String>,
    pub lang: Option<String>,
    pub source: Source,
}

impl SapSystem {
    /// Collects everything the entry has, whichever rule made it an SAP
    /// system. The SID comes from `SAP_SID`, else from the title; the client
    /// from `SAP_CLIENT`, else from the title; the role from `SAP_ROLE`, else
    /// from the last letter of the SID.
    pub(super) fn read(entry: &VaultEntry, source: Source) -> Self {
        let from_title = title_pattern(&entry.title);
        let sid = field(entry, KEY_SID)
            .and_then(Sid::parse)
            .or_else(|| from_title.as_ref().map(|(sid, _)| sid.clone()));
        let client = field(entry, KEY_CLIENT)
            .map(str::to_string)
            .or_else(|| from_title.map(|(_, client)| client));
        let role = field(entry, KEY_ROLE)
            .and_then(SapRole::parse)
            .or_else(|| sid.as_ref().and_then(role_from_sid));
        let user = field(entry, KEY_USER)
            .map(str::to_string)
            .or_else(|| (!entry.username.trim().is_empty()).then(|| entry.username.clone()));
        Self {
            sid,
            client,
            role,
            line: field(entry, KEY_LINE).map(str::to_string),
            host: field(entry, KEY_HOST).map(str::to_string),
            instance: field(entry, KEY_INSTANCE).map(str::to_string),
            user,
            lang: field(entry, KEY_LANG).map(str::to_string),
            source,
        }
    }

    pub fn is_production(&self) -> bool {
        self.role == Some(SapRole::Prd)
    }
}

/// Host and user stay out of debug output, like the entry's own fields.
impl std::fmt::Debug for SapSystem {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SapSystem")
            .field("sid", &self.sid)
            .field("client", &self.client)
            .field("role", &self.role)
            .field("line", &self.line)
            .field("has_host", &self.host.is_some())
            .field("has_instance", &self.instance.is_some())
            .field("has_user", &self.user.is_some())
            .field("has_lang", &self.lang.is_some())
            .field("source", &self.source)
            .finish()
    }
}

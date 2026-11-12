//! The quick access panel's model: what it searched for, what it found,
//! which row is selected, and what its footer says.
//!
//! The panel searches the open vault and acts on the entry it selected
//! itself; it never reads or moves the main window's search or selection.
//! The window is built anew every time the panel opens, and this model
//! outlives it: a search and its selection are still there when the panel
//! opens again within [`MEMORY`]. Locking forgets them at once.
//!
//! The rules it keeps:
//!
//! - At most [`MAX_HITS`] rows, one for each of ⌘1 to ⌘9.
//! - A new query selects the first row. Results that change under the same
//!   query, as after a sync, keep the selected entry where it still is.
//! - From an empty search, Return never acts on a production system the user
//!   has not picked with ↓ or ⌘1 to ⌘9. The footer says so instead of naming
//!   an action, and [`FooterModel::primary`] is `None`. The rule itself is
//!   the registry's, the availability of `entry.primary`; this model asks it.
//! - When the selected row stands for the same system as other rows, the
//!   same SID and client at two customers, say, the footer counts them
//!   ("2 identical matches"). While Return waits for a production system to
//!   be picked, the footer says that instead.
//! - A row is named after its entry: kind and title, the role in words, the
//!   client, the user and the vault. The letters of a role, such as PRD,
//!   stand on the row's chip and never in its name.
//! - The panel reads an entry without its notes and its protected custom
//!   fields ([`shown_fields`]), so neither reaches a row or the footer.
//! - Nothing here writes to a log, and no text here holds a password or a
//!   one-time code.

use std::time::{Duration, Instant};

use gpui::SharedString;

use crate::app::kinds::{self, EntryKind, SapRole};
use crate::app::registry::{
    self, CommandCtx, CommandId, Enablement, SelectedKind, Selection, UrlKind,
};
use crate::app::{AppState, VaultStatus};
use crate::domain::VaultEntry;
use crate::ui::ds::components::{
    ChipTone, FooterTone, ListSelection, ResultsDrawn, RowParts, VaultBadge, move_selection,
    select_by_number,
};
use crate::{tp, tr};

/// The most rows the panel lists, one for each of ⌘1 to ⌘9.
pub const MAX_HITS: usize = 9;

/// How long the panel keeps a search and its selection after it closes.
pub const MEMORY: Duration = Duration::from_secs(2 * 60);

/// One row of the panel: the entry it stands for, by id, and what the row
/// and the footer show of it. Its `Debug` output leaves out everything but
/// the id and the flags: the title, the user and the host stay out of any
/// log a `{:?}` could reach.
#[derive(Clone, PartialEq)]
pub struct Hit {
    pub entry_id: String,
    pub parts: RowParts,
    pub kind: EntryKind,
    /// The chip after the title: the letters of an SAP system's role.
    pub chip: Option<(ChipTone, SharedString)>,
    pub badge: VaultBadge,
    pub production: bool,
    pub url: UrlKind,
    pub has_username: bool,
    pub has_otp: bool,
    /// How many listed rows stand for the same system as this one, this one
    /// included: 1 for a row without a twin.
    pub twins: usize,
}

impl Hit {
    fn new(entry: &VaultEntry, twins: usize, vault: &SharedString) -> Self {
        let shown = shown_fields(entry);
        let kind = kinds::classify(&shown, None);
        let chip = match &kind {
            EntryKind::Sap(sap) => sap.role.map(|role| {
                let tone = if role == SapRole::Prd {
                    ChipTone::Production
                } else {
                    ChipTone::Neutral
                };
                (tone, SharedString::new_static(role_letters(role)))
            }),
            EntryKind::Remote(_) | EntryKind::Web(_) => None,
        };
        Self {
            entry_id: entry.id.clone(),
            parts: row_parts(&shown, &kind, vault),
            production: kind.is_production(),
            chip,
            // Vaults carry no kind yet, so every row shows the badge of the
            // user's own vault; the row's name says which vault it is.
            badge: VaultBadge::Personal,
            url: UrlKind::of(&shown.url),
            has_username: !shown.username.trim().is_empty(),
            has_otp: shown.has_otp,
            twins,
            kind,
        }
    }

    /// What the registry needs to know about the entry's kind.
    pub fn selected_kind(&self) -> SelectedKind {
        match &self.kind {
            EntryKind::Sap(_) => SelectedKind::Sap {
                production: self.production,
            },
            EntryKind::Remote(_) => SelectedKind::Remote,
            EntryKind::Web(_) => SelectedKind::Web,
        }
    }
}

impl std::fmt::Debug for Hit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Hit")
            .field("entry_id", &self.entry_id)
            .field("production", &self.production)
            .field("has_username", &self.has_username)
            .field("has_otp", &self.has_otp)
            .field("twins", &self.twins)
            .finish_non_exhaustive()
    }
}

/// The letters of a role, as the chip shows them.
fn role_letters(role: SapRole) -> &'static str {
    match role {
        SapRole::Prd => "PRD",
        SapRole::Qas => "QAS",
        SapRole::Dev => "DEV",
        SapRole::Sbx => "SBX",
    }
}

/// A role in words, as a row's name says it.
fn role_word(role: SapRole) -> SharedString {
    match role {
        SapRole::Prd => tr!("glossary.role.production_system"),
        SapRole::Qas => tr!("glossary.role.quality_system"),
        SapRole::Dev => tr!("glossary.role.development_system"),
        SapRole::Sbx => tr!("glossary.role.sandbox_system"),
    }
}

/// The kind of an entry as a word.
fn kind_word(kind: &EntryKind) -> SharedString {
    match kind {
        EntryKind::Sap(_) => tr!("glossary.type.sap_system"),
        EntryKind::Remote(_) => tr!("glossary.type.remote_access"),
        EntryKind::Web(_) => tr!("glossary.type.web_login"),
    }
}

/// What a row says about `entry` of `kind` in `vault`: the kind and the
/// title, the role in words, the client, the user and the vault. A part the
/// entry does not have is left out. The role is always written out, so the
/// name of a production system says "Production system" and never the
/// letters on its chip.
pub fn row_parts(entry: &VaultEntry, kind: &EntryKind, vault: &SharedString) -> RowParts {
    let mut parts = RowParts::new(kind_word(kind), entry.title.clone());
    let mut user = entry.username.trim().to_string();
    if let EntryKind::Sap(sap) = kind {
        if let Some(role) = sap.role {
            parts = parts.role_word(role_word(role));
        }
        if let Some(client) = sap
            .client
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
        {
            parts = parts.client(format!("{} {client}", tr!("glossary.sap_client")));
        }
        if let Some(sap_user) = sap.user.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
            user = sap_user.to_string();
        }
    }
    if !user.is_empty() {
        parts = parts.user(user);
    }
    if !vault.trim().is_empty() {
        parts = parts.vault(vault.clone());
    }
    parts
}

/// What the panel reads of an entry: everything but its notes and its
/// protected custom fields. A protected `SAP_USER` or `SAP_SID` therefore
/// counts as not set, as it does for the search, and never reaches a row's
/// name or the footer.
///
/// Built field by field, so neither is copied, and so a field added to
/// `VaultEntry` does not compile here until someone decides whether the
/// panel may read it.
pub fn shown_fields(entry: &VaultEntry) -> VaultEntry {
    let VaultEntry {
        id,
        title,
        username,
        url,
        notes: _,
        has_password,
        password_length,
        has_otp,
        updated,
        tags,
        starred,
        favicon: _,
        strength,
        group_path,
        in_recycle_bin,
        auto_type_enabled,
        auto_type_windows,
        custom_fields,
    } = entry;
    VaultEntry {
        id: id.clone(),
        title: title.clone(),
        username: username.clone(),
        url: url.clone(),
        notes: String::new(),
        has_password: *has_password,
        password_length: *password_length,
        has_otp: *has_otp,
        updated: updated.clone(),
        tags: tags.clone(),
        starred: *starred,
        favicon: Default::default(),
        strength: *strength,
        group_path: group_path.clone(),
        in_recycle_bin: *in_recycle_bin,
        auto_type_enabled: *auto_type_enabled,
        auto_type_windows: auto_type_windows.clone(),
        custom_fields: custom_fields
            .iter()
            .filter(|field| !field.protected)
            .cloned()
            .collect(),
    }
}

/// What the footer says after an action, until the selection or the query
/// changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FooterNote {
    CopiedPassword,
    CopiedUser,
    CopiedCode,
    /// The password could not be copied, or the entry has none.
    CopyFailed,
    /// The website did not open: its address is not http or https.
    WebsiteFailed,
}

impl FooterNote {
    fn text(self) -> SharedString {
        match self {
            Self::CopiedPassword => tr!("quick.footer.copied.password"),
            Self::CopiedUser => tr!("quick.footer.copied.user"),
            Self::CopiedCode => tr!("quick.footer.copied.one_time_code"),
            Self::CopyFailed => tr!("quick.error.copy_failed"),
            Self::WebsiteFailed => tr!("quick.error.website_failed"),
        }
    }
}

/// What the footer shows, decided here and drawn by `PanelFooter`. No
/// `Debug`: its words name the system and the user.
#[derive(Clone, PartialEq)]
pub struct FooterModel {
    /// What Return runs on the selected entry, or `None` when Return does
    /// nothing: no row, or a production system nobody picked.
    pub primary: Option<CommandId>,
    /// The first line: the keys and the words. The words say what Return
    /// does, with the target and the user where the entry has them, or why
    /// it waits; the keys are empty when the sentence names them itself.
    pub main: Option<(&'static str, SharedString)>,
    pub tone: FooterTone,
    /// The shortcuts on the second line.
    pub more: Vec<CommandId>,
    /// A sentence at the end of the second line: what an action did, or
    /// that other vaults are switched to in the main window.
    pub note: Option<SharedString>,
}

/// The panel's search, results and selection. Its `Debug` output gives the
/// length of the query, never the query: what someone types is often a
/// title.
#[derive(Clone)]
pub struct PanelVm {
    query: String,
    results: Vec<Hit>,
    selection: ListSelection,
    /// The user picked the selected row with ↑, ↓ or ⌘1 to ⌘9. A row that
    /// is selected because it came first is not picked.
    explicit: bool,
    drawn: ResultsDrawn,
    /// When the panel last closed, on GPUI's clock.
    remembered_at: Option<Instant>,
    /// The name of the open vault, or `None` while no vault is open.
    vault_name: Option<SharedString>,
    /// Vaults unlocked besides the open one. The panel searches the open
    /// one only.
    parked_vaults: usize,
    footer_note: Option<FooterNote>,
}

impl std::fmt::Debug for PanelVm {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PanelVm")
            .field("query_len", &self.query.chars().count())
            .field("results", &self.results)
            .field("selection", &self.selection)
            .field("explicit", &self.explicit)
            .field("remembered_at", &self.remembered_at)
            .field("vault_open", &self.vault_name.is_some())
            .field("parked_vaults", &self.parked_vaults)
            .field("footer_note", &self.footer_note)
            .finish_non_exhaustive()
    }
}

impl Default for PanelVm {
    fn default() -> Self {
        Self {
            query: String::new(),
            results: Vec::new(),
            selection: ListSelection::first(0),
            explicit: false,
            drawn: ResultsDrawn::new(),
            remembered_at: None,
            vault_name: None,
            parked_vaults: 0,
            footer_note: None,
        }
    }
}

impl PanelVm {
    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn results(&self) -> &[Hit] {
        &self.results
    }

    pub fn selection(&self) -> ListSelection {
        self.selection
    }

    /// The selected row, if any.
    pub fn selected(&self) -> Option<&Hit> {
        self.selection
            .index()
            .and_then(|index| self.results.get(index))
    }

    /// Whether the user picked the selected row.
    pub fn is_explicit(&self) -> bool {
        self.explicit
    }

    /// When the list first drew the current results.
    pub fn drawn(&self) -> &ResultsDrawn {
        &self.drawn
    }

    /// The open vault's name, or `None` while no vault is open, when the
    /// panel shows that it needs the main window.
    pub fn vault_name(&self) -> Option<&SharedString> {
        self.vault_name.as_ref()
    }

    pub fn parked_vaults(&self) -> usize {
        self.parked_vaults
    }

    /// Reads the open vault again for the current query. The selected entry
    /// stays selected where it is still listed; otherwise the first row is.
    pub fn refresh(&mut self, state: &AppState) {
        let VaultStatus::Open { path, .. } = state.vault_status() else {
            self.vault_name = None;
            self.parked_vaults = 0;
            self.set_results(Vec::new());
            return;
        };
        let name: SharedString = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default()
            .into();
        self.parked_vaults = state
            .unlocked_paths()
            .iter()
            .filter(|unlocked| *unlocked != path)
            .count();
        let hits = state
            .ranked_for_panel(&self.query, MAX_HITS)
            .iter()
            .map(|(entry, twins)| Hit::new(entry, *twins, &name))
            .collect();
        self.vault_name = Some(name);
        self.set_results(hits);
    }

    /// A new query: the results for it, with the first row selected.
    pub fn set_query(&mut self, query: impl Into<String>, state: &AppState) {
        let query = query.into();
        if query == self.query {
            return;
        }
        self.query = query;
        self.results.clear();
        self.selection = ListSelection::first(0);
        self.explicit = false;
        self.footer_note = None;
        self.refresh(state);
    }

    fn set_results(&mut self, hits: Vec<Hit>) {
        let kept = self.selected().map(|hit| hit.entry_id.clone());
        let same_rows = hits.len() == self.results.len()
            && hits
                .iter()
                .zip(&self.results)
                .all(|(new, old)| new.entry_id == old.entry_id);
        if !same_rows {
            self.drawn.clear();
        }
        self.results = hits;
        let index = kept.and_then(|id| self.results.iter().position(|hit| hit.entry_id == id));
        match index {
            Some(index) => self.selection = ListSelection::new(Some(index), self.results.len()),
            None => {
                self.selection = ListSelection::first(self.results.len());
                self.explicit = false;
            }
        }
    }

    /// ↑ (`-1`) or ↓ (`1`). The selection stops at the first and the last
    /// row, and the row it lands on counts as picked.
    pub fn move_selection(&mut self, delta: isize) {
        if self.results.is_empty() {
            return;
        }
        self.selection = move_selection(self.selection, delta);
        self.explicit = true;
        self.footer_note = None;
    }

    /// ⌘ and `number`, 1 to 9: picks that row. A number past the end of the
    /// list changes nothing.
    pub fn select_number(&mut self, number: u8) {
        let next = select_by_number(self.selection, number);
        if next.index() != Some(usize::from(number).wrapping_sub(1)) {
            return;
        }
        self.selection = next;
        self.explicit = true;
        self.footer_note = None;
    }

    /// Counts `parked` vaults as unlocked besides the open one, for tests
    /// that cannot park a vault in `AppState`.
    #[cfg(test)]
    pub(crate) fn assume_parked_vaults(&mut self, parked: usize) {
        self.parked_vaults = parked;
    }

    /// Shows `note` in the footer until the selection or the query changes.
    pub fn note(&mut self, note: FooterNote) {
        self.footer_note = Some(note);
    }

    /// The panel closed at `now`.
    pub fn remember(&mut self, now: Instant) {
        self.remembered_at = Some(now);
        self.footer_note = None;
    }

    /// The panel opens at `now`: a search older than [`MEMORY`] is
    /// forgotten, with its selection.
    pub fn forget_if_stale(&mut self, now: Instant) {
        let stale = self
            .remembered_at
            .is_some_and(|closed| now.saturating_duration_since(closed) >= MEMORY);
        if stale {
            self.forget();
        }
    }

    /// Forgets the search, the results and the selection, as locking does.
    pub fn forget(&mut self) {
        *self = Self::default();
    }

    /// What the footer shows for the selected row.
    pub fn footer(&self) -> FooterModel {
        let note = self
            .footer_note
            .map(FooterNote::text)
            .or_else(|| (self.parked_vaults > 0).then(|| tr!("quick.footer.other_vaults")));
        if self.vault_name.is_none() {
            return FooterModel {
                primary: None,
                main: Some(("cmd-o", tr!("action.open.label"))),
                tone: FooterTone::Normal,
                more: Vec::new(),
                note: None,
            };
        }
        let Some(hit) = self.selected() else {
            return FooterModel {
                primary: None,
                main: None,
                tone: FooterTone::Normal,
                more: Vec::new(),
                note,
            };
        };
        let kind = hit.selected_kind();
        let ctx = CommandCtx {
            locked: false,
            query_empty: self.query.trim().is_empty(),
            selection: Some(Selection {
                kind,
                explicit: self.explicit,
                url: hit.url,
                has_otp: hit.has_otp,
                has_username: hit.has_username,
            }),
        };
        let main_action = registry::primary_command(&kind);
        let returns = registry::command(CommandId::ENTRY_PRIMARY)
            .map_or(Enablement::Hidden, |spec| (spec.enabled)(&ctx));
        let (primary, main, tone) = match returns {
            Enablement::Enabled => (
                Some(main_action),
                Some(("enter", main_words(hit))),
                FooterTone::Normal,
            ),
            Enablement::Disabled(_) => (
                None,
                Some(("", tr!("quick.footer.prd_select_first"))),
                FooterTone::ProductionSelectFirst,
            ),
            Enablement::Hidden => (None, None, FooterTone::Normal),
        };
        // A twin of the selected row is counted, unless the footer says
        // that Return waits for a production system to be picked.
        let tone = if tone == FooterTone::Normal && hit.twins > 1 {
            FooterTone::Tie(tp!("quick.results.same_matches", hit.twins))
        } else {
            tone
        };
        let mut more = Vec::new();
        if main_action == CommandId::ENTRY_COPY_PASSWORD {
            if hit.has_username {
                more.push(CommandId::ENTRY_COPY_USERNAME);
            }
        } else {
            more.push(CommandId::ENTRY_COPY_PASSWORD);
        }
        more.push(CommandId::ENTRY_SHOW_IN_APP);
        FooterModel {
            primary,
            main,
            tone,
            more,
            note,
        }
    }
}

/// What Return does for `hit`, in words: signing in names the system, the
/// client and the user, when the entry has all three.
fn main_words(hit: &Hit) -> SharedString {
    match &hit.kind {
        EntryKind::Sap(sap) => {
            let sid = sap.sid.as_ref().map(|sid| sid.as_str().to_string());
            let client = sap
                .client
                .as_deref()
                .map(str::trim)
                .filter(|c| !c.is_empty());
            let user = sap.user.as_deref().map(str::trim).filter(|u| !u.is_empty());
            match (sid, client, user) {
                (Some(sid), Some(client), Some(user)) => {
                    tr!(
                        "quick.footer.sap_login",
                        sid = sid,
                        client = client,
                        user = user
                    )
                }
                _ => registry::primary_label(&hit.selected_kind()),
            }
        }
        EntryKind::Web(_) => tr!("quick.footer.open_website"),
        EntryKind::Remote(_) => tr!("quick.footer.copy_password"),
    }
}

/// The address the panel may open for an entry: http and https only, and
/// an address without a scheme as https, the way the main window opens it.
/// Anything else, `javascript:` and `file:` among them, is `None`.
pub fn web_url(url: &str) -> Option<String> {
    let url = url.trim();
    match UrlKind::of(url) {
        UrlKind::Web => {
            let lower = url.to_ascii_lowercase();
            if lower.starts_with("http://") || lower.starts_with("https://") {
                Some(url.to_string())
            } else {
                Some(format!("https://{url}"))
            }
        }
        UrlKind::Empty | UrlKind::Other => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_http_and_https_addresses_open() {
        assert_eq!(
            web_url("https://login.example.com").as_deref(),
            Some("https://login.example.com")
        );
        assert_eq!(
            web_url(" HTTP://intranet.example/start ").as_deref(),
            Some("HTTP://intranet.example/start")
        );
        assert_eq!(
            web_url("mail.example.com/login").as_deref(),
            Some("https://mail.example.com/login"),
            "an address without a scheme opens as https"
        );
        for refused in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "file:///etc/passwd",
            "ftp://files.example",
            "data:text/html,hello",
            "vbscript:run",
            "localhost:8080",
            "https://",
            "",
            "   ",
        ] {
            assert_eq!(web_url(refused), None, "{refused:?} would open");
        }
    }

    #[test]
    fn a_search_is_kept_for_two_minutes_after_the_panel_closes() {
        let start = Instant::now();
        let mut vm = PanelVm {
            query: "nh".to_string(),
            explicit: true,
            ..PanelVm::default()
        };
        vm.remember(start);
        // Two minutes, written out: the rule, not whatever the constant says.
        vm.forget_if_stale(start + Duration::from_secs(119));
        assert_eq!(vm.query(), "nh");
        assert!(vm.is_explicit());
        vm.forget_if_stale(start + Duration::from_secs(120));
        assert_eq!(vm.query(), "");
        assert!(!vm.is_explicit());
        // Never remembered, never forgotten for being old.
        let mut fresh = PanelVm {
            query: "x".to_string(),
            ..PanelVm::default()
        };
        fresh.forget_if_stale(start + Duration::from_secs(1200));
        assert_eq!(fresh.query(), "x");
    }
}

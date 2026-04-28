use crate::domain::{VaultEntry, VaultSnapshot};
use crate::keepass::{EntryDraft, MutationError, OtpDisplay, StrengthReport, VaultDocument};
use gpui::{AppContext as _, Context};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

#[derive(Debug, Default)]
pub struct AppState {
    vault: VaultStatus,
    overlay: Overlay,
    /// Background-save lifecycle of the open vault. Drives the status indicator
    /// and gates retry / explicit-save UX.
    save_status: SaveStatus,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SaveStatus {
    /// No save has happened since the vault was opened (the on-disk file is
    /// authoritative and equal to the in-memory state).
    #[default]
    Idle,
    /// A background save is in flight.
    Saving,
    /// The most recent save succeeded.
    Saved,
    /// The most recent save failed; message is suitable for a toast.
    Failed(String),
}

#[derive(Debug, Default)]
pub enum VaultStatus {
    #[default]
    Empty,
    AwaitingPassword {
        path: PathBuf,
        keyfile: Option<PathBuf>,
        error: Option<String>,
    },
    Opening {
        path: PathBuf,
    },
    Open {
        path: PathBuf,
        document: Box<VaultDocument>,
        selection: LibrarySelection,
        selected_entry_id: Option<String>,
        search_query: String,
        /// Pre-computed result of `entries_for_selection(selection, search_query)`,
        /// rebuilt only when selection / search changes. Sharing via `Rc` makes
        /// `vault_browser()` cheap on every render frame, which keeps scrolling
        /// smooth on large vaults (3 500+ entries).
        visible_entries: Rc<Vec<VaultEntry>>,
        /// Real `zxcvbn` score for the currently-selected entry. Computed once
        /// per selection change so the detail view can render an accurate bar
        /// without paying the ~1-5 ms zxcvbn cost on every frame.
        selected_strength: Option<StrengthReport>,
    },
    Error {
        message: String,
        path: Option<PathBuf>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibrarySelection {
    Group(String),
    AllItems,
    Favorites,
    RecentlyUsed,
    Trash,
    Tag(String),
}

impl LibrarySelection {
    pub fn group_id(&self) -> Option<&str> {
        match self {
            LibrarySelection::Group(id) => Some(id.as_str()),
            _ => None,
        }
    }

    pub fn tag(&self) -> Option<&str> {
        match self {
            LibrarySelection::Tag(name) => Some(name.as_str()),
            _ => None,
        }
    }

    pub fn is_all_items(&self) -> bool {
        matches!(self, LibrarySelection::AllItems)
    }
    pub fn is_favorites(&self) -> bool {
        matches!(self, LibrarySelection::Favorites)
    }
    pub fn is_recently_used(&self) -> bool {
        matches!(self, LibrarySelection::RecentlyUsed)
    }
    pub fn is_trash(&self) -> bool {
        matches!(self, LibrarySelection::Trash)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Overlay {
    #[default]
    None,
    /// Cloud provider picker (welcome → connect flow).
    Connect,
    /// Sync settings — full window over vault.
    SyncSettings,
    /// New entry modal — appears over the vault.
    AddEntry,
    /// Edit existing entry. Carries the entry id so the Save handler knows
    /// what to update; same modal layout as `AddEntry`, just a different
    /// header + save action.
    EditEntry { entry_id: String },
    /// Three-way conflict resolution.
    Conflict,
}

impl Overlay {
    pub fn is_active(&self) -> bool {
        !matches!(self, Overlay::None)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnlockPrompt {
    pub path: PathBuf,
    pub file_name: String,
    pub display_path: String,
    pub keyfile: Option<PathBuf>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultSummary {
    pub title: String,
    pub subtitle: String,
    pub status: String,
    pub entries: usize,
    pub groups: usize,
    pub is_open: bool,
    pub is_busy: bool,
    /// Demo provider name; in this build always `Some("OneDrive")` once a vault is open.
    pub provider: Option<&'static str>,
    /// Demo "synced" timestamp string; static for the demo.
    pub synced_at: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub struct VaultBrowserModel {
    /// Cheap `Arc` clone of the current snapshot — held so renderers can read
    /// the group tree, recently-used count, etc. without re-cloning.
    pub snapshot: Arc<VaultSnapshot>,
    pub selection: LibrarySelection,
    pub selection_label: String,
    pub selected_entry_id: Option<String>,
    /// Currently-visible entries (after selection + search filter), shared by
    /// `Rc` so the virtual list, scroll handler, and detail-pane all read from
    /// the same allocation.
    pub entries: Rc<Vec<VaultEntry>>,
    pub selected_entry: Option<VaultEntry>,
    pub selected_strength: Option<StrengthReport>,
    pub search_query: String,
    pub showing_search_results: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyValueKind {
    Username,
    Url,
    Password,
}

impl AppState {
    pub fn vault_status(&self) -> &VaultStatus {
        &self.vault
    }

    pub fn overlay(&self) -> &Overlay {
        &self.overlay
    }

    pub fn open_overlay(&mut self, overlay: Overlay, cx: &mut Context<Self>) {
        if self.overlay == overlay {
            return;
        }
        self.overlay = overlay;
        cx.notify();
    }

    pub fn close_overlay(&mut self, cx: &mut Context<Self>) -> bool {
        if matches!(self.overlay, Overlay::None) {
            return false;
        }
        self.overlay = Overlay::None;
        cx.notify();
        true
    }

    pub fn request_password(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let keyfile = crate::keepass::KeePassRepository::suggested_keyfile(&path);
        self.vault = VaultStatus::AwaitingPassword {
            path,
            keyfile,
            error: None,
        };
        self.overlay = Overlay::None;
        cx.notify();
    }

    pub fn set_unlock_keyfile(&mut self, keyfile: Option<PathBuf>, cx: &mut Context<Self>) {
        if let VaultStatus::AwaitingPassword {
            keyfile: existing,
            error,
            ..
        } = &mut self.vault
        {
            *existing = keyfile;
            *error = None;
            cx.notify();
        }
    }

    pub fn pending_unlock_keyfile(&self) -> Option<PathBuf> {
        match &self.vault {
            VaultStatus::AwaitingPassword { keyfile, .. } => keyfile.clone(),
            _ => None,
        }
    }

    pub fn set_unlock_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        if let VaultStatus::AwaitingPassword { error, .. } = &mut self.vault {
            *error = Some(message.into());
            cx.notify();
        }
    }

    pub fn begin_open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.vault = VaultStatus::Opening { path };
        cx.notify();
    }

    pub fn finish_open_attempt(
        &mut self,
        path: PathBuf,
        result: Result<VaultDocument, String>,
        cx: &mut Context<Self>,
    ) {
        if !matches!(&self.vault, VaultStatus::Opening { path: active } if active == &path) {
            return;
        }

        self.vault = match result {
            Ok(document) => {
                let snapshot = document.snapshot();
                let selection = LibrarySelection::Group(snapshot.root.id.clone());
                let selected_entry_id =
                    snapshot.root.entries.first().map(|entry| entry.id.clone());
                let visible_entries =
                    Rc::new(entries_for_selection(snapshot, &selection, ""));
                let selected_strength = selected_entry_id
                    .as_deref()
                    .and_then(|id| document.strength_for_entry(id));

                VaultStatus::Open {
                    path,
                    document: Box::new(document),
                    selection,
                    selected_entry_id,
                    search_query: String::new(),
                    visible_entries,
                    selected_strength,
                }
            }
            Err(message) => VaultStatus::AwaitingPassword {
                path: path.clone(),
                keyfile: crate::keepass::KeePassRepository::suggested_keyfile(&path),
                error: Some(message),
            },
        };
        cx.notify();
    }

    pub fn fail_vault_selection(
        &mut self,
        path: Option<PathBuf>,
        message: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        self.vault = VaultStatus::Error {
            message: message.into(),
            path,
        };
        cx.notify();
    }

    pub fn lock_vault(&mut self, cx: &mut Context<Self>) {
        self.vault = VaultStatus::Empty;
        self.overlay = Overlay::None;
        self.save_status = SaveStatus::Idle;
        cx.notify();
    }

    pub fn save_status(&self) -> &SaveStatus {
        &self.save_status
    }

    /// Compute the live TOTP code for the currently-selected entry, if any.
    /// Recomputed on every render (cheap, ~µs); the per-second AppShell tick
    /// triggers `cx.notify` which causes the detail panel to re-call this.
    pub fn totp_for_selected_entry(&self) -> Option<OtpDisplay> {
        let VaultStatus::Open {
            document,
            selected_entry_id,
            ..
        } = &self.vault
        else {
            return None;
        };
        let id = selected_entry_id.as_deref()?;
        document.totp_for_entry(id)
    }

    /// Spawn an atomic save of the open vault on a background thread.
    ///
    /// Concurrency model: snapshots the live `Database` once on the foreground
    /// (cheap memcpy) and ships the clone + key material to a worker. The UI
    /// thread is free during the ~500 ms Argon2 KDF. If a save is already in
    /// flight we deliberately let the new one queue behind it — the latest
    /// state always wins, but we don't drop user changes.
    pub fn save_async(&mut self, cx: &mut Context<Self>) {
        let VaultStatus::Open { document, path, .. } = &self.vault else {
            return;
        };
        let payload = document.save_payload();
        let target = path.clone();

        self.save_status = SaveStatus::Saving;
        cx.notify();

        let task = cx.background_spawn(async move { payload.save_to(&target) });

        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| {
                state.save_status = match result {
                    Ok(()) => SaveStatus::Saved,
                    Err(error) => SaveStatus::Failed(error.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// Create an entry inside the given group, refresh the snapshot-derived
    /// caches, focus the new entry, and trigger a background save. Returns the
    /// new entry's id on success.
    pub fn create_entry(
        &mut self,
        group_id: &str,
        draft: EntryDraft,
        cx: &mut Context<Self>,
    ) -> Result<String, MutationError> {
        let new_id = {
            let VaultStatus::Open {
                document,
                selection,
                selected_entry_id,
                search_query,
                visible_entries,
                selected_strength,
                ..
            } = &mut self.vault
            else {
                return Err(MutationError::EntryNotFound);
            };

            let new_id = document.create_entry(group_id, &draft)?;

            // Snap the user to the entry's group so they can see what they
            // just created — otherwise creating from inside "Favorites" or a
            // tag filter would silently land the entry off-screen.
            *selection = LibrarySelection::Group(group_id.to_string());
            search_query.clear();

            let entries = entries_for_selection(document.snapshot(), selection, "");
            *selected_entry_id = Some(new_id.clone());
            *visible_entries = Rc::new(entries);
            *selected_strength = document.strength_for_entry(&new_id);

            new_id
        };
        cx.notify();
        self.save_async(cx);
        Ok(new_id)
    }

    pub fn update_entry(
        &mut self,
        entry_id: &str,
        draft: EntryDraft,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        {
            let VaultStatus::Open {
                document,
                selection,
                selected_entry_id,
                search_query,
                visible_entries,
                selected_strength,
                ..
            } = &mut self.vault
            else {
                return Err(MutationError::EntryNotFound);
            };

            document.update_entry(entry_id, &draft)?;

            *visible_entries =
                Rc::new(entries_for_selection(document.snapshot(), selection, search_query));
            // Re-score; the password may have changed.
            if selected_entry_id.as_deref() == Some(entry_id) {
                *selected_strength = document.strength_for_entry(entry_id);
            }
        }
        cx.notify();
        self.save_async(cx);
        Ok(())
    }

    /// Move an entry to the recycle bin (creating one if necessary). Selection
    /// jumps to the next visible entry so the detail pane stays populated.
    pub fn delete_entry(
        &mut self,
        entry_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        self.run_entry_mutation(cx, |doc| doc.delete_entry(entry_id), entry_id)
    }

    /// Permanent (unrecoverable) delete. Use only after a confirmation step in
    /// the UI — `save_async` flushes the result to disk and the entry is gone.
    pub fn delete_entry_permanent(
        &mut self,
        entry_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        self.run_entry_mutation(cx, |doc| doc.delete_entry_permanent(entry_id), entry_id)
    }

    /// Restore an entry from the recycle bin to the vault root.
    pub fn restore_entry(
        &mut self,
        entry_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        self.run_entry_mutation(cx, |doc| doc.restore_entry(entry_id), entry_id)
    }

    /// Shared post-mutation bookkeeping for delete / restore / permanent-delete:
    /// run the mutation, refresh the visible entry list, repoint the selection
    /// if the affected entry was selected, then schedule the autosave.
    fn run_entry_mutation<F>(
        &mut self,
        cx: &mut Context<Self>,
        mutate: F,
        entry_id: &str,
    ) -> Result<(), MutationError>
    where
        F: FnOnce(&mut VaultDocument) -> Result<(), MutationError>,
    {
        {
            let VaultStatus::Open {
                document,
                selection,
                selected_entry_id,
                search_query,
                visible_entries,
                selected_strength,
                ..
            } = &mut self.vault
            else {
                return Err(MutationError::EntryNotFound);
            };

            mutate(document)?;

            let entries = entries_for_selection(document.snapshot(), selection, search_query);
            if selected_entry_id.as_deref() == Some(entry_id) {
                *selected_entry_id = entries.first().map(|e| e.id.clone());
                *selected_strength = selected_entry_id
                    .as_deref()
                    .and_then(|id| document.strength_for_entry(id));
            }
            *visible_entries = Rc::new(entries);
        }
        cx.notify();
        self.save_async(cx);
        Ok(())
    }

    pub fn pending_unlock_path(&self) -> Option<PathBuf> {
        match &self.vault {
            VaultStatus::AwaitingPassword { path, .. } => Some(path.clone()),
            _ => None,
        }
    }

    pub fn unlock_prompt(&self) -> Option<UnlockPrompt> {
        match &self.vault {
            VaultStatus::AwaitingPassword {
                path,
                keyfile,
                error,
            } => Some(UnlockPrompt {
                path: path.clone(),
                file_name: file_name(path),
                display_path: path.display().to_string(),
                keyfile: keyfile.clone(),
                error: error.clone(),
            }),
            _ => None,
        }
    }

    pub fn select_group(&mut self, group_id: impl Into<String>, cx: &mut Context<Self>) {
        let group_id = group_id.into();

        let VaultStatus::Open {
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            ..
        } = &mut self.vault
        else {
            return;
        };

        let snapshot = document.snapshot();
        if snapshot.find_group(&group_id).is_none() {
            return;
        }

        *selection = LibrarySelection::Group(group_id);
        search_query.clear();
        let entries = entries_for_selection(snapshot, selection, "");
        *selected_entry_id = entries.first().map(|entry| entry.id.clone());
        *selected_strength = selected_entry_id
            .as_deref()
            .and_then(|id| document.strength_for_entry(id));
        *visible_entries = Rc::new(entries);
        cx.notify();
    }

    pub fn select_library(&mut self, sel: LibrarySelection, cx: &mut Context<Self>) {
        let VaultStatus::Open {
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            ..
        } = &mut self.vault
        else {
            return;
        };
        if *selection == sel {
            return;
        }
        *selection = sel;
        search_query.clear();
        let entries = entries_for_selection(document.snapshot(), selection, "");
        *selected_entry_id = entries.first().map(|entry| entry.id.clone());
        *selected_strength = selected_entry_id
            .as_deref()
            .and_then(|id| document.strength_for_entry(id));
        *visible_entries = Rc::new(entries);
        cx.notify();
    }

    pub fn select_entry(&mut self, entry_id: impl Into<String>, cx: &mut Context<Self>) {
        let entry_id = entry_id.into();

        let VaultStatus::Open {
            document,
            selected_entry_id,
            selected_strength,
            ..
        } = &mut self.vault
        else {
            return;
        };

        if document.snapshot().find_entry(&entry_id).is_some() {
            *selected_strength = document.strength_for_entry(&entry_id);
            *selected_entry_id = Some(entry_id);
            cx.notify();
        }
    }

    pub fn set_search_query(&mut self, query: impl Into<String>, cx: &mut Context<Self>) {
        let query = query.into();

        let VaultStatus::Open {
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            ..
        } = &mut self.vault
        else {
            return;
        };

        if *search_query == query {
            return;
        }

        *search_query = query;
        let entries = entries_for_selection(document.snapshot(), selection, search_query);
        let selected_entry_is_visible = selected_entry_id
            .as_deref()
            .is_some_and(|id| entries.iter().any(|entry| entry.id == id));

        if !selected_entry_is_visible {
            *selected_entry_id = entries.first().map(|entry| entry.id.clone());
            *selected_strength = selected_entry_id
                .as_deref()
                .and_then(|id| document.strength_for_entry(id));
        }

        *visible_entries = Rc::new(entries);
        cx.notify();
    }

    pub fn clear_search(&mut self, cx: &mut Context<Self>) {
        let VaultStatus::Open {
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            ..
        } = &mut self.vault
        else {
            return;
        };

        if search_query.is_empty() {
            return;
        }

        search_query.clear();
        let entries = entries_for_selection(document.snapshot(), selection, "");
        *selected_entry_id = entries.first().map(|entry| entry.id.clone());
        *selected_strength = selected_entry_id
            .as_deref()
            .and_then(|id| document.strength_for_entry(id));
        *visible_entries = Rc::new(entries);
        cx.notify();
    }

    pub fn copy_selected_value(&self, kind: CopyValueKind) -> Option<String> {
        let model = self.vault_browser()?;
        let entry = model.selected_entry?;

        match kind {
            CopyValueKind::Username => non_empty_copy(entry.username),
            CopyValueKind::Url => non_empty_copy(entry.url),
            CopyValueKind::Password => {
                let VaultStatus::Open { document, .. } = &self.vault else {
                    return None;
                };

                document.password_for_entry(&entry.id)
            }
        }
    }

    pub fn vault_browser(&self) -> Option<VaultBrowserModel> {
        let VaultStatus::Open {
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            ..
        } = &self.vault
        else {
            return None;
        };

        let snapshot = document.snapshot_rc();
        let showing_search_results = !search_query.trim().is_empty();

        let selected_entry = selected_entry_id
            .as_deref()
            .and_then(|id| visible_entries.iter().find(|entry| entry.id == id))
            .cloned()
            .or_else(|| visible_entries.first().cloned());

        let selection_label = selection_label_for(selection, &snapshot);

        Some(VaultBrowserModel {
            snapshot,
            selection: selection.clone(),
            selection_label,
            selected_entry_id: selected_entry.as_ref().map(|entry| entry.id.clone()),
            entries: Rc::clone(visible_entries),
            selected_entry,
            selected_strength: *selected_strength,
            search_query: search_query.clone(),
            showing_search_results,
        })
    }

    pub fn summary(&self) -> VaultSummary {
        match &self.vault {
            VaultStatus::Empty => VaultSummary {
                title: "No vault open".to_string(),
                subtitle: "Choose a KeePass database to begin.".to_string(),
                status: "Locked".to_string(),
                entries: 0,
                groups: 0,
                is_open: false,
                is_busy: false,
                provider: None,
                synced_at: None,
            },
            VaultStatus::AwaitingPassword { path, .. } => VaultSummary {
                title: file_name(path),
                subtitle: path.display().to_string(),
                status: "Password required".to_string(),
                entries: 0,
                groups: 0,
                is_open: false,
                is_busy: false,
                provider: Some("OneDrive"),
                synced_at: Some("2 minutes ago"),
            },
            VaultStatus::Opening { path } => VaultSummary {
                title: file_name(path),
                subtitle: "Decrypting database…".to_string(),
                status: "Opening".to_string(),
                entries: 0,
                groups: 0,
                is_open: false,
                is_busy: true,
                provider: Some("OneDrive"),
                synced_at: Some("2 minutes ago"),
            },
            VaultStatus::Open { path, document, .. } => VaultSummary {
                title: file_name(path),
                subtitle: path.display().to_string(),
                status: "Synced".to_string(),
                entries: document.snapshot().entry_count,
                groups: document.snapshot().group_count.saturating_sub(1),
                is_open: true,
                is_busy: false,
                provider: Some("OneDrive"),
                synced_at: Some("2 minutes ago"),
            },
            VaultStatus::Error { message, path } => VaultSummary {
                title: "Could not open vault".to_string(),
                subtitle: path
                    .as_ref()
                    .map_or_else(|| message.clone(), |path| path.display().to_string()),
                status: "Error".to_string(),
                entries: 0,
                groups: 0,
                is_open: false,
                is_busy: false,
                provider: None,
                synced_at: None,
            },
        }
    }
}

fn entries_for_selection(
    snapshot: &VaultSnapshot,
    selection: &LibrarySelection,
    search_query: &str,
) -> Vec<VaultEntry> {
    let query = search_query.trim().to_lowercase();

    if !query.is_empty() {
        return snapshot
            .entries_recursive()
            .into_iter()
            .filter(|entry| entry_matches_query(entry, &query))
            .cloned()
            .collect();
    }

    match selection {
        LibrarySelection::Group(id) => snapshot
            .find_group(id)
            .unwrap_or(&snapshot.root)
            .entries
            .clone(),
        LibrarySelection::AllItems => snapshot
            .entries_recursive()
            .into_iter()
            .cloned()
            .collect(),
        LibrarySelection::Favorites => {
            snapshot.entries_starred().into_iter().cloned().collect()
        }
        LibrarySelection::RecentlyUsed => {
            let mut entries: Vec<VaultEntry> =
                snapshot.entries_recursive().into_iter().cloned().collect();
            entries.sort_by(|a, b| b.updated.cmp(&a.updated));
            entries.truncate(50);
            entries
        }
        LibrarySelection::Trash => snapshot
            .recycle_bin_id
            .as_deref()
            .and_then(|bin_id| snapshot.find_group(bin_id))
            .map(|bin| bin.entries.clone())
            .unwrap_or_default(),
        LibrarySelection::Tag(name) => snapshot
            .entries_with_tag(name)
            .into_iter()
            .cloned()
            .collect(),
    }
}

fn selection_label_for(selection: &LibrarySelection, snapshot: &VaultSnapshot) -> String {
    match selection {
        LibrarySelection::Group(id) => snapshot
            .find_group(id)
            .map(|g| g.name.clone())
            .unwrap_or_else(|| snapshot.root.name.clone()),
        LibrarySelection::AllItems => "All items".to_string(),
        LibrarySelection::Favorites => "Favorites".to_string(),
        LibrarySelection::RecentlyUsed => "Recently used".to_string(),
        LibrarySelection::Trash => "Trash".to_string(),
        LibrarySelection::Tag(name) => format!("Tag · {name}"),
    }
}

fn entry_matches_query(entry: &VaultEntry, query: &str) -> bool {
    entry.title.to_lowercase().contains(query)
        || entry.username.to_lowercase().contains(query)
        || entry.url.to_lowercase().contains(query)
        || entry
            .tags
            .iter()
            .any(|tag| tag.to_lowercase().contains(query))
}

fn non_empty_copy(value: String) -> Option<String> {
    if value.is_empty() { None } else { Some(value) }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| path.display().to_string(), ToString::to_string)
}

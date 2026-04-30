use crate::domain::{VaultEntry, VaultSnapshot};
use crate::keepass::{EntryDraft, MutationError, OtpDisplay, StrengthReport, VaultDocument};
use crate::keepass::merge::{ConflictReport, Side};
use crate::sync::auth::{AccessToken, DeviceCodeChallenge};
use crate::sync::config::SyncConfig;
use crate::sync::graph::DriveItemHit;
use gpui::{AppContext as _, Context};
use keepass::db::Database;
use std::collections::HashMap;
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
    /// Cloud-sync binding for the currently-open vault. `Some` while a synced
    /// vault is open; `None` while in Welcome / unlocked-but-not-synced state.
    /// Holds the in-memory access token alongside the persisted SyncConfig.
    sync: Option<SyncBinding>,
    /// User-facing sync state. Drives the status pill, the SyncSettings card
    /// content, and whether the Conflict overlay opens.
    sync_status: SyncStatus,
    /// Active during the multi-step Connect overlay (provider pick → URL →
    /// device code → download). `None` when overlay isn't Connect.
    connect_flow: Option<ConnectFlow>,
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

/// Live sync binding for an open synced vault. Owns the access token in
/// memory; the refresh token lives in the keychain (loaded on demand by
/// `service::refresh_access_token`).
#[derive(Debug)]
pub struct SyncBinding {
    pub config: SyncConfig,
    pub access_token: AccessToken,
}

/// User-facing sync lifecycle. Mirrors the SaveStatus shape so the UI
/// status pill can read both with the same vocabulary.
#[derive(Clone, Debug, Default)]
pub enum SyncStatus {
    /// No sync configured for this vault, or no vault open.
    #[default]
    Disconnected,
    /// Synced, idle. Equivalent to "everything's good".
    Idle,
    /// Initial connect in progress (multi-step — see `ConnectFlow` for which step).
    Connecting,
    /// Push or pull in flight.
    Syncing,
    /// Last operation succeeded at the given time. `chrono::Local` for the
    /// "Synced 2 minutes ago" UI string.
    Synced { at: chrono::DateTime<chrono::Local> },
    /// Server returned 412 — local + remote diverged. UI opens the Conflict
    /// overlay; resolution clears this back to Synced.
    Conflict(Box<ConflictState>),
    /// Last operation failed. Caller (UI) decides whether to retry.
    Failed(String),
    /// Refresh token is gone or revoked — user must re-run Connect.
    Reconnect,
}

/// Heavy state owned by `SyncStatus::Conflict`. Holds both decrypted
/// databases, the report computed by `keepass::merge::diff`, the user's
/// per-entry picks, and the remote ETag we need to send back when uploading
/// the merged result.
///
/// Clone-ability is required because `SyncStatus` is `Clone` (the renderer
/// snapshots it). The two `Database` clones inside aren't free but they're
/// the same memcpy `save_payload` already does on every save — acceptable.
#[derive(Clone, Debug)]
pub struct ConflictState {
    pub local_db: Database,
    pub remote_db: Database,
    pub remote_etag: String,
    pub report: ConflictReport,
    pub picks: HashMap<String, Side>,
}

/// Step machine for the Connect overlay. The Connect screen renders a
/// stepper (Choose provider → Authorize → Pick vault) keyed off this.
#[derive(Clone, Debug)]
pub enum ConnectFlow {
    /// Initial: three provider buttons (only SharePoint is wired in this MVP).
    PickProvider,
    /// Device code shown; background task is polling for token. No file
    /// has been chosen yet — that comes after sign-in completes.
    SigningIn { challenge: DeviceCodeChallenge },
    /// Token in hand. Initial state shows a loading spinner while we fetch
    /// the user's `.kdbx` files; once `results` is populated the picker
    /// renders. `query` is the live filter the user types into the picker.
    Picking {
        token: AccessToken,
        results: Vec<DriveItemHit>,
        query: String,
        loading: bool,
        error: Option<String>,
    },
    /// User picked a file; downloading + persisting config.
    Downloading,
    /// Anything went wrong before we hit the unlock screen. Carries a
    /// human-readable message for the UI.
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
    /// Provider name from the active SyncBinding. `None` when the open vault
    /// is local-only.
    pub provider: Option<String>,
    /// Human-readable last-synced indicator (e.g. "just now", "2 minutes ago",
    /// "Failed", "Connecting…"). Derived from `SyncStatus`.
    pub synced_at: Option<String>,
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
        let was_connect = matches!(self.overlay, Overlay::Connect);
        self.overlay = Overlay::None;
        // Closing the Connect overlay also unwinds its flow state; otherwise
        // the next "Connect SharePoint" click would re-open into whichever
        // sub-step the user left it on (e.g., a stale "Failed" message).
        if was_connect {
            self.connect_flow = None;
            // Cancel any in-flight Connecting status; if it's still mid-poll
            // the polling loop will notice connect_flow is None and exit.
            if matches!(self.sync_status, SyncStatus::Connecting | SyncStatus::Failed(_)) {
                self.sync_status = match &self.sync {
                    Some(_) => SyncStatus::Idle,
                    None => SyncStatus::Disconnected,
                };
            }
        }
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

    pub fn sync_status(&self) -> &SyncStatus {
        &self.sync_status
    }

    pub fn sync_binding(&self) -> Option<&SyncBinding> {
        self.sync.as_ref()
    }

    pub fn connect_flow(&self) -> Option<&ConnectFlow> {
        self.connect_flow.as_ref()
    }

    /// Reset the Connect overlay to its initial step. Called when the user
    /// opens Connect from Welcome.
    pub fn begin_connect_flow(&mut self, cx: &mut Context<Self>) {
        self.connect_flow = Some(ConnectFlow::PickProvider);
        cx.notify();
    }

    /// Drop any in-progress Connect flow state. Called by Cancel + on
    /// successful completion.
    pub fn clear_connect_flow(&mut self, cx: &mut Context<Self>) {
        if self.connect_flow.is_some() {
            self.connect_flow = None;
            cx.notify();
        }
    }

    /// Replace the current connect flow step. Used by the Connect overlay's
    /// Back / provider-pick buttons.
    pub fn connect_flow_set(&mut self, flow: ConnectFlow, cx: &mut Context<Self>) {
        self.connect_flow = Some(flow);
        cx.notify();
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
                let succeeded = result.is_ok();
                state.save_status = match result {
                    Ok(()) => SaveStatus::Saved,
                    Err(error) => SaveStatus::Failed(error.to_string()),
                };
                cx.notify();
                // Chain into sync if we have a binding. `sync_now` is a
                // no-op when sync is None or when the vault isn't Open, so
                // it's safe to call unconditionally on success.
                if succeeded && state.sync.is_some() {
                    state.sync_now(cx);
                }
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
        let provider = self.sync.as_ref().map(|b| match b.config.provider {
            crate::sync::config::SyncProvider::SharePoint => "SharePoint".to_string(),
        });
        let synced_at = sync_status_label(&self.sync_status);

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
                provider: provider.clone(),
                synced_at: synced_at.clone(),
            },
            VaultStatus::Opening { path } => VaultSummary {
                title: file_name(path),
                subtitle: "Decrypting database…".to_string(),
                status: "Opening".to_string(),
                entries: 0,
                groups: 0,
                is_open: false,
                is_busy: true,
                provider: provider.clone(),
                synced_at: synced_at.clone(),
            },
            VaultStatus::Open { path, document, .. } => VaultSummary {
                title: file_name(path),
                subtitle: path.display().to_string(),
                status: "Synced".to_string(),
                entries: document.snapshot().entry_count,
                groups: document.snapshot().group_count.saturating_sub(1),
                is_open: true,
                is_busy: false,
                provider,
                synced_at,
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

    // ============== Sync actions ==============

    /// Tear down the current sync binding: drop the in-memory token, mark
    /// status as Disconnected, then in the background remove the keychain
    /// entry + sync-config file. UI updates immediately; cleanup is fire-
    /// and-forget (failures here just leave a stale config we'll happily
    /// overwrite next Connect).
    pub fn disconnect_sync(&mut self, cx: &mut Context<Self>) {
        let Some(binding) = self.sync.take() else {
            return;
        };
        self.sync_status = SyncStatus::Disconnected;
        cx.notify();
        cx.background_spawn(async move {
            let _ = crate::sync::service::disconnect(&binding.config);
        })
        .detach();
    }

    /// Drop the Connect overlay's transient state. Wired to the Cancel
    /// button + the Escape key.
    pub fn cancel_connect(&mut self, cx: &mut Context<Self>) {
        self.connect_flow = None;
        self.sync_status = match &self.sync {
            Some(_) => SyncStatus::Idle,
            None => SyncStatus::Disconnected,
        };
        cx.notify();
    }

    /// Step 1 of Connect: request a device code and kick off the polling
    /// loop. UI should observe `connect_flow` transitioning to
    /// `Some(SigningIn { .. })` and switch to the device-code screen.
    /// No URL/path is needed up front — the user picks a file *after*
    /// signing in (see `Picking`).
    pub fn start_sharepoint_connect(&mut self, cx: &mut Context<Self>) {
        self.sync_status = SyncStatus::Connecting;
        cx.notify();

        let task = cx.background_spawn(async move {
            crate::sync::service::request_device_code()
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| match result {
                Ok(challenge) => {
                    state.connect_flow = Some(ConnectFlow::SigningIn {
                        challenge: challenge.clone(),
                    });
                    cx.notify();
                    state.start_token_polling(challenge, cx);
                }
                Err(e) => {
                    let msg = e.to_string();
                    state.connect_flow = Some(ConnectFlow::Failed(msg.clone()));
                    state.sync_status = SyncStatus::Failed(msg);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Background polling loop. Runs until token received, code expired,
    /// auth declined, or the user cancels (we observe `connect_flow`
    /// transitioning out of `SigningIn` between iterations).
    fn start_token_polling(
        &mut self,
        challenge: DeviceCodeChallenge,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let mut interval = challenge.interval;
            loop {
                // Cooperative cancel: if the user closed Connect (or moved
                // past SigningIn for any other reason), stop polling.
                let still_signing_in = this
                    .update(cx, |s, _| {
                        matches!(s.connect_flow, Some(ConnectFlow::SigningIn { .. }))
                    })
                    .unwrap_or(false);
                if !still_signing_in {
                    return;
                }

                // Hard timeout: if the device-code expiry passed, give up.
                if std::time::SystemTime::now() > challenge.expires_at {
                    let _ = this.update(cx, |s, cx| {
                        let msg = "Device code expired before sign-in.".to_string();
                        s.connect_flow = Some(ConnectFlow::Failed(msg.clone()));
                        s.sync_status = SyncStatus::Failed(msg);
                        cx.notify();
                    });
                    return;
                }

                cx.background_executor().timer(interval).await;

                let challenge_clone = challenge.clone();
                let outcome = cx
                    .background_spawn(async move {
                        crate::sync::auth::poll_token(&challenge_clone)
                    })
                    .await;

                use crate::sync::auth::PollOutcome;
                match outcome {
                    PollOutcome::Pending => continue,
                    PollOutcome::SlowDown => {
                        // Server asked us to back off; double the interval as
                        // suggested by the OAuth device-code spec.
                        interval = interval.saturating_mul(2);
                        continue;
                    }
                    PollOutcome::Token(token) => {
                        let _ = this.update(cx, |s, cx| {
                            // Transition to picker (loading state); spawn the
                            // file-list fetch.
                            s.connect_flow = Some(ConnectFlow::Picking {
                                token: token.clone(),
                                results: Vec::new(),
                                query: String::new(),
                                loading: true,
                                error: None,
                            });
                            cx.notify();
                            s.start_kdbx_search(token, cx);
                        });
                        return;
                    }
                    PollOutcome::Failed(e) => {
                        let msg = e.to_string();
                        let _ = this.update(cx, |s, cx| {
                            s.connect_flow = Some(ConnectFlow::Failed(msg.clone()));
                            s.sync_status = SyncStatus::Failed(msg);
                            cx.notify();
                        });
                        return;
                    }
                }
            }
        })
        .detach();
    }

    /// Step 2 of Connect: fetch the user's `.kdbx` files. Cheap (one
    /// search call); results are filtered client-side as the user types.
    fn start_kdbx_search(&mut self, token: AccessToken, cx: &mut Context<Self>) {
        let token_for_task = token.clone();
        let task = cx.background_spawn(async move {
            crate::sync::service::list_kdbx_files(&token_for_task)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| {
                if let Some(ConnectFlow::Picking { results, loading, error, .. }) =
                    &mut state.connect_flow
                {
                    *loading = false;
                    match result {
                        Ok(hits) => {
                            *results = hits;
                            *error = None;
                        }
                        Err(e) => {
                            *error = Some(e.to_string());
                        }
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Live-filter the picker as the user types. Cheap — runs against the
    /// already-fetched list, no API calls.
    pub fn set_picker_query(&mut self, query: String, cx: &mut Context<Self>) {
        if let Some(ConnectFlow::Picking { query: q, .. }) = &mut self.connect_flow {
            *q = query;
            cx.notify();
        }
    }

    /// Step 3 of Connect: user picked one of the search results. Download
    /// the file, write it locally, persist SyncConfig + keychain token,
    /// then transition the vault into AwaitingPassword so the unlock flow
    /// takes over.
    pub fn pick_kdbx_file(
        &mut self,
        hit: DriveItemHit,
        local_path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        // The picker holds the access token; capture it before transitioning
        // out of Picking (which drops the token).
        let token = match &self.connect_flow {
            Some(ConnectFlow::Picking { token, .. }) => token.clone(),
            _ => return,
        };
        self.connect_flow = Some(ConnectFlow::Downloading);
        cx.notify();

        let path_for_task = local_path.clone();
        let task = cx.background_spawn(async move {
            let result = crate::sync::service::complete_connect_picked(
                &hit,
                token,
                &path_for_task,
            )?;
            // Write bytes to disk before returning so the unlock flow's
            // `KeePassRepository::open` finds them.
            std::fs::write(&path_for_task, &result.remote_bytes).map_err(|e| {
                crate::sync::service::ServiceError::Io {
                    path: path_for_task.clone(),
                    source: e,
                }
            })?;
            Ok::<_, crate::sync::service::ServiceError>(result)
        });
        let final_path = local_path;
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| match result {
                Ok(connect_result) => {
                    state.sync = Some(SyncBinding {
                        config: connect_result.config,
                        access_token: connect_result.access_token,
                    });
                    state.sync_status =
                        SyncStatus::Synced { at: chrono::Local::now() };
                    state.connect_flow = None;
                    state.overlay = Overlay::None;
                    state.request_password(final_path, cx);
                }
                Err(e) => {
                    let msg = e.to_string();
                    state.connect_flow = Some(ConnectFlow::Failed(msg.clone()));
                    state.sync_status = SyncStatus::Failed(msg);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Push the current local file to SharePoint. Used both as the chain
    /// after a local save (auto) and as the SyncSettings → Sync now button
    /// (manual). No-op when the vault is local-only.
    pub fn sync_now(&mut self, cx: &mut Context<Self>) {
        let Some(binding) = self.sync.as_ref() else {
            return;
        };
        let VaultStatus::Open { path, document, .. } = &self.vault else {
            return;
        };

        // Snapshot everything the background task needs. The master password
        // is captured up front because we need it later to decrypt remote
        // bytes if the upload returns 412.
        let config = binding.config.clone();
        let token = binding.access_token.clone();
        let local_path = path.clone();
        let master_password = document.password().to_string();

        self.sync_status = SyncStatus::Syncing;
        cx.notify();

        let task = cx.background_spawn(async move {
            let token = crate::sync::service::ensure_fresh(token, &config.account_email)?;
            let bytes = crate::sync::service::read_local(&local_path)?;
            let outcome =
                crate::sync::service::upload_after_save(&config, &token, &bytes)?;
            Ok::<_, crate::sync::service::ServiceError>((outcome, token))
        });

        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| match result {
                Ok((outcome, fresh_token)) => {
                    if let Some(b) = state.sync.as_mut() {
                        b.access_token = fresh_token;
                    }
                    use crate::sync::service::UploadAfterSave;
                    match outcome {
                        UploadAfterSave::Synced { new_etag, item: _ } => {
                            if let Some(b) = state.sync.as_mut() {
                                b.config.last_etag = new_etag;
                                // Persist updated etag — best effort; if the
                                // disk write fails we'll just re-detect a
                                // conflict next push (and re-resolve).
                                let _ = crate::sync::config::save(&b.config);
                            }
                            state.sync_status =
                                SyncStatus::Synced { at: chrono::Local::now() };
                            cx.notify();
                        }
                        UploadAfterSave::Conflict { remote_bytes, remote_etag } => {
                            state.handle_remote_conflict(
                                remote_bytes,
                                remote_etag,
                                master_password,
                                cx,
                            );
                        }
                    }
                }
                Err(e) => {
                    state.sync_status = match &e {
                        crate::sync::service::ServiceError::Auth(
                            crate::sync::auth::AuthError::InvalidGrant,
                        ) => SyncStatus::Reconnect,
                        _ => SyncStatus::Failed(e.to_string()),
                    };
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Decrypt remote bytes with the master password, build a `ConflictReport`
    /// against the in-memory local DB, and open the Conflict overlay.
    fn handle_remote_conflict(
        &mut self,
        remote_bytes: Vec<u8>,
        remote_etag: String,
        master_password: String,
        cx: &mut Context<Self>,
    ) {
        let VaultStatus::Open { document, .. } = &self.vault else {
            return;
        };
        let local_db = document.database().clone();

        match crate::keepass::KeePassRepository::open_bytes(
            &remote_bytes,
            &master_password,
            None,
        ) {
            Ok(remote_doc) => {
                let remote_db = remote_doc.database().clone();
                let report = crate::keepass::merge::diff(&local_db, &remote_db);
                let mut picks: HashMap<String, Side> = HashMap::new();
                for c in &report.conflicts {
                    // Prefill every conflict with Local (last writer wins —
                    // we just hit save here, so local was the user's intent).
                    picks.insert(c.id.clone(), Side::Local);
                }
                self.sync_status = SyncStatus::Conflict(Box::new(ConflictState {
                    local_db,
                    remote_db,
                    remote_etag,
                    report,
                    picks,
                }));
                self.overlay = Overlay::Conflict;
                cx.notify();
            }
            Err(_) => {
                // Master password mismatch on remote (or remote is corrupt).
                // Surface as a failure; user can manually resolve via
                // SyncSettings (force-overwrite isn't wired yet).
                self.sync_status = SyncStatus::Failed(
                    "Remote file uses a different master password — \
                     cannot merge automatically."
                        .to_string(),
                );
                cx.notify();
            }
        }
    }

    /// Mutate one user pick. Called by the Conflict overlay when the user
    /// clicks "Keep this" on either side. Idempotent.
    pub fn set_conflict_pick(&mut self, entry_id: &str, side: Side, cx: &mut Context<Self>) {
        let SyncStatus::Conflict(state) = &mut self.sync_status else {
            return;
        };
        state.picks.insert(entry_id.to_string(), side);
        cx.notify();
    }

    /// Finalise the conflict: build the merged DB from picks, save it
    /// locally, force-upload to SharePoint, dismiss the overlay.
    ///
    /// Concurrency note: we send `If-Match: conflict.remote_etag` so a
    /// third device that wrote during the user's resolution surfaces as a
    /// fresh 412 → re-decrypt → re-diff → re-prompt. That's safer than
    /// blind force-overwrite, at the cost of one extra round trip in the
    /// rare race case.
    pub fn apply_conflict_resolution(&mut self, cx: &mut Context<Self>) {
        let SyncStatus::Conflict(state) = &self.sync_status else {
            return;
        };
        let VaultStatus::Open { document, path, .. } = &self.vault else {
            return;
        };
        let Some(binding) = self.sync.as_ref() else {
            return;
        };

        let merged = crate::keepass::merge::apply_picks(
            &state.local_db,
            &state.remote_db,
            &state.picks,
            &state.report,
        );

        // Encrypt + save locally first. Re-uses the existing save path so
        // crash-safety semantics match a normal save.
        let payload = crate::keepass::SavePayload::for_merged(
            merged.clone(),
            document.password().to_string(),
            document.keyfile_path().map(std::path::Path::to_path_buf),
        );
        let local_path = path.clone();
        let config = binding.config.clone();
        let token = binding.access_token.clone();
        let if_match = state.remote_etag.clone();
        let master_password = document.password().to_string();

        self.sync_status = SyncStatus::Syncing;
        cx.notify();

        let task = cx.background_spawn(async move {
            payload.save_to(&local_path).map_err(|e| {
                crate::sync::service::ServiceError::Io {
                    path: local_path.clone(),
                    source: std::io::Error::other(e.to_string()),
                }
            })?;
            let token = crate::sync::service::ensure_fresh(token, &config.account_email)?;
            let bytes = crate::sync::service::read_local(&local_path)?;
            let outcome = crate::sync::graph::upload_content(
                &config.drive_id,
                &config.item_id,
                &bytes,
                Some(&if_match),
                &token,
            )?;
            Ok::<_, crate::sync::service::ServiceError>((
                outcome,
                token,
                local_path,
            ))
        });

        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| match result {
                Ok((outcome, fresh_token, local_path)) => {
                    if let Some(b) = state.sync.as_mut() {
                        b.access_token = fresh_token;
                    }
                    use crate::sync::graph::UploadOutcome;
                    match outcome {
                        UploadOutcome::Ok { new_etag, .. } => {
                            if let Some(b) = state.sync.as_mut() {
                                b.config.last_etag = new_etag;
                                let _ = crate::sync::config::save(&b.config);
                            }
                            // Reload the document from the merged file so the
                            // in-memory snapshot matches what's now on disk.
                            match crate::keepass::KeePassRepository::open_bytes(
                                &std::fs::read(&local_path).unwrap_or_default(),
                                &master_password,
                                None,
                            ) {
                                Ok(reloaded) => {
                                    if let VaultStatus::Open { document, .. } =
                                        &mut state.vault
                                    {
                                        *document = Box::new(reloaded);
                                    }
                                }
                                Err(_) => {
                                    // Shouldn't happen — we just wrote it.
                                    // Surface a warning and let the user
                                    // re-open manually.
                                }
                            }
                            state.sync_status =
                                SyncStatus::Synced { at: chrono::Local::now() };
                            state.overlay = Overlay::None;
                            cx.notify();
                        }
                        UploadOutcome::Conflict => {
                            // Third device wrote during resolution. Re-trigger
                            // the conflict flow against the freshly merged
                            // local + the new remote.
                            state.sync_status = SyncStatus::Syncing;
                            cx.notify();
                            state.sync_now(cx);
                        }
                    }
                }
                Err(e) => {
                    state.sync_status = SyncStatus::Failed(e.to_string());
                    cx.notify();
                }
            });
        })
        .detach();
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

/// Map `SyncStatus` to a short, user-facing string for the header / status pill.
/// `None` means "no sync indicator at all" — used when the vault is local-only.
fn sync_status_label(status: &SyncStatus) -> Option<String> {
    use chrono::Local;
    match status {
        SyncStatus::Disconnected => None,
        SyncStatus::Idle => Some("Synced".into()),
        SyncStatus::Connecting => Some("Connecting…".into()),
        SyncStatus::Syncing => Some("Syncing…".into()),
        SyncStatus::Synced { at } => Some(relative_time_label(*at, Local::now())),
        SyncStatus::Conflict(_) => Some("Conflict".into()),
        SyncStatus::Failed(_) => Some("Sync failed".into()),
        SyncStatus::Reconnect => Some("Sign-in expired".into()),
    }
}

/// "just now" / "N seconds ago" / "N minutes ago" / "N hours ago" — same
/// granularity as KeePass2's last-sync indicator. Past-only; future
/// timestamps clip to "just now".
fn relative_time_label(
    when: chrono::DateTime<chrono::Local>,
    now: chrono::DateTime<chrono::Local>,
) -> String {
    let secs = (now - when).num_seconds().max(0);
    if secs < 10 {
        "just now".into()
    } else if secs < 60 {
        format!("{secs} seconds ago")
    } else if secs < 3600 {
        let m = secs / 60;
        if m == 1 { "1 minute ago".into() } else { format!("{m} minutes ago") }
    } else if secs < 86_400 {
        let h = secs / 3600;
        if h == 1 { "1 hour ago".into() } else { format!("{h} hours ago") }
    } else {
        when.format("%Y-%m-%d %H:%M").to_string()
    }
}

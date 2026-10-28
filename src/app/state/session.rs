//! Constructors, plain accessors, overlays and the pending notice, and
//! switching between unlocked vaults.

use super::*;

impl AppState {
    pub(crate) fn auto_type_cancellation(&self) -> CancellationToken {
        self.auto_type_cancellation.clone()
    }

    pub(super) fn rotate_auto_type_context(&mut self) {
        self.auto_type_cancellation.cancel();
        self.auto_type_cancellation = CancellationToken::new();
    }

    /// Construct an AppState that auto-resumes the most recently opened
    /// vault. Reads the recents file synchronously (a few hundred bytes
    /// of JSON - cheap), prunes entries whose file no longer exists, and
    /// pre-populates the unlock screen with the head of the list.
    ///
    /// Falls back to an empty AppState (Welcome screen) when the list is
    /// empty or the file isn't readable.
    pub fn with_resume() -> Self {
        let recents = recents::load_pruned();
        let initial_vault = recents
            .entries
            .first()
            .map(|entry| VaultStatus::AwaitingPassword {
                path: entry.path.clone(),
                keyfile: crate::keepass::KeePassRepository::suggested_keyfile(&entry.path),
                error: None,
            })
            .unwrap_or_default();

        let pending_whats_new = crate::update::load_whats_new_for_version(crate::app::APP_VERSION);
        let whats_new_info = pending_whats_new
            .as_ref()
            .map(|pending| pending.info.clone());
        let overlay = pending_whats_new
            .filter(|pending| !pending.auto_shown)
            .map(|pending| Overlay::WhatsNew { info: pending.info })
            .unwrap_or_default();

        let mut state = Self::default();
        state.vault = initial_vault;
        state.overlay = overlay;
        state.recents = recents.entries;
        state.whats_new_info = whats_new_info;
        state.biometric = crate::biometric::default_store();
        state.biometric_registry = crate::biometric::registry::load_or_default();
        state
    }

    /// Test-friendly constructor: same field layout as `with_resume`
    /// but takes the biometric store + registry explicitly so unit
    /// tests can inject `InMemoryBiometricStore`. Skips the recents
    /// auto-resume so tests start from a clean Welcome.
    #[cfg(test)]
    pub(crate) fn with_biometric(
        biometric: Arc<dyn BiometricStore>,
        biometric_registry: BiometricRegistry,
    ) -> Self {
        let mut state = Self::default();
        state.biometric = biometric;
        state.biometric_registry = biometric_registry;
        state
    }

    /// Put `document` in the active slot as a freshly unlocked session,
    /// without a file or a KDF. Whatever was active is replaced, not parked;
    /// call `park_active` first to keep it.
    #[cfg(test)]
    pub(crate) fn open_for_test(&mut self, path: PathBuf, document: VaultDocument) {
        self.vault = VaultStatus::Open {
            path,
            document: Box::new(document),
            selection: LibrarySelection::AllItems,
            selected_entry_id: None,
            search_query: String::new(),
            visible_entries: Rc::new(Vec::new()),
            selected_strength: None,
            last_used: HashMap::new(),
        };
        self.active_vault_session_id = Some(VaultSessionId::next());
    }

    /// Production wiring used by `app::run` after the AppState entity
    /// has already been constructed (e.g. when we want to load the
    /// registry off the UI thread). Not used today; included so
    /// the indirection point is documented and future code can swap
    /// the store without re-running `with_resume`.
    pub fn install_biometric(
        &mut self,
        biometric: Arc<dyn BiometricStore>,
        biometric_registry: BiometricRegistry,
    ) {
        self.biometric = biometric;
        self.biometric_registry = biometric_registry;
    }

    pub fn vault_status(&self) -> &VaultStatus {
        &self.vault
    }

    /// Recently opened vaults, most recent first. Drives the Welcome
    /// screen's "Recent" section.
    pub fn recents(&self) -> &[RecentEntry] {
        &self.recents
    }

    /// Forget a vault from the Welcome screen's recent list and persist the
    /// updated list. This does not delete the vault or any related keychain or
    /// sync configuration.
    pub fn remove_recent(&mut self, path: &Path, cx: &mut Context<Self>) -> bool {
        if !recents::remove_path_in(&mut self.recents, path) {
            return false;
        }

        let snapshot = recents::RecentVaults {
            entries: self.recents.clone(),
        };
        cx.background_spawn(async move {
            let _ = recents::save(&snapshot);
        })
        .detach();
        cx.notify();
        true
    }

    pub fn favicon_status(&self) -> &FaviconDownloadStatus {
        &self.favicon_status
    }

    pub fn overlay(&self) -> &Overlay {
        &self.overlay
    }

    /// Take the message a background path left for the user, if any.
    ///
    /// Some failures happen where no window is in hand: a rejected file pick
    /// while a vault is open, or a keychain entry that would not be removed on
    /// disconnect. They used to be dropped, so the file picker closed and
    /// nothing happened at all. The shell drains this in its observer and
    /// shows it once.
    pub fn take_pending_notice(&mut self) -> Option<String> {
        self.vault_selection_error.take()
    }

    /// Leave a message for the shell to show. Overwrites any earlier one:
    /// the newest failure is the one the user is looking at the result of.
    pub(super) fn leave_pending_notice(&mut self, message: impl Into<String>) {
        self.vault_selection_error = Some(message.into());
    }

    pub fn open_overlay(&mut self, overlay: Overlay, cx: &mut Context<Self>) {
        if matches!(self.vault, VaultStatus::LockedPendingSave)
            && !matches!(
                overlay,
                Overlay::None | Overlay::Settings | Overlay::WhatsNew { .. } | Overlay::About
            )
        {
            return;
        }
        if self.overlay == overlay {
            return;
        }
        // Switching directly between overlays (e.g. ⌘O while Connect is
        // mid-`SigningIn`) has to run the same teardown as
        // `close_overlay` - otherwise the device-code polling loop would
        // outlive the overlay and keep mutating `connect_flow` /
        // `sync_status` behind a screen the user has already moved on
        // from.
        let leaving_connect = matches!(self.overlay, Overlay::Connect);
        self.overlay = overlay;
        if leaving_connect {
            self.unwind_connect_flow();
        }
        cx.notify();
    }

    pub fn close_overlay(&mut self, cx: &mut Context<Self>) -> bool {
        if matches!(self.overlay, Overlay::None) {
            return false;
        }
        let leaving_connect = matches!(self.overlay, Overlay::Connect);
        let leaving_whats_new = matches!(self.overlay, Overlay::WhatsNew { .. });
        self.overlay = Overlay::None;
        if leaving_connect {
            self.unwind_connect_flow();
        }
        if leaving_whats_new {
            let _ = crate::update::mark_whats_new_auto_shown(crate::app::APP_VERSION);
        }
        cx.notify();
        true
    }

    /// Drop any in-flight Connect state when the Connect overlay is left,
    /// regardless of whether we're closing to None or jumping to a
    /// different overlay. The device-code polling loop watches
    /// `connect_flow` and exits when it observes `None`; the sync status
    /// reset clears whichever transient `Connecting` / `Failed` pill the
    /// flow had pushed. Without this the next "Connect SharePoint" click
    /// would also re-open into whichever sub-step the user left it on
    /// (e.g. a stale "Failed" message).
    fn unwind_connect_flow(&mut self) {
        self.connect_operations.advance();
        self.connect_flow = None;
        // A reconnect that's abandoned (Cancel / Esc / overlay switch) must
        // not leave its target armed - otherwise the next plain Connect's
        // token would rebind this vault instead of opening the picker.
        self.reconnect_target = None;
        if matches!(
            self.sync_status,
            SyncStatus::Connecting | SyncStatus::Failed(_)
        ) {
            self.sync_status = match &self.sync {
                Some(_) => SyncStatus::Idle,
                None => SyncStatus::Disconnected,
            };
        }
    }

    pub fn request_password(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if matches!(self.vault, VaultStatus::LockedPendingSave) {
            return;
        }
        // Never create a second decrypted session for the same file. Two
        // divergent documents cannot be serialized safely through the
        // path-keyed save coordinator without arbitrarily overwriting one.
        if self.switch_to_unlocked(&path, cx) {
            return;
        }
        self.rotate_auto_type_context();
        self.vault_selection_error = None;
        self.clear_pending_sync_unless(&path);
        // Already looking at an unlocked vault? Park it so the user can
        // come back without re-typing their master password. The
        // cold-unlock screen we're about to render points at a *different*
        // path; the previously-active vault stays in `parked` until the
        // user switches to it again (or hits Esc, which rehydrates the
        // most-recently parked one).
        self.park_active();
        let keyfile = crate::keepass::KeePassRepository::suggested_keyfile(&path);
        self.vault = VaultStatus::AwaitingPassword {
            path,
            keyfile,
            error: None,
        };
        self.overlay = Overlay::None;
        // Every transition into AwaitingPassword starts a fresh Touch ID
        // story - drop stale UI state from a previous vault's attempt.
        self.clear_biometric_attempt();
        self.pending_biometric_enrollment = false;
        cx.notify();
    }

    /// Swap to an already-unlocked vault if we know one for `path`. Returns
    /// `true` when the swap happened (caller can skip the unlock prompt),
    /// `false` when `path` is cold and needs a password.
    ///
    /// A no-op (returning `true`) when `path` is already the active vault -
    /// the caller hasn't told us they wanted to do anything, so we don't
    /// disturb selection / search state.
    pub fn switch_to_unlocked(&mut self, path: &Path, cx: &mut Context<Self>) -> bool {
        self.vault_selection_error = None;
        if let VaultStatus::Open { path: active, .. } = &self.vault
            && active.as_path() == path
        {
            // Same vault - nothing to do. Still count as "handled" so the
            // caller doesn't fall through to the password prompt.
            return true;
        }
        if !self.parked.contains_key(path) {
            return false;
        }
        self.rotate_auto_type_context();
        // Park whatever's currently active (Open or AwaitingPassword) so
        // it survives the switch.
        self.park_active();
        if !self.unpark(path) {
            // Defensive: park_active above could in theory race away the
            // map entry (it can't - we just checked). Treat as cold.
            return false;
        }
        // Front-rank in recents so Welcome / ⌘O reflect the switch.
        self.push_recent(path.to_path_buf(), cx);
        cx.notify();
        true
    }

    /// Bring the most-recently-parked vault back into the active slot.
    /// Used by Esc-on-unlock to undo a request_password that the user
    /// decided against. Returns `true` when something was rehydrated.
    pub fn rehydrate_most_recent_park(&mut self, cx: &mut Context<Self>) -> bool {
        self.pending_sync = None;
        let Some(path) = self.parked_order.last().cloned() else {
            return false;
        };
        self.rotate_auto_type_context();
        // We're abandoning the AwaitingPassword screen - drop it without
        // parking (no decrypted state to preserve).
        self.vault = VaultStatus::Empty;
        self.save_status = SaveStatus::Idle;
        self.sync = None;
        self.sync_status = SyncStatus::Disconnected;
        if !self.unpark(&path) {
            return false;
        }
        cx.notify();
        true
    }

    /// Paths of every currently-unlocked vault, *including* the active
    /// one. Vault Switcher UI uses this to render the "Open" section.
    /// Order: parked entries newest-last, active last. Callers that need
    /// the active marker should consult `current_vault_path` separately.
    pub fn unlocked_paths(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = self.parked_order.clone();
        if let VaultStatus::Open { path, .. } = &self.vault
            && !out.iter().any(|p| p == path)
        {
            out.push(path.clone());
        }
        out
    }

    /// `true` whenever any vault is unlocked in memory (active or parked).
    /// Drives the global auto-lock task gate: the idle timer must keep
    /// ticking even when the active slot is `Empty`/`AwaitingPassword` but
    /// parked vaults are still decrypted in memory.
    pub fn has_any_unlocked(&self) -> bool {
        matches!(self.vault, VaultStatus::Open { .. }) || !self.parked.is_empty()
    }
}

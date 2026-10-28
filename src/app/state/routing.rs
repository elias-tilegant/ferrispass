//! Vault sessions and where results go: session ids, parking and
//! unparking, and routing save and sync results to the session that
//! started them.

use super::*;

impl AppState {
    /// Resolve a path to the exact unlocked session that currently owns it.
    /// The active slot wins if the same path also exists in `parked` during a
    /// close/reopen transition.
    fn vault_session_id_for(&self, target: &Path) -> Option<VaultSessionId> {
        if self.is_active_vault(target) {
            return self.active_vault_session_id;
        }
        self.parked.get(target).map(|session| session.session_id)
    }

    /// Forget the relationship this vault had, in memory.
    ///
    /// The disk side of a disconnect runs on a background task; this is the
    /// part that matters for anything still in flight, and the part a test
    /// can drive. What is in flight belongs to the relationship that just
    /// ended, whatever the user binds this vault to next.
    pub(super) fn end_sync_relationship(&mut self, target: Option<&Path>) {
        self.connect_flow = None;
        self.reconnect_target = None;
        self.sync = None;
        if let Some(target) = target {
            self.deferred_conflicts.remove(target);
            self.remote_kdf_confirmations.remove(target);
            self.remote_merges_in_flight.remove(target);
        }
        if let Some(target) = target {
            let generation = self
                .sync_binding_generations
                .entry(target.to_path_buf())
                .or_insert(0);
            *generation = generation.wrapping_add(1);
        }
        self.sync_status = SyncStatus::Disconnected;
    }

    pub(super) fn sync_binding_generation_for(&self, target: &Path) -> u64 {
        self.sync_binding_generations
            .get(target)
            .copied()
            .unwrap_or(0)
    }

    /// The sync relationship a callback started now belongs to. `None` when
    /// the vault is not open here at all.
    pub(super) fn sync_session_for(&self, target: &Path) -> Option<SyncSession> {
        self.vault_session_id_for(target)
            .or_else(|| self.parked.get(target).map(|session| session.session_id))
            .map(|vault| SyncSession {
                vault,
                binding: self.sync_binding_generation_for(target),
            })
    }

    /// Whether that relationship is still the one this vault has.
    pub(super) fn sync_session_is_current(&self, target: &Path, session: SyncSession) -> bool {
        session.binding == self.sync_binding_generation_for(target)
            && self.vault_session_is_current(target, session.vault)
    }

    pub(super) fn vault_session_is_current(
        &self,
        target: &Path,
        session_id: VaultSessionId,
    ) -> bool {
        self.vault_session_id_for(target) == Some(session_id)
            || self
                .parked
                .get(target)
                .is_some_and(|session| session.session_id == session_id)
    }

    /// Whether `target` is the vault currently on screen. Distinct from
    /// [`Self::vault_session_is_current`], which also accepts a parked vault:
    /// window-scoped state (overlay, connect flow) may only be touched for
    /// the active one.
    pub(super) fn is_active_vault(&self, target: &Path) -> bool {
        matches!(&self.vault, VaultStatus::Open { path, .. } if path.as_path() == target)
    }

    /// Save-only ownership check. Unlike `vault_session_is_current`, this also
    /// admits quarantined sessions after user access has already been revoked.
    pub(super) fn save_session_is_retained(
        &self,
        target: &Path,
        session_id: VaultSessionId,
    ) -> bool {
        self.vault_session_is_current(target, session_id)
            || self
                .deferred_lock_sessions
                .get(&session_id)
                .is_some_and(|session| session.path == target)
    }

    pub(super) fn path_for_vault_session(&self, session_id: VaultSessionId) -> Option<PathBuf> {
        if self.active_vault_session_id == Some(session_id)
            && let VaultStatus::Open { path, .. } = &self.vault
        {
            return Some(path.clone());
        }
        self.parked
            .iter()
            .find_map(|(path, session)| (session.session_id == session_id).then(|| path.clone()))
            .or_else(|| {
                self.deferred_lock_sessions
                    .get(&session_id)
                    .map(|session| session.path.clone())
            })
    }

    pub(super) fn mark_vault_session_unpersisted(&mut self, session_id: VaultSessionId) {
        if self.unpersisted_vault_sessions.insert(session_id) {
            // AcqRel, matching the decrement below: this counter gates
            // whether quitting can discard a save, so both edges need the
            // same ordering.
            UNPERSISTED_VAULT_SESSION_COUNT.fetch_add(1, Ordering::AcqRel);
        }
    }

    pub(super) fn mark_vault_session_persisted(&mut self, session_id: VaultSessionId) {
        if self.unpersisted_vault_sessions.remove(&session_id) {
            UNPERSISTED_VAULT_SESSION_COUNT.fetch_sub(1, Ordering::AcqRel);
        }
        self.deferred_lock_sessions.remove(&session_id);
        self.refresh_locked_pending_save_status();
    }

    pub(super) fn has_unpersisted_save_work(&self) -> bool {
        !self.unpersisted_vault_sessions.is_empty() || !self.saves_in_flight.is_empty()
    }

    pub(super) fn save_queue_contains_session(&self, session_id: VaultSessionId) -> bool {
        self.saves_in_flight
            .values()
            .any(|queued| queued.owner == session_id || queued.pending_session == Some(session_id))
    }

    /// Move the currently-active `Open` vault into the parked map, taking
    /// its save lifecycle + sync binding with it. No-op on every other
    /// `VaultStatus` (Empty/AwaitingPassword/Opening/Error carry no
    /// decrypted state worth preserving).
    pub(super) fn park_active(&mut self) {
        if !matches!(self.vault, VaultStatus::Open { .. }) {
            return;
        }
        let session_id = self
            .active_vault_session_id
            .take()
            .expect("open vault must have a session id");
        let prev = std::mem::take(&mut self.vault);
        let VaultStatus::Open {
            path,
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            last_used,
        } = prev
        else {
            // Unreachable - guard above already established Open. Restoring
            // to Empty is the safe fallthrough if a future variant slips in.
            return;
        };
        let session = ParkedSession {
            session_id,
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            last_used,
            save_status: std::mem::take(&mut self.save_status),
            sync: self.sync.take(),
            sync_status: std::mem::take(&mut self.sync_status),
            sync_history: std::mem::take(&mut self.sync_history),
        };
        // Refresh order - if this vault was parked before (shouldn't be,
        // but defend), move it to the tail.
        self.parked_order.retain(|p| p != &path);
        self.parked_order.push(path.clone());
        self.parked.insert(path, session);
    }

    /// Hydrate a parked session back into the active `Open` variant.
    /// Returns `false` when `path` isn't in the parked map. Caller is
    /// expected to have parked whatever was active first.
    pub(super) fn unpark(&mut self, path: &Path) -> bool {
        let Some(session) = self.parked.remove(path) else {
            return false;
        };
        self.parked_order.retain(|p| p != path);
        let ParkedSession {
            session_id,
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            last_used,
            save_status,
            sync,
            sync_status,
            sync_history,
        } = session;
        self.vault = VaultStatus::Open {
            path: path.to_path_buf(),
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            last_used,
        };
        self.active_vault_session_id = Some(session_id);
        self.save_status = save_status;
        self.sync = sync;
        self.sync_status = sync_status;
        self.sync_history = sync_history;
        true
    }

    /// Set save state only on the session that dispatched the async write.
    /// Returns whether the active slot changed and therefore needs a redraw.
    pub(super) fn set_save_status_for_session(
        &mut self,
        target: &Path,
        session_id: VaultSessionId,
        status: SaveStatus,
    ) -> bool {
        if self.active_vault_session_id == Some(session_id) && self.is_active_vault(target) {
            self.save_status = status;
            true
        } else if let Some(parked) = self
            .parked
            .get_mut(target)
            .filter(|parked| parked.session_id == session_id)
        {
            parked.save_status = status;
            false
        } else if let Some(session) = self
            .deferred_lock_sessions
            .get_mut(&session_id)
            .filter(|session| session.path == target)
        {
            session.save_status = status;
            self.refresh_locked_pending_save_status();
            true
        } else {
            false
        }
    }

    /// Apply a save result against the exact vault session that started it.
    /// Path equality alone is insufficient because a user can reopen the same
    /// file before a background callback lands.
    pub(super) fn apply_save_status_for_session(
        &mut self,
        target: &Path,
        session_id: VaultSessionId,
        status: SaveStatus,
        cx: &mut Context<Self>,
    ) {
        let notify = self.set_save_status_for_session(target, session_id, status);
        self.publish_stalled_save();
        if notify {
            cx.notify();
        }
    }

    /// Republish whether a save is stuck rather than running, so the app-global
    /// quit handler can tell the two apart without borrowing this state.
    pub(super) fn publish_stalled_save(&self) {
        let stalled = self
            .failed_save_paths()
            .next()
            .map(file_name)
            .filter(|_| self.saves_in_flight.is_empty());
        if let Ok(mut slot) = STALLED_SAVE_VAULT.lock() {
            *slot = stalled;
        }
    }

    /// Every retained session whose latest save failed.
    fn failed_save_paths(&self) -> impl Iterator<Item = &Path> {
        let active = matches!(self.save_status, SaveStatus::Failed(_))
            .then(|| match &self.vault {
                VaultStatus::Open { path, .. } => Some(path.as_path()),
                _ => None,
            })
            .flatten();
        let parked = self
            .parked
            .iter()
            .filter(|(_, session)| matches!(session.save_status, SaveStatus::Failed(_)))
            .map(|(path, _)| path.as_path());
        let deferred = self
            .deferred_lock_sessions
            .values()
            .filter(|session| matches!(session.save_status, SaveStatus::Failed(_)))
            .map(|session| session.path.as_path());
        active.into_iter().chain(parked).chain(deferred)
    }

    /// Mirror of `apply_save_status` for the cloud-sync lifecycle. Notifies
    /// only when the active vault was touched.
    pub(super) fn apply_sync_status(
        &mut self,
        target: &Path,
        status: SyncStatus,
        cx: &mut Context<Self>,
    ) {
        if self.route_sync_status(target, status) {
            cx.notify();
        }
    }

    /// Writes `status` to whichever slot holds `target`. Returns true when
    /// that was the active vault and the window therefore needs a repaint.
    /// Split out from [`Self::apply_sync_status`] so the routing itself is
    /// reachable without a gpui context.
    pub(super) fn route_sync_status(&mut self, target: &Path, status: SyncStatus) -> bool {
        // A healthy outcome means the provider let us through, so any throttle
        // window is over. Doing this here rather than at the five success
        // sites is the only way it cannot be forgotten at one of them.
        if matches!(status, SyncStatus::Synced { .. } | SyncStatus::Idle) {
            self.clear_sync_backoff(target);
        }
        // A cost question belongs to the remote it was asked about. Once the
        // sync is healthy again or the vault is no longer synced, it is
        // stale, and allowing a stale cost must not be on offer.
        // A new attempt (`Syncing`) fetches the remote again, so the notice
        // would otherwise offer a cost from a revision that is being replaced.
        if matches!(
            status,
            SyncStatus::Synced { .. }
                | SyncStatus::Idle
                | SyncStatus::Disconnected
                | SyncStatus::Syncing
        ) {
            self.remote_kdf_confirmations.remove(target);
        }
        // Same for a merge still decrypting: a new attempt supersedes it, and
        // its old remote must not land in the middle of the new one.
        // `handle_remote_conflict_for` sets its token after the `Syncing`
        // that precedes it, so its own merge is unaffected.
        if matches!(
            status,
            SyncStatus::Synced { .. }
                | SyncStatus::Idle
                | SyncStatus::Disconnected
                | SyncStatus::Syncing
        ) {
            self.remote_merges_in_flight.remove(target);
        }
        // Same reasoning, same place: anything that is not the deferred
        // conflict itself ends the wait, including the overlay finally
        // opening. Doing it here is the only way it cannot be forgotten at
        // one of the sites that set a status.
        if !matches!(status, SyncStatus::Failed(_)) {
            self.deferred_conflicts.remove(target);
        }
        if self.is_active_vault(target) {
            self.sync_status = status;
            true
        } else {
            if let Some(parked) = self.parked.get_mut(target) {
                parked.sync_status = status;
            }
            false
        }
    }

    /// `route_sync_status` with the session guard, for completion bodies
    /// that must stay reachable without a gpui context. Returns true when
    /// the active vault was touched and the window needs a repaint.
    pub(super) fn route_sync_status_for_session(
        &mut self,
        target: &Path,
        session: SyncSession,
        status: SyncStatus,
    ) -> bool {
        self.sync_session_is_current(target, session) && self.route_sync_status(target, status)
    }

    /// Session-aware counterpart used by async callbacks. A path can be
    /// locked and reopened while a request is in flight; only the exact
    /// session that dispatched it may consume the result.
    pub(super) fn apply_sync_status_for_session(
        &mut self,
        target: &Path,
        session: SyncSession,
        status: SyncStatus,
        cx: &mut Context<Self>,
    ) {
        if self.sync_session_is_current(target, session) {
            self.apply_sync_status(target, status, cx);
        }
    }

    /// Mutate the `SyncBinding` for the vault at `target` if one exists.
    /// Used by sync callbacks to write back a refreshed access token /
    /// updated ETag against the vault that actually issued the upload,
    /// even after the user has switched away.
    pub(super) fn with_sync_binding_mut_for(
        &mut self,
        target: &Path,
        f: impl FnOnce(&mut SyncBinding),
    ) {
        if self.is_active_vault(target) {
            if let Some(b) = self.sync.as_mut() {
                f(b);
            }
        } else if let Some(parked) = self.parked.get_mut(target)
            && let Some(b) = parked.sync.as_mut()
        {
            f(b);
        }
    }

    pub(super) fn with_sync_binding_mut_for_session(
        &mut self,
        target: &Path,
        session: SyncSession,
        f: impl FnOnce(&mut SyncBinding),
    ) {
        if self.sync_session_is_current(target, session) {
            self.with_sync_binding_mut_for(target, f);
        }
    }

    /// The decrypted document of the vault at `target`, but only while that
    /// exact session still owns it. Parked vaults included: a long-running
    /// background mutation keeps writing to the vault that started it, not to
    /// whichever one the user switched to.
    pub(super) fn document_mut_for_session(
        &mut self,
        target: &Path,
        session_id: VaultSessionId,
    ) -> Option<&mut VaultDocument> {
        if let VaultStatus::Open { document, path, .. } = &mut self.vault
            && path.as_path() == target
            && self.active_vault_session_id == Some(session_id)
        {
            return Some(document);
        }
        self.parked
            .get_mut(target)
            .filter(|session| session.session_id == session_id)
            .map(|session| session.document.as_mut())
    }

    /// Install a fresh `SyncBinding` for the vault at `target`, replacing
    /// any existing one (or filling an empty slot left by a failed restore).
    /// Routes to the active vault or the matching parked session. Returns
    /// `true` when a slot was found - `false` means the vault was locked /
    /// closed between dispatch and callback (the on-disk config + keychain
    /// are already updated, so the next open restores cleanly). Used by the
    /// reconnect rebind, which must *set* a binding rather than mutate one
    /// in place (`with_sync_binding_mut_for` can't, since after an expired
    /// restore `self.sync` is `None`).
    fn rebind_sync(&mut self, target: &Path, binding: SyncBinding) -> bool {
        if self.is_active_vault(target) {
            self.sync = Some(binding);
            true
        } else if let Some(parked) = self.parked.get_mut(target) {
            parked.sync = Some(binding);
            true
        } else {
            false
        }
    }

    pub(super) fn rebind_sync_for_session(
        &mut self,
        target: &Path,
        session: SyncSession,
        binding: SyncBinding,
    ) -> bool {
        self.sync_session_is_current(target, session) && self.rebind_sync(target, binding)
    }

    /// What the auto-sync tick needs to know about a vault before spending a
    /// round-trip on it. Returns a `Copy` summary rather than a clone of
    /// `SyncStatus`, whose `Conflict` variant owns two decrypted databases:
    /// the tick asked for it once per vault per tick and paid a full deep
    /// copy of both, leaving the cleartext behind for the allocator to reuse.
    pub(super) fn sync_activity_for(&self, target: &Path) -> Option<SyncActivity> {
        let status = if self.is_active_vault(target) {
            &self.sync_status
        } else {
            &self.parked.get(target)?.sync_status
        };
        Some(SyncActivity::of(status))
    }

    /// `true` when at least one synced vault is in memory (active or
    /// parked). Gates the AppShell auto-sync timer so it only ticks when
    /// there's actually a binding to pull against / keep alive.
    pub fn has_any_sync_binding(&self) -> bool {
        self.sync.is_some() || self.parked.values().any(|s| s.sync.is_some())
    }

    /// `true` when the vault at `target` still has a live sync binding -
    /// active or parked. Used to drop a background sync result whose vault
    /// was disconnected (or never synced) while the request was in flight.
    pub(super) fn has_sync_binding_for(&self, target: &Path) -> bool {
        if self.is_active_vault(target) {
            return self.sync.is_some();
        }
        self.parked.get(target).is_some_and(|p| p.sync.is_some())
    }

    pub(super) fn is_unlocked_path(&self, target: &Path) -> bool {
        self.is_active_vault(target) || self.parked.contains_key(target)
    }

    pub(super) fn clear_pending_sync_unless(&mut self, target: &Path) {
        if self
            .pending_sync
            .as_ref()
            .is_some_and(|pending| pending.local_path != target)
        {
            self.pending_sync = None;
        }
    }

    pub(super) fn install_pending_sync_for(&mut self, opened: &Path) -> bool {
        if !self
            .pending_sync
            .as_ref()
            .is_some_and(|pending| pending.local_path == opened)
        {
            return false;
        }
        let pending = self.pending_sync.take().expect("checked above");
        self.sync = Some(pending.binding);
        // The Connect flow downloaded this file, but no revision check has
        // run since; "Synced" would be a timestamp for something that never
        // happened.
        self.sync_status = SyncStatus::Idle;
        true
    }

    pub(super) fn has_open_sync_remote(&self, drive_id: &str, item_id: &str) -> bool {
        let matches_remote = |binding: &SyncBinding| {
            binding.config.drive_id == drive_id && binding.config.item_id == item_id
        };
        self.sync.as_ref().is_some_and(matches_remote)
            || self.parked.values().any(|session| {
                session.sync.as_ref().is_some_and(|binding| {
                    binding.config.drive_id == drive_id && binding.config.item_id == item_id
                })
            })
            || self.pending_sync.as_ref().is_some_and(|pending| {
                pending.binding.config.drive_id == drive_id
                    && pending.binding.config.item_id == item_id
            })
    }

    /// Snapshot just enough state to run a sync against `target` from a
    /// background task - works whether `target` is the active vault or
    /// one we parked away from. Returns `None` when the vault is locked
    /// or has no sync binding (= local-only / disconnected).
    /// The config and token a push needs. It used to hand back a third
    /// element, a fresh heap copy of the master password, which both callers
    /// bound to `_master_password` and dropped unread: a cleartext copy made
    /// on every sync attempt, never used and never wiped.
    pub(super) fn snapshot_sync_inputs(
        &self,
        target: &Path,
    ) -> Option<(crate::sync::config::SyncConfig, AccessToken)> {
        let binding = if self.is_active_vault(target) {
            self.sync.as_ref()?
        } else {
            self.parked.get(target)?.sync.as_ref()?
        };
        Some((binding.config.clone(), binding.access_token.clone()))
    }

    pub(super) fn snapshot_sync_inputs_for_session(
        &self,
        target: &Path,
        session_id: VaultSessionId,
    ) -> Option<(crate::sync::config::SyncConfig, AccessToken)> {
        self.vault_session_is_current(target, session_id)
            .then(|| self.snapshot_sync_inputs(target))
            .flatten()
    }

    /// Borrow the live `Database` for the vault at `target` if it's
    /// still unlocked. Used by the conflict-diff path so it works for
    /// vaults parked while a sync was in flight.
    pub(super) fn database_clone_for(&self, target: &Path) -> Option<keepass::Database> {
        if let VaultStatus::Open { path, document, .. } = &self.vault
            && path.as_path() == target
        {
            return Some(document.database().clone());
        }
        self.parked
            .get(target)
            .map(|p| p.document.database().clone())
    }

    pub(super) fn database_clone_for_session(
        &self,
        target: &Path,
        session_id: VaultSessionId,
    ) -> Option<keepass::Database> {
        self.vault_session_is_current(target, session_id)
            .then(|| self.database_clone_for(target))
            .flatten()
    }

    pub(super) fn database_key_for_session(
        &self,
        target: &Path,
        session_id: VaultSessionId,
    ) -> Option<DatabaseKey> {
        if !self.vault_session_is_current(target, session_id) {
            return None;
        }
        if let VaultStatus::Open { path, document, .. } = &self.vault
            && path.as_path() == target
        {
            return Some(document.database_key());
        }
        self.parked
            .get(target)
            .map(|session| session.document.database_key())
    }

    /// Resolve the session's document to a deferred bytes-reader. The lookup
    /// runs here on the UI thread; the returned closure takes the storage
    /// mutex + file locks and must only be invoked on a background thread -
    /// an in-flight save holds those guards across the whole KDF + fsync.
    pub(super) fn current_bytes_reader_for_session(
        &self,
        target: &Path,
        session_id: VaultSessionId,
    ) -> Option<impl FnOnce() -> Result<Arc<Vec<u8>>, crate::keepass::SaveError> + Send + 'static>
    {
        if self.active_vault_session_id == Some(session_id)
            && let VaultStatus::Open { path, document, .. } = &self.vault
            && path.as_path() == target
        {
            return Some(document.current_bytes_reader());
        }
        self.parked
            .get(target)
            .filter(|session| session.session_id == session_id)
            .map(|session| session.document.current_bytes_reader())
    }

    pub(super) fn document_generation_for_session(
        &self,
        target: &Path,
        session_id: VaultSessionId,
    ) -> Option<u64> {
        if self.active_vault_session_id == Some(session_id)
            && let VaultStatus::Open { path, document, .. } = &self.vault
            && path.as_path() == target
        {
            return Some(document.generation());
        }
        self.parked
            .get(target)
            .filter(|session| session.session_id == session_id)
            .map(|session| session.document.generation())
            .or_else(|| {
                self.deferred_lock_sessions
                    .get(&session_id)
                    .filter(|session| session.path == target)
                    .map(|session| session.document.generation())
            })
    }

    /// Install a merged database only in the session that initiated the save.
    /// The existing document retains its cached key and shared storage state;
    /// derived UI caches are rebuilt from the merged snapshot in the same
    /// foreground update.
    pub(super) fn replace_database_for_session(
        &mut self,
        target: &Path,
        session_id: VaultSessionId,
        database: Database,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.active_vault_session_id == Some(session_id) && self.is_active_vault(target) {
            if let VaultStatus::Open {
                document,
                selection,
                selected_entry_id,
                search_query,
                visible_entries,
                selected_strength,
                last_used,
                ..
            } = &mut self.vault
            {
                document.replace_database(database);
                let entries =
                    entries_for_selection(document.snapshot(), selection, search_query, last_used);
                if !entries
                    .iter()
                    .any(|entry| Some(entry.id.as_str()) == selected_entry_id.as_deref())
                {
                    *selected_entry_id = entries.first().map(|entry| entry.id.clone());
                }
                *selected_strength = selected_entry_id
                    .as_deref()
                    .and_then(|id| document.strength_for_entry(id));
                *visible_entries = Rc::new(entries);
                cx.notify();
                return true;
            }
        } else if let Some(parked) = self
            .parked
            .get_mut(target)
            .filter(|parked| parked.session_id == session_id)
        {
            parked.document.replace_database(database);
            let entries = entries_for_selection(
                parked.document.snapshot(),
                &parked.selection,
                &parked.search_query,
                &parked.last_used,
            );
            if !entries
                .iter()
                .any(|entry| Some(entry.id.as_str()) == parked.selected_entry_id.as_deref())
            {
                parked.selected_entry_id = entries.first().map(|entry| entry.id.clone());
            }
            parked.selected_strength = parked
                .selected_entry_id
                .as_deref()
                .and_then(|id| parked.document.strength_for_entry(id));
            parked.visible_entries = Rc::new(entries);
            return true;
        } else if let Some(deferred) = self
            .deferred_lock_sessions
            .get_mut(&session_id)
            .filter(|deferred| deferred.path == target)
        {
            deferred.document.replace_database(database);
            return true;
        }
        false
    }

    /// Preserve a failed merge result for retry only when no newer mutation
    /// was queued and the quarantined document still matches the merge base.
    pub(super) fn install_failed_merge_for_deferred_retry(
        &mut self,
        target: &Path,
        session: SyncSession,
        base_generation: u64,
        pending_session: Option<VaultSessionId>,
        database: Database,
    ) -> bool {
        if pending_session.is_some() {
            return false;
        }
        let Some(deferred) = self
            .deferred_lock_sessions
            .get_mut(&session.vault)
            .filter(|deferred| deferred.path == target)
        else {
            return false;
        };
        if deferred.document.generation() != base_generation {
            return false;
        }
        deferred.document.replace_database(database);
        true
    }
}

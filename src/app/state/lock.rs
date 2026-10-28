//! Locking: a failed vault selection, the deferred lock that waits for
//! pending saves, quarantined sessions, and discarding unsaved work to
//! lock now.

use super::*;

impl AppState {
    pub fn fail_vault_selection(
        &mut self,
        path: Option<PathBuf>,
        message: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        self.record_vault_selection_failure(path, message.into());
        cx.notify();
    }

    pub(super) fn record_vault_selection_failure(
        &mut self,
        path: Option<PathBuf>,
        message: String,
    ) {
        if matches!(
            self.vault,
            VaultStatus::Open { .. } | VaultStatus::LockedPendingSave
        ) {
            self.vault_selection_error = Some(message);
            return;
        }

        self.active_vault_session_id = None;
        self.vault_selection_error = None;
        self.rotate_auto_type_context();
        self.vault = VaultStatus::Error { message, path };
    }

    pub(super) fn refresh_locked_pending_save_status(&mut self) {
        if !matches!(self.vault, VaultStatus::LockedPendingSave) {
            return;
        }
        self.save_status = self
            .deferred_lock_sessions
            .values()
            .find_map(|session| match &session.save_status {
                SaveStatus::Failed(message) => Some(SaveStatus::Failed(message.clone())),
                _ => None,
            })
            .unwrap_or(SaveStatus::Saving);
    }

    /// Revoke all user access immediately while retaining only documents that
    /// still need a local durability retry. Every public vault lookup ignores
    /// `deferred_lock_sessions` by construction.
    pub(super) fn quarantine_sessions_for_deferred_lock(&mut self) {
        if matches!(self.vault, VaultStatus::LockedPendingSave) {
            return;
        }

        self.rotate_auto_type_context();
        self.discard_deferred_armed = false;
        self.deferred_lock_since = Some(std::time::Instant::now());
        let previous_vault = std::mem::replace(&mut self.vault, VaultStatus::LockedPendingSave);
        let mut candidates = Vec::with_capacity(self.parked.len() + 1);
        if let VaultStatus::Open { path, document, .. } = previous_vault {
            let session_id = self
                .active_vault_session_id
                .take()
                .expect("open vault must have a session id");
            candidates.push((
                path,
                session_id,
                document,
                std::mem::take(&mut self.save_status),
            ));
        }
        for (path, session) in std::mem::take(&mut self.parked) {
            let ParkedSession {
                session_id,
                document,
                save_status,
                ..
            } = session;
            candidates.push((path, session_id, document, save_status));
        }

        for (path, session_id, document, save_status) in candidates {
            if self.unpersisted_vault_sessions.contains(&session_id)
                || self.save_queue_contains_session(session_id)
            {
                self.deferred_lock_sessions.insert(
                    session_id,
                    DeferredSaveSession {
                        path,
                        document,
                        save_status,
                    },
                );
            }
        }

        self.parked_order.clear();
        self.active_vault_session_id = None;
        self.overlay = Overlay::None;
        self.sync = None;
        self.sync_status = SyncStatus::Disconnected;
        self.deferred_conflicts.clear();
        self.sync_history.clear();
        self.connect_operations.advance();
        self.connect_flow = None;
        self.pending_sync = None;
        self.reconnect_target = None;
        // Holds published vault ciphertext; it must not outlive the lock.
        self.pushes_awaiting_restore.clear();
        self.auto_sync_in_flight.clear();
        self.syncs_in_flight.clear();
        self.clear_biometric_attempt();
        self.pending_biometric_enrollment = false;
        self.refresh_locked_pending_save_status();

        debug_assert!(
            self.unpersisted_vault_sessions
                .iter()
                .all(|session_id| { self.deferred_lock_sessions.contains_key(session_id) })
        );
    }

    /// Remove all decrypted vault state. Callers must first establish that no
    /// local save work can still be lost.
    pub(super) fn lock_vault_now(&mut self) {
        debug_assert!(!self.has_unpersisted_save_work());
        self.rotate_auto_type_context();
        self.discard_deferred_armed = false;
        // Nothing is waiting for an answer once there is nothing to answer
        // about: the next unlock starts from whatever the file says then.
        self.deferred_conflicts.clear();
        #[cfg(test)]
        characterization_tests::WipeWitness::observe(self.wipe_witness.as_ref());
        self.vault = VaultStatus::Empty;
        self.active_vault_session_id = None;
        self.overlay = Overlay::None;
        self.save_status = SaveStatus::Idle;
        self.sync = None;
        self.sync_status = SyncStatus::Disconnected;
        self.connect_operations.advance();
        self.connect_flow = None;
        self.pending_sync = None;
        self.reconnect_target = None;
        // Holds published vault ciphertext; it must not outlive the lock.
        self.pushes_awaiting_restore.clear();
        // A run belonging to the vault being locked. Leaving it Running made
        // `start_favicon_download` early-return for the next vault opened.
        self.favicon_status = FaviconDownloadStatus::Idle;
        // A stale "not a valid KDBX" message would otherwise reappear on the
        // next Welcome screen.
        self.vault_selection_error = None;
        self.deferred_lock_since = None;
        // Clear with the rest of the session secrets - entry titles in
        // the history would otherwise outlive the unlocked DB they came
        // from, which contradicts the rest of the lock contract.
        self.sync_history.clear();
        // Global auto-lock semantics: any parked vault gets wiped too so
        // a single idle timeout sweeps every decrypted session at once.
        self.parked.clear();
        self.parked_order.clear();
        self.deferred_lock_sessions.clear();
        self.lock_requested_after_save = false;
        // Touch ID UI bits are session-scoped; drop them so the next
        // unlock screen starts clean rather than re-showing a stale
        // "Touch ID cancelled" toast from minutes ago.
        self.clear_biometric_attempt();
        self.pending_biometric_enrollment = false;
    }

    pub(super) fn finish_deferred_lock_if_ready(&mut self) -> bool {
        if !self.lock_requested_after_save || self.has_unpersisted_save_work() {
            return false;
        }
        self.lock_vault_now();
        true
    }

    /// Retry sessions left dirty by a previous failed write. In-flight and
    /// already-queued sessions need no extra request: their normal completion
    /// path will finish the deferred lock.
    pub(super) fn retry_unpersisted_vault_saves(&mut self, cx: &mut Context<Self>) {
        let retries: Vec<(PathBuf, VaultSessionId)> = self
            .unpersisted_vault_sessions
            .iter()
            .copied()
            .filter(|session_id| !self.save_queue_contains_session(*session_id))
            .filter_map(|session_id| {
                self.path_for_vault_session(session_id)
                    .map(|path| (path, session_id))
            })
            .collect();

        for (path, session_id) in retries {
            // Lock only promises durable local bytes. A cloud push can resume
            // after the next unlock and must not delay clearing secrets.
            self.request_save_for_session(path, session_id, false, cx);
        }
    }

    /// `true` when a vault finished unlocking less than `window` ago.
    /// `NSWorkspaceDidWakeNotification` and friends routinely arrive
    /// seconds after the user is already back and re-authenticated; without
    /// this grace check the session-lock monitor would re-lock the vault
    /// mid-typing on every wake.
    pub fn unlocked_within(&self, window: std::time::Duration) -> bool {
        // A backwards clock jump makes `duration_since` fail - treat that
        // as "grace expired": the fail-safe direction is locking.
        self.last_unlock_at.is_some_and(|at| {
            std::time::SystemTime::now()
                .duration_since(at)
                .is_ok_and(|elapsed| elapsed < window)
        })
    }

    /// Whether the deferred-lock screen may offer "Discard changes and
    /// lock": every remaining save has actually failed, or the deferred
    /// lock has been waiting long past any realistic save duration - the
    /// flock has a deadline, but open/canonicalize/write/fsync on a dead
    /// network volume can block a save task forever. The action is hidden once
    /// a worker has claimed publication because those bytes can no longer be
    /// truthfully described as discarded.
    pub fn can_discard_deferred_saves(&self) -> bool {
        self.deferred_discard_gate_open() && !self.deferred_save_is_publishing()
    }

    fn deferred_discard_gate_open(&self) -> bool {
        if !matches!(self.vault, VaultStatus::LockedPendingSave) {
            return false;
        }
        let failed_and_settled =
            matches!(self.save_status, SaveStatus::Failed(_)) && self.saves_in_flight.is_empty();
        let stuck = self
            .deferred_lock_since
            .is_some_and(|since| since.elapsed() >= DEFERRED_LOCK_STUCK_AFTER);
        failed_and_settled || stuck
    }

    /// A writer has crossed the last abortable point and may already have
    /// replaced the vault file. Its task must report the result before the app
    /// can truthfully discard anything or release the lifecycle guard.
    pub fn deferred_save_is_publishing(&self) -> bool {
        matches!(self.vault, VaultStatus::LockedPendingSave)
            && self
                .saves_in_flight
                .values()
                .any(|queued| queued.abort.publication_started())
    }

    pub fn discard_deferred_armed(&self) -> bool {
        self.discard_deferred_armed
    }

    pub fn arm_discard_deferred(&mut self, armed: bool, cx: &mut Context<Self>) {
        self.discard_deferred_armed = armed;
        cx.notify();
    }

    /// Escape hatch for a deferred lock whose save keeps failing (volume
    /// unmounted, disk full, file replaced externally): drop the quarantined
    /// documents together with their unsaved changes and finish the lock.
    /// Without this, a persistent save failure wedges the app - unlock is
    /// refused, quit is vetoed, and retry loops forever.
    pub fn discard_deferred_saves_and_lock(&mut self, cx: &mut Context<Self>) {
        if self.discard_deferred_saves_and_lock_now() {
            cx.notify();
        } else if self.deferred_save_is_publishing() {
            // The confirmation may have raced the worker's publication CAS.
            // Disarm it and redraw so the UI can represent that waiting (or
            // Force Quit) is now the only honest choice.
            self.discard_deferred_armed = false;
            cx.notify();
        }
    }

    pub(super) fn discard_deferred_saves_and_lock_now(&mut self) -> bool {
        // Use the time/status gate directly rather than `can_discard`: the UI
        // may have rendered the action while all writers were still abortable
        // and then lose the CAS race to a publisher before this callback.
        if !self.deferred_discard_gate_open() {
            return false;
        }
        // Arbitrate every writer before mutating AppState. This can partially
        // abort a mixed Writing/Publishing set, but if any publisher already
        // won, every session, document and lifecycle-guard contribution stays
        // retained until its normal completion callback establishes what is
        // durable. `try_abort` is idempotent so a later attempt can safely
        // finish after the publishing slot settles.
        let mut all_aborted = true;
        for queued in self.saves_in_flight.values() {
            all_aborted &= queued.abort.try_abort();
        }
        if !all_aborted {
            return false;
        }

        let sessions: Vec<VaultSessionId> = self
            .unpersisted_vault_sessions
            .iter()
            .copied()
            .chain(self.deferred_lock_sessions.keys().copied())
            .collect();
        for session_id in sessions {
            self.mark_vault_session_persisted(session_id);
        }
        self.saves_in_flight.clear();
        self.lock_requested_after_save = false;
        self.deferred_lock_since = None;
        self.lock_vault_now();
        true
    }

    /// Lock immediately when disk is current, otherwise retry failures and
    /// defer wiping the decrypted sessions until the final save succeeds.
    /// Returns `true` only when the lock completed synchronously.
    #[must_use]
    pub fn lock_vault(&mut self, cx: &mut Context<Self>) -> bool {
        if self.has_unpersisted_save_work() {
            self.lock_requested_after_save = true;
            self.quarantine_sessions_for_deferred_lock();
            self.retry_unpersisted_vault_saves(cx);
            // The stuck-save escape (`can_discard_deferred_saves`) is
            // time-gated, but a genuinely hung save emits no further state
            // events to re-render on - poke the UI once the gate opens so
            // the discard button actually appears.
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(DEFERRED_LOCK_STUCK_AFTER + std::time::Duration::from_secs(1))
                    .await;
                let _ = this.update(cx, |state, cx| {
                    if matches!(state.vault, VaultStatus::LockedPendingSave) {
                        cx.notify();
                    }
                });
            })
            .detach();
            cx.notify();
            return false;
        }

        self.lock_vault_now();
        cx.notify();
        true
    }
}

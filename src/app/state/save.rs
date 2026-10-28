//! Background saves: the payload per vault session and one writer per file.

use super::*;

impl AppState {
    /// Spawn an atomic save of the open vault on a background thread.
    ///
    /// Concurrency model: snapshots the live `Database` once on the foreground
    /// (cheap memcpy) and ships the clone + key material to a worker. The UI
    /// thread is free during the ~500 ms Argon2 KDF. Saves are serialized per
    /// path via `saves_in_flight`: while one runs, further requests only set
    /// a flag, and the completion callback snapshots the then-latest document
    /// state for one trailing save - the latest state always wins, but we
    /// never have two writers racing on the same file.
    pub fn save_async(&mut self, cx: &mut Context<Self>) {
        self.save_async_internal(true, cx);
    }

    /// Save locally but skip the cloud-sync push afterwards. Used for
    /// purely cosmetic mutations (today: sidebar group collapse/expand)
    /// where firing a SharePoint upload on every click would burn
    /// bandwidth and - worse - race against any already-in-flight sync
    /// to produce a 412 ETag mismatch and an unnecessary Conflict
    /// overlay. The change still rides out to the cloud the next time
    /// any "real" mutation triggers `save_async`, so other devices
    /// eventually see the updated flags.
    pub fn save_async_local_only(&mut self, cx: &mut Context<Self>) {
        self.save_async_internal(false, cx);
    }

    fn save_async_internal(&mut self, sync_after: bool, cx: &mut Context<Self>) {
        if matches!(self.vault, VaultStatus::LockedPendingSave) {
            self.retry_unpersisted_vault_saves(cx);
            return;
        }
        let VaultStatus::Open { path, .. } = &self.vault else {
            return;
        };
        let target = path.clone();
        let Some(session_id) = self.active_vault_session_id else {
            return;
        };
        self.request_save_for_session(target, session_id, sync_after, cx);
    }

    /// Snapshot a `SavePayload` only from the session that issued the request.
    /// Matching only by path would let a reopened copy of the same file donate
    /// its payload to an old session's trailing save.
    pub(super) fn save_payload_for_session(
        &self,
        target: &Path,
        session_id: VaultSessionId,
    ) -> Option<SavePayload> {
        if let VaultStatus::Open { document, path, .. } = &self.vault
            && path == target
            && self.active_vault_session_id == Some(session_id)
        {
            return Some(document.save_payload());
        }
        self.parked
            .get(target)
            .filter(|session| session.session_id == session_id)
            .map(|session| session.document.save_payload())
            .or_else(|| {
                self.deferred_lock_sessions
                    .get(&session_id)
                    .filter(|session| session.path == target)
                    .map(|session| session.document.save_payload())
            })
    }

    /// Build a merged save from the exact session that owns the file. This
    /// retains the document's shared disk revision guard; constructing a free
    /// payload here would let conflict resolution bypass external-change
    /// detection and overwrite a newer file from another KeePass client.
    pub(super) fn save_payload_for_database_for_session(
        &self,
        target: &Path,
        session_id: VaultSessionId,
        database: Database,
    ) -> Option<SavePayload> {
        if let VaultStatus::Open { document, path, .. } = &self.vault
            && path == target
            && self.active_vault_session_id == Some(session_id)
        {
            return Some(document.save_payload_for_database(database));
        }
        if let Some(session) = self
            .parked
            .get(target)
            .filter(|session| session.session_id == session_id)
        {
            return Some(session.document.save_payload_for_database(database));
        }
        self.deferred_lock_sessions
            .get(&session_id)
            .filter(|session| session.path == target)
            .map(|session| session.document.save_payload_for_database(database))
    }

    /// Run or queue a save for an exact vault session. The per-path slot still
    /// serializes writers, while `session_id` prevents cross-session results.
    pub(super) fn request_save_for_session(
        &mut self,
        target: PathBuf,
        session_id: VaultSessionId,
        sync_after: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.save_session_is_retained(&target, session_id) {
            return;
        }
        self.mark_vault_session_unpersisted(session_id);

        if let Some(queued) = self.saves_in_flight.get_mut(&target) {
            queued.pending_session = Some(session_id);
            queued.sync_after |= sync_after;
            self.apply_save_status_for_session(&target, session_id, SaveStatus::Saving, cx);
            return;
        }
        // Resolve the payload before announcing `Saving`. Announcing first and
        // then bailing left the session counted as unpersisted with no writer
        // running: quit stayed vetoed, the status never left `Saving`, and the
        // discard escape waits for `Failed`, which would never arrive.
        let Some(payload) = self.save_payload_for_session(&target, session_id) else {
            self.apply_save_status_for_session(
                &target,
                session_id,
                SaveStatus::Failed("This vault is no longer open for saving.".into()),
                cx,
            );
            return;
        };
        self.apply_save_status_for_session(&target, session_id, SaveStatus::Saving, cx);
        let queued_save = QueuedSave::new(session_id);
        let abort = queued_save.abort.clone();
        self.saves_in_flight.insert(target.clone(), queued_save);

        let save_target = target.clone();
        let task =
            cx.background_spawn(async move { payload.save_to_with_abort(&save_target, &abort) });

        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| {
                let Some(queued) = state.saves_in_flight.remove(&target) else {
                    return;
                };
                if queued.owner != session_id {
                    // A different writer owns the slot. This should be
                    // unreachable, but never let an unexpected callback
                    // consume or update another session's bookkeeping.
                    state.saves_in_flight.insert(target, queued);
                    return;
                }
                let receipt = match result {
                    Ok(receipt) => Some(receipt),
                    Err(error) => {
                        state.apply_save_status_for_session(
                            &target,
                            session_id,
                            SaveStatus::Failed(error.to_string()),
                            cx,
                        );
                        None
                    }
                };
                let durable = receipt.as_ref().is_some_and(|receipt| receipt.is_durable());
                if let Some(error) = receipt
                    .as_ref()
                    .and_then(|receipt| receipt.durability_error())
                {
                    state.apply_save_status_for_session(
                        &target,
                        session_id,
                        SaveStatus::Failed(format!(
                            "Vault was written, but directory sync failed: {error}"
                        )),
                        cx,
                    );
                } else if durable {
                    state.apply_save_status_for_session(&target, session_id, SaveStatus::Saved, cx);
                }

                // A successful payload makes its owner durable unless that
                // same session requested a newer trailing snapshot.
                if durable && queued.pending_session != Some(session_id) {
                    state.mark_vault_session_persisted(session_id);
                }

                if let Some(pending_session) = queued.pending_session {
                    // More mutations landed while this save ran. Run one
                    // trailing save of the *latest* document state; any
                    // cloud push (ours or the queued requests') rides on
                    // that save instead, so the newest bytes are what
                    // reach the server - pushing now would upload state
                    // the trailing save is about to supersede.
                    state.request_save_for_session(
                        target,
                        pending_session,
                        queued.sync_after || sync_after,
                        cx,
                    );
                    return;
                }

                // Lock requests wait for local durability, not cloud sync.
                // Once the final writer succeeds, clear secrets immediately.
                if durable && state.finish_deferred_lock_if_ready() {
                    cx.notify();
                    return;
                }
                // Chain into sync against the same vault that just saved
                // - even if the user has switched away. `sync_now_for_path`
                // routes its results back to whichever slot still owns
                // `target`, so a parked vault's edit still makes it to
                // SharePoint.
                if durable
                    && sync_after
                    && state.vault_session_is_current(&target, session_id)
                    && state.save_should_reach_the_cloud(&target)
                    // The sync that follows belongs to whatever relationship
                    // this vault has *now*: the save carried none of its own.
                    && let Some(session) = state.sync_session_for(&target)
                {
                    let bytes = receipt.expect("durable save has a receipt").bytes();
                    state.sync_now_for_path_inner(&target, session, true, Some(bytes), cx);
                }
            });
        })
        .detach();
    }
}

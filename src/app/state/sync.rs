//! Syncing: manual and automatic runs, remote conflicts, and committing a
//! merge.

use super::*;

impl AppState {
    /// Push the active vault's local file to SharePoint. Used as the
    /// manual SyncSettings → Sync now button. No-op when no vault is
    /// active or the active vault is local-only.
    pub fn sync_now(&mut self, cx: &mut Context<Self>) {
        let target = match &self.vault {
            VaultStatus::Open { path, .. } => path.clone(),
            _ => return,
        };
        self.sync_now_for_path(&target, cx);
    }

    /// Path-aware sync trigger. Works for both the active vault and a
    /// vault the user has parked (e.g., after edit-then-switch). All
    /// background-task results are routed back to whichever slot
    /// (`vault` or `parked[target]`) holds the binding at completion
    /// time, so a sync that finishes after the user has switched away
    /// updates the saving vault, not whoever is now in focus.
    ///
    /// This is the *interactive* entry point (manual "Sync now" button,
    /// on-save push, post-resolution re-sync) - a resulting conflict is
    /// allowed to open the Conflict overlay. Background auto-sync uses
    /// `sync_now_for_path_inner(.., false, ..)` so it can never do that.
    pub fn sync_now_for_path(&mut self, target: &Path, cx: &mut Context<Self>) {
        let Some(session) = self.sync_session_for(target) else {
            return;
        };
        self.sync_now_for_path_inner(target, session, true, None, cx);
    }

    pub(super) fn sync_now_for_path_inner(
        &mut self,
        target: &Path,
        session: SyncSession,
        interactive: bool,
        published_bytes: Option<Arc<Vec<u8>>>,
        cx: &mut Context<Self>,
    ) {
        if !self.sync_session_is_current(target, session) {
            return;
        }

        // One push per vault at a time - see `syncs_in_flight`. Queue a
        // follow-up instead of racing; it runs once the in-flight push
        // completes and uses the newest request's exact session + bytes.
        if let Some(queued) = self.syncs_in_flight.get_mut(target) {
            queued.queue(QueuedSyncRequest {
                session,
                interactive,
                bytes: published_bytes,
            });
            return;
        }
        let Some((config, token)) = self.snapshot_sync_inputs_for_session(target, session.vault)
        else {
            // No live binding. For the active vault this usually means the
            // unlock-time binding restore failed (network blip), so retry it
            // and hold this push until it lands: `retry_sync_restore` is
            // idempotent, and auto-sync is pull-only unless the status is
            // `Failed`, so a dropped push here was never retried by anything.
            // Parked vaults keep the silent bail: the restore path is
            // active-vault-only by design.
            if self.is_active_vault(target) {
                self.queue_push_awaiting_restore(
                    target,
                    QueuedSyncRequest {
                        session,
                        interactive,
                        bytes: published_bytes,
                    },
                );
                self.retry_sync_restore(cx);
            }
            return;
        };
        let bytes_source = match published_bytes {
            Some(bytes) => BytesSource::Published(bytes),
            None => match self.current_bytes_reader_for_session(target, session.vault) {
                Some(reader) => BytesSource::Read(Box::new(reader)),
                None => return,
            },
        };
        let local_path = target.to_path_buf();
        self.syncs_in_flight
            .insert(local_path.clone(), QueuedSync::new(session));

        self.apply_sync_status_for_session(target, session, SyncStatus::Syncing, cx);

        let task_config = config.clone();
        let task = cx.background_spawn(async move {
            let bytes = match bytes_source {
                BytesSource::Published(bytes) => bytes,
                BytesSource::Read(reader) => reader().map_err(|error| {
                    crate::sync::service::ServiceError::LocalVault(error.to_string())
                })?,
            };
            let token = crate::sync::service::ensure_provider_ready(&task_config, token)?;
            let outcome = crate::sync::service::upload_after_save(&task_config, &token, &bytes)?;
            let uploaded = crate::sync::config::local_revision(&bytes);
            Ok::<_, crate::sync::service::ServiceError>((outcome, token, uploaded))
        });

        let callback_path = local_path;
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| {
                let Some(queued) = state.syncs_in_flight.remove(&callback_path) else {
                    return;
                };
                if queued.owner != session {
                    state.syncs_in_flight.insert(callback_path, queued);
                    return;
                }

                // Lock + reopen creates a new owner even for the same path.
                // Never apply the old token/etag/conflict to that session;
                // only hand off an explicitly queued request owned by it.
                if !state.sync_session_is_current(&callback_path, session) {
                    if let Some(pending) = queued.pending {
                        state.start_queued_sync(&callback_path, pending, cx);
                    }
                    return;
                }

                let conflict_interactive = interactive
                    || queued
                        .pending
                        .as_ref()
                        .is_some_and(|pending| pending.session == session && pending.interactive);
                match result {
                    Ok((outcome, fresh_token, uploaded_revision)) => {
                        state.with_sync_binding_mut_for_session(
                            &callback_path,
                            session,
                            |binding| binding.access_token = fresh_token,
                        );
                        use crate::sync::service::UploadAfterSave;
                        match outcome {
                            UploadAfterSave::Synced { new_etag, item: _ } => {
                                // Persist best effort; if the disk write fails
                                // we'll just re-detect a conflict next push
                                // (and re-resolve).
                                if let Some(config) = state.record_upload_baseline(
                                    &callback_path,
                                    session,
                                    new_etag,
                                    uploaded_revision,
                                ) {
                                    let _ = crate::sync::config::save(&config);
                                }
                                state.apply_sync_status_for_session(
                                    &callback_path,
                                    session,
                                    SyncStatus::Synced {
                                        at: chrono::Local::now(),
                                        auto_merged: 0,
                                    },
                                    cx,
                                );
                                // Saves that completed while this push ran
                                // queued behind it - run the one follow-up
                                // push that carries their bytes.
                                if let Some(pending) = queued.pending {
                                    state.start_queued_sync(&callback_path, pending, cx);
                                }
                            }
                            UploadAfterSave::Conflict {
                                remote_bytes,
                                remote_etag,
                            } => {
                                // Any queued follow-up is deliberately
                                // dropped here: the merge diff runs against
                                // the *current* in-memory document, so edits
                                // that queued behind this push are already
                                // part of what gets merged and re-uploaded.
                                state.handle_remote_conflict_for(
                                    &callback_path,
                                    session,
                                    remote_bytes,
                                    remote_etag,
                                    conflict_interactive,
                                    cx,
                                );
                            }
                        }
                    }
                    Err(e) => {
                        // Queued follow-up dropped on purpose: it would hit
                        // the same broken network/grant immediately. The
                        // auto-sync tick's Failed-recovery branch owns the
                        // retry, and the queued edits are already on disk
                        // for it to push.
                        let status = state.note_sync_failure(&callback_path, e);
                        state.apply_sync_status_for_session(&callback_path, session, status, cx);
                    }
                }
            });
        })
        .detach();
    }

    fn start_queued_sync(
        &mut self,
        target: &Path,
        request: QueuedSyncRequest,
        cx: &mut Context<Self>,
    ) {
        if self.sync_session_is_current(target, request.session) {
            self.sync_now_for_path_inner(
                target,
                request.session,
                request.interactive,
                request.bytes,
                cx,
            );
        }
    }

    /// Run a background auto-sync pull for every synced vault in memory -
    /// the active one plus any the user parked. Driven by the AppShell's
    /// auto-sync timer. Cheap when nothing changed remotely (one metadata
    /// round-trip per vault); only downloads + merges the vaults whose
    /// remote actually advanced.
    pub fn auto_sync_all(&mut self, cx: &mut Context<Self>) {
        let mut targets: Vec<PathBuf> = Vec::new();
        if let VaultStatus::Open { path, .. } = &self.vault
            && self.sync.is_some()
        {
            targets.push(path.clone());
        }
        for (path, session) in &self.parked {
            if session.sync.is_some() {
                targets.push(path.clone());
            }
        }
        for target in targets {
            self.auto_sync_for_path(&target, cx);
        }
    }

    /// Background auto-sync *pull* for a single vault. Unlike
    /// `sync_now_for_path` (which always re-uploads the local bytes), this
    /// only ever writes to the server when there's genuinely something to
    /// pull: it does a cheap metadata check (`refresh_check`) and only
    /// downloads + merges when the remote moved ahead. That stops a
    /// 15-minute timer from minting a fresh SharePoint version every tick
    /// with byte-identical content.
    ///
    /// It doubles as the OAuth keep-alive: `ensure_fresh` refreshes the
    /// access token when it's near expiry, and we write the fresh token
    /// back into the binding - that refresh resets the refresh token's
    /// sliding-inactivity window, which is the whole point of running on a
    /// timer in the first place.
    ///
    /// Quiet by design. Transient failures (offline, a brief Graph blip)
    /// leave the current status untouched rather than flapping the UI to
    /// `Failed` on every tick. A terminal `InvalidGrant` does flip to
    /// `Reconnect` (with the Azure reason) so the user discovers an expired
    /// sign-in while the app is open - early enough to fix it before it
    /// blocks a real save.
    fn auto_sync_for_path(&mut self, target: &Path, cx: &mut Context<Self>) {
        let Some(session) = self.sync_session_for(target) else {
            return;
        };
        // Skip vaults mid-operation, and don't fire while the user is in a
        // Connect / Conflict overlay - auto-merging underneath them would
        // be jarring.
        let activity = self.sync_activity_for(target);
        let busy = activity == Some(SyncActivity::Busy);
        if busy || matches!(self.overlay, Overlay::Conflict | Overlay::Connect) {
            return;
        }
        // One auto-sync request per vault at a time. A stalled Graph call
        // must not let the next tick start a competing one.
        // Nor while a remote from an earlier sync is still being decrypted:
        // two long KDF runs for one vault would only compete for the CPU.
        if self.auto_sync_in_flight.contains(target)
            || self.remote_merges_in_flight.contains_key(target)
        {
            return;
        }
        // Recover from a prior failed sync (e.g. a push that lost the
        // network) with a full push retry - `sync_now_for_path` re-uploads
        // local bytes and falls into the merge path on a 412, so it heals
        // both stranded local edits and a half-finished merge. The cheap
        // pull-check below would miss those because the remote ETag hasn't
        // moved. `interactive: false` so a background retry of a *conflict*
        // never pops the overlay unattended.
        // A conflict waiting for the user is not a failure to retry. It
        // reports itself as failed so the pill says something, but pushing
        // again only earns the same 412 and defers again, so the cheap checks
        // below decide instead: they push when this copy has moved, look when
        // the remote may have, and otherwise leave it alone until the user
        // answers.
        if activity == Some(SyncActivity::Failed) && !self.deferred_conflicts.contains_key(target) {
            // A throttled vault waits. Retrying a full upload every tick is
            // what provoked the limit, and the recovery push is the most
            // expensive request this app makes.
            if self.sync_is_backing_off(target) {
                return;
            }
            self.sync_now_for_path_inner(target, session, false, None, cx);
            return;
        }
        let Some((config, token)) = self.snapshot_sync_inputs_for_session(target, session.vault)
        else {
            return;
        };

        // A save that never reached the cloud leaves the local file ahead of
        // what was last uploaded, and the remote etag has not moved either, so
        // the pull check below would call that Synced. It happens whenever a
        // push was dropped: a lock during a save, a quit, a crash, a binding
        // that had not been restored yet. Push instead of checking.
        if Self::local_is_ahead_of_the_cloud(target, &config) {
            self.sync_now_for_path_inner(target, session, false, None, cx);
            return;
        }

        self.auto_sync_in_flight.insert(target.to_path_buf());
        let task_config = config;
        // The revision a conflict on this vault is waiting to be answered
        // about. The check below already names the remote's, so recognising
        // the same one costs nothing and saves fetching the whole file again
        // on every interval for as long as the user puts off answering.
        let deferred = self.deferred_conflicts.get(target).cloned();
        let task = cx.background_spawn(async move {
            let token = crate::sync::service::ensure_provider_ready(&task_config, token)?;
            let pulled = match crate::sync::service::refresh_check(&task_config, &token)? {
                crate::sync::service::RefreshCheck::Same => None,
                crate::sync::service::RefreshCheck::RemoteAhead { remote_etag, .. }
                    if deferred.as_deref() == Some(remote_etag.as_str()) =>
                {
                    None
                }
                crate::sync::service::RefreshCheck::RemoteAhead { .. } => {
                    Some(crate::sync::service::download_remote(&task_config, &token)?)
                }
            };
            Ok::<_, crate::sync::service::ServiceError>((token, pulled))
        });

        let callback_path = target.to_path_buf();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| {
                state.auto_sync_in_flight.remove(&callback_path);

                // Revalidate: while this pull was in flight the user may have
                // started a manual sync, hit a conflict, disconnected, or
                // locked and reopened the same path. Results belong only to
                // the exact session that dispatched the request.
                if !state.sync_session_is_current(&callback_path, session)
                    || !state.has_sync_binding_for(&callback_path)
                {
                    return;
                }

                match result {
                    Ok((fresh_token, pulled)) => {
                        // Keep-alive: persist the (possibly refreshed) token
                        // so the next tick rides on it and the inactivity
                        // window stays reset.
                        state.with_sync_binding_mut_for_session(&callback_path, session, |b| {
                            b.access_token = fresh_token
                        });
                        // Only act on the pull result from a healthy resting
                        // state. If a manual sync moved us to Syncing, or a
                        // reconnect arrived, this background result is stale -
                        // drop it; the next tick re-checks. (The token write
                        // above is always safe and worth keeping.)
                        //
                        // A vault waiting for the user is the exception, and
                        // the one that matters: it reports itself as failed,
                        // so dropping the result here meant a remote that had
                        // moved was fetched and thrown away on every tick,
                        // and the user was never told there was more to it.
                        let waiting_for_the_user =
                            state.deferred_conflicts.get(&callback_path).cloned();
                        if waiting_for_the_user.is_none()
                            && state.sync_activity_for(&callback_path)
                                != Some(SyncActivity::Resting)
                        {
                            return;
                        }
                        match pulled {
                            // Nothing new, and a conflict is still waiting: it
                            // is not synced, whatever the keep-alive says.
                            None if waiting_for_the_user.is_some() => {}
                            None => {
                                // Up to date - stamp "synced just now" so the
                                // UI shows the keep-alive ran.
                                state.apply_sync_status_for_session(
                                    &callback_path,
                                    session,
                                    SyncStatus::Synced {
                                        at: chrono::Local::now(),
                                        auto_merged: 0,
                                    },
                                    cx,
                                );
                            }
                            Some((remote_bytes, remote_etag)) => {
                                // `interactive: false` - a background pull
                                // silently auto-merges conflict-free changes
                                // but must never replace whatever overlay the
                                // user has open with the Conflict overlay; a
                                // real conflict is deferred to a Failed hint
                                // instead.
                                state.handle_remote_conflict_for(
                                    &callback_path,
                                    session,
                                    remote_bytes,
                                    remote_etag,
                                    false,
                                    cx,
                                );
                            }
                        }
                    }
                    Err(crate::sync::service::ServiceError::Auth(
                        crate::sync::auth::AuthError::InvalidGrant(detail),
                    )) => {
                        state.apply_sync_status_for_session(
                            &callback_path,
                            session,
                            SyncStatus::Reconnect { detail },
                            cx,
                        );
                    }
                    Err(_) => {
                        // Transient - stay quiet; the next tick (or the next
                        // manual save / sync) retries.
                    }
                }
            });
        })
        .detach();
    }

    /// Decrypt remote bytes with the master password, build a `ConflictReport`
    /// against the in-memory local DB, and either open the Conflict overlay
    /// (when `target` is the active vault) or mark the parked vault Failed
    /// so the user can resolve it after switching back. Auto-merge is run
    /// for both active and parked targets - silent merges don't need the UI.
    ///
    /// `interactive` gates the Conflict overlay: the user-driven paths pass
    /// `true` and a real conflict opens the overlay; background auto-sync
    /// passes `false` and a real conflict is deferred to a Failed hint
    /// instead, so an unattended pull never hijacks whatever the user is
    /// doing. Conflict-free silent merges run regardless of `interactive`.
    fn handle_remote_conflict_for(
        &mut self,
        target: &Path,
        session: SyncSession,
        remote_bytes: Vec<u8>,
        remote_etag: String,
        interactive: bool,
        cx: &mut Context<Self>,
    ) {
        // The binding can vanish between issuing the request and this
        // callback (disconnect mid-flight). With no binding there's nothing
        // to sync against, so don't decrypt/diff or surface a conflict.
        if !self.sync_session_is_current(target, session) || !self.has_sync_binding_for(target) {
            return;
        }
        let Some(local_db) = self.database_clone_for_session(target, session.vault) else {
            // Vault was locked between issuing the upload and the 412
            // response landing - nothing to merge against. Drop silently.
            return;
        };
        // Stamp the mutation generation that `local_db` represents. The
        // merge runs against this clone while the live document stays
        // editable; `commit_merged_for` refuses to install a result whose
        // base generation no longer matches the live document.
        let Some(base_generation) = self.document_generation_for_session(target, session.vault)
        else {
            return;
        };

        let Some(remote_key) = self.database_key_for_session(target, session.vault) else {
            return;
        };

        // Decrypting runs the full Argon2 KDF: half a second normally, up to
        // a minute and more for a cost the user allowed. On this thread that
        // froze the whole window, so it runs in the background and the rest
        // picks up on the foreground with every guard checked again.
        self.remote_merge_token += 1;
        let token = self.remote_merge_token;
        self.remote_merges_in_flight
            .insert(target.to_path_buf(), token);
        let parse = cx.background_spawn(async move {
            crate::keepass::repository::parse_database_bytes(&remote_bytes, remote_key)
        });
        let target = target.to_path_buf();
        cx.spawn(async move |this, cx| {
            let parsed = parse.await;
            let _ = this.update(cx, |state, cx| {
                if state.remote_merges_in_flight.get(&target) != Some(&token) {
                    return;
                }
                state.remote_merges_in_flight.remove(&target);
                if !state.sync_session_is_current(&target, session)
                    || !state.has_sync_binding_for(&target)
                {
                    return;
                }
                state.continue_remote_conflict_for(
                    &target,
                    session,
                    parsed,
                    local_db,
                    base_generation,
                    remote_etag,
                    interactive,
                    cx,
                );
            });
        })
        .detach();
    }

    /// Second half of [`Self::handle_remote_conflict_for`], on the foreground
    /// once the remote is decrypted. `local_db` and `base_generation` are
    /// from before the decrypt: the merge runs against that snapshot, and
    /// `commit_merged_for` refuses it if the live document moved on.
    #[allow(clippy::too_many_arguments)]
    fn continue_remote_conflict_for(
        &mut self,
        target: &Path,
        session: SyncSession,
        parsed: Result<Database, keepass::db::DatabaseOpenError>,
        local_db: Database,
        base_generation: u64,
        remote_etag: String,
        interactive: bool,
        cx: &mut Context<Self>,
    ) {
        let target_is_active = matches!(
            &self.vault,
            VaultStatus::Open { path, .. } if path.as_path() == target
        );

        match parsed {
            Ok(remote_db) => {
                self.remote_kdf_confirmations.remove(target);
                let report = crate::keepass::merge::diff(&local_db, &remote_db);

                let editing = matches!(
                    self.overlay,
                    Overlay::AddEntry | Overlay::EditEntry { .. } | Overlay::AddGroup { .. }
                );
                let plan = RemoteMergePlan::of(&report, target_is_active, interactive, editing);
                if plan == RemoteMergePlan::AutoMerge {
                    let auto_merged_count = report.remote_only.len() + report.auto_resolved.len();
                    // Whether the merge actually changes the *remote*. A pure
                    // fast-forward - we only pulled remote-only additions
                    // and/or remote-wins resolutions - leaves the merged DB
                    // logically equal to what's already on the server, so
                    // there's nothing to push. Uploading anyway would mint a
                    // redundant SharePoint version for someone else's change.
                    // We must push only when the local side contributes:
                    // entries only we have, or a divergence our side won.
                    let needs_upload = report.has_local_contribution();
                    // Snapshot the change list now (silent path → empty
                    // picks), but defer the actual append until the
                    // local save phase inside commit_merged_for has
                    // succeeded. That way a save/reload failure doesn't
                    // leave phantom rows in the activity log.
                    let history_entries = sync_history::entries_from_report(
                        &report,
                        &crate::keepass::merge::Resolutions::default(),
                        chrono::Local::now(),
                    );
                    let merged = match crate::keepass::merge::apply_picks(
                        &local_db,
                        &remote_db,
                        &crate::keepass::merge::Resolutions::default(),
                        &report,
                    ) {
                        Ok(merged) => merged,
                        Err(error) => {
                            self.apply_sync_status_for_session(
                                target,
                                session,
                                SyncStatus::Failed(format!("Merge blocked: {error}")),
                                cx,
                            );
                            return;
                        }
                    };
                    self.commit_merged_for(
                        target,
                        session,
                        merged,
                        remote_etag,
                        auto_merged_count,
                        needs_upload,
                        history_entries,
                        base_generation,
                        cx,
                    );
                    return;
                }

                if plan == RemoteMergePlan::Defer {
                    // Two reasons, two sentences: a parked vault needs
                    // bringing back, everything else needs an explicit
                    // "Sync now" once the user is free.
                    let hint = if target_is_active {
                        "Remote conflict - choose Sync now to resolve."
                    } else {
                        "Remote conflict - switch back to this vault to resolve."
                    };
                    self.deferred_conflicts
                        .insert(target.to_path_buf(), remote_etag.clone());
                    self.apply_sync_status_for_session(
                        target,
                        session,
                        SyncStatus::Failed(hint.into()),
                        cx,
                    );
                    return;
                }
                self.deferred_conflicts.remove(target);

                let mut picks = crate::keepass::merge::Resolutions::default();
                for conflict in &report.conflicts {
                    picks.entries.insert(conflict.id.clone(), Side::Local);
                }
                for conflict in &report.group_conflicts {
                    picks.groups.insert(conflict.id.clone(), Side::Local);
                }
                if report.metadata_conflict.is_some() {
                    picks.metadata = Some(Side::Local);
                }
                let conflict = SyncStatus::Conflict(Box::new(ConflictState {
                    local_db,
                    remote_db,
                    remote_etag,
                    report,
                    picks,
                    base_generation,
                    session,
                }));
                // Route like every other async result even though the guard
                // above already established the active vault: the overlay
                // seizes the window, so it may only open when routing
                // confirms this session still owns the screen.
                if self.route_sync_status_for_session(target, session, conflict) {
                    self.overlay = Overlay::Conflict;
                }
                cx.notify();
            }
            Err(error) => {
                // Only credential/crypto failures mean "wrong password".
                // The resource limits in `keepass::limits` surface as
                // `Io(InvalidData)` with a user-readable message (vault too
                // large, Argon2 memory over the cap, malformed header) -
                // blaming the master password for those sends the user
                // chasing a credential problem that does not exist.
                let message = match &error {
                    _ if let Some(cost) = crate::keepass::limits::confirmation_required(&error) => {
                        self.remote_kdf_confirmations
                            .insert(target.to_path_buf(), cost);
                        "The synced vault now takes longer to unlock. \
                         Review it in Settings → Sync."
                            .to_string()
                    }
                    keepass::db::DatabaseOpenError::Io(io_error)
                        if io_error.kind() == std::io::ErrorKind::InvalidData =>
                    {
                        format!("Cannot read the remote vault: {io_error}")
                    }
                    _ => "Remote file uses a different master password - \
                          cannot merge automatically."
                        .to_string(),
                };
                self.apply_sync_status_for_session(
                    target,
                    session,
                    SyncStatus::Failed(message),
                    cx,
                );
            }
        }
    }

    /// Mutate one user pick. Called by the Conflict overlay when the user
    /// clicks "Keep this" on either side. Idempotent.
    pub fn set_conflict_pick(
        &mut self,
        kind: ConflictKind,
        id: &str,
        side: Side,
        cx: &mut Context<Self>,
    ) {
        let SyncStatus::Conflict(state) = &mut self.sync_status else {
            return;
        };
        match kind {
            ConflictKind::Entry => {
                state.picks.entries.insert(id.to_string(), side);
            }
            ConflictKind::Group => {
                state.picks.groups.insert(id.to_string(), side);
            }
            // One per merge, so the id is ignored.
            ConflictKind::Metadata => state.picks.metadata = Some(side),
        }
        cx.notify();
    }

    /// Set every pick to the same side. Called by the Conflict overlay's
    /// "All local" / "All remote" bulk buttons. Idempotent.
    pub fn set_all_conflict_picks(&mut self, side: Side, cx: &mut Context<Self>) {
        let SyncStatus::Conflict(state) = &mut self.sync_status else {
            return;
        };
        for conflict in &state.report.conflicts {
            state.picks.entries.insert(conflict.id.clone(), side);
        }
        for conflict in &state.report.group_conflicts {
            state.picks.groups.insert(conflict.id.clone(), side);
        }
        if state.report.metadata_conflict.is_some() {
            state.picks.metadata = Some(side);
        }
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
        let target = match &self.vault {
            VaultStatus::Open { path, .. } => path.clone(),
            _ => return,
        };
        let (merge_result, remote_etag, base_generation, history_entries, session) =
            match &self.sync_status {
                SyncStatus::Conflict(state) => (
                    crate::keepass::merge::apply_picks(
                        &state.local_db,
                        &state.remote_db,
                        &state.picks,
                        &state.report,
                    ),
                    state.remote_etag.clone(),
                    state.base_generation,
                    sync_history::entries_from_report(
                        &state.report,
                        &state.picks,
                        chrono::Local::now(),
                    ),
                    state.session,
                ),
                _ => return,
            };
        if !self.sync_session_is_current(&target, session) {
            return;
        }
        let merged = match merge_result {
            Ok(merged) => merged,
            Err(error) => {
                self.route_sync_status_for_session(
                    &target,
                    session,
                    SyncStatus::Failed(format!("Merge blocked: {error}")),
                );
                self.overlay = Overlay::None;
                cx.notify();
                return;
            }
        };

        // User-driven resolution - the "Synced · N merged" badge is reserved
        // for git-style silent merges where the user got no overlay at all.
        // Manual resolution always reports auto_merged = 0. It also always
        // uploads: the user just chose sides, so the merged result is a
        // deliberate new state that must reach the server.
        self.commit_merged_for(
            &target,
            session,
            merged,
            remote_etag,
            0,
            true,
            history_entries,
            base_generation,
            cx,
        );
    }

    /// Save a merged Database locally, reload the in-memory document from
    /// the freshly-encrypted bytes, and force-upload to SharePoint with the
    /// supplied `If-Match` ETag. Used by both:
    ///
    /// - **Manual conflict resolution** (`apply_conflict_resolution`) where
    ///   the user picked sides in the overlay, and
    /// - **Silent auto-merge** (in `handle_remote_conflict` when the diff
    ///   is conflict-free) where there was nothing for the user to decide.
    ///
    /// `auto_merged` is the count surfaced in the "Synced · N merged" badge -
    /// non-zero only on the silent-merge path.
    ///
    /// `history_entries` are the pre-computed activity-log rows for this
    /// merge. They're appended to the target vault's history only after
    /// the local save + reload succeeds (phase 1) - so a save failure
    /// can't leave phantom rows referencing changes that never
    /// actually committed.
    ///
    /// `base_generation` is the document's mutation generation at the time
    /// `merged` was computed. The UI stays fully interactive while the
    /// ~500 ms merge save runs, so the user can edit the live document in
    /// that window; installing the merged copy would silently discard
    /// those edits, and uploading it would publish a remote state that
    /// lacks them. Whenever the live generation no longer matches, the
    /// commit aborts and instead schedules a save + sync of the *latest*
    /// state - the resulting 412 re-runs the whole download → diff →
    /// merge cycle against that state, so neither side's changes are lost.
    #[allow(clippy::too_many_arguments)]
    fn commit_merged_for(
        &mut self,
        target: &Path,
        session: SyncSession,
        merged: keepass::Database,
        remote_etag: String,
        auto_merged: usize,
        needs_upload: bool,
        history_entries: Vec<SyncHistoryEntry>,
        base_generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !self.sync_session_is_current(target, session) {
            return;
        }
        // Pull config + token from whichever slot owns `target` right now.
        let (config, token) = match self.snapshot_sync_inputs_for_session(target, session.vault) {
            Some(inputs) => inputs,
            None => return,
        };

        let local_path = target.to_path_buf();

        // Abort before the expensive save when the merge base is already
        // stale (the user edited while the Conflict overlay was open) or
        // an ordinary save is mid-flight for this path (its bytes will
        // land on disk at an arbitrary point relative to ours). Either
        // way: write + push the latest state instead and let the 412
        // cycle recompute the merge against it.
        if self.document_generation_for_session(target, session.vault) != Some(base_generation)
            || self.saves_in_flight.contains_key(&local_path)
        {
            self.apply_sync_status_for_session(target, session, SyncStatus::Syncing, cx);
            self.request_save_for_session(local_path, session.vault, true, cx);
            return;
        }
        let merged_for_document = merged.clone();
        let Some(payload) =
            self.save_payload_for_database_for_session(target, session.vault, merged)
        else {
            return;
        };

        // Hold the per-path save slot for the whole of phase 1 so ordinary
        // saves queue behind the merge write instead of racing it on disk.
        self.mark_vault_session_unpersisted(session.vault);
        self.apply_save_status_for_session(target, session.vault, SaveStatus::Saving, cx);
        let queued_save = QueuedSave::new(session.vault);
        let abort = queued_save.abort.clone();
        self.saves_in_flight.insert(local_path.clone(), queued_save);

        let if_match = remote_etag;

        self.apply_sync_status_for_session(target, session, SyncStatus::Syncing, cx);

        // Phase 1: local merge save. Splitting this off from the network
        // step lets us commit the merge into the in-memory document
        // *before* we go anywhere near the network. Without that, an
        // upload failure (or a token-refresh failure) parked the user back
        // on the pre-merge in-memory state while the already-merged bytes
        // sat on disk - the next ordinary save would clobber the merge
        // with stale data.
        let save_path = local_path.clone();
        let local_save_task =
            cx.background_spawn(async move { payload.save_to_with_abort(&save_path, &abort) });

        let callback_path = local_path;

        cx.spawn(async move |this, cx| {
            // Wrapped in an Option so the inner FnOnce can `.take()` it
            // on the success branch without forcing the whole closure to
            // `move` (which would also consume `callback_path`, still
            // needed by phase 2 below).
            let mut history_slot = Some(history_entries);
            let (save_receipt, save_error) = match local_save_task.await {
                Ok(receipt) => (Some(receipt), None),
                Err(error) => (None, Some(error.to_string())),
            };
            let published_bytes = this
                .update(cx, |state, cx| {
                    // Release the per-path save slot taken above; pick up
                    // any ordinary-save request that queued behind the
                    // merge write meanwhile.
                    let queued = state.saves_in_flight.remove(&callback_path)?;
                    if queued.owner != session.vault {
                        state.saves_in_flight.insert(callback_path.clone(), queued);
                        return None;
                    }
                    let Some(receipt) = save_receipt.as_ref() else {
                        let error = save_error
                            .as_deref()
                            .unwrap_or("merge save failed before publication");
                        // Access may have been revoked while the merge save was
                        // running. Preserve the user's resolved merge inside the
                        // save-only quarantine so a retry cannot write the older
                        // pre-merge document back to disk.
                        let _ = state.install_failed_merge_for_deferred_retry(
                            &callback_path,
                            session,
                            base_generation,
                            queued.pending_session,
                            merged_for_document,
                        );
                        state.apply_save_status_for_session(
                            &callback_path,
                            session.vault,
                            SaveStatus::Failed(error.into()),
                            cx,
                        );
                        state.apply_sync_status_for_session(
                            &callback_path,
                            session,
                            SyncStatus::Failed(error.into()),
                            cx,
                        );
                        if let Some(pending_session) = queued.pending_session {
                            state.request_save_for_session(
                                callback_path.clone(),
                                pending_session,
                                true,
                                cx,
                            );
                        }
                        return None;
                    };
                    // The user edited this vault while the merged bytes
                    // were being encrypted + written (the UI stays
                    // interactive during the ~500 ms background save), or
                    // locked it outright. Installing the merged document
                    // would silently discard those edits - and phase 2
                    // would publish a remote state that lacks them. Abort:
                    // the save scheduled here writes the *latest* in-memory
                    // state back over the merged file on disk, and its
                    // chained sync re-runs the 412 → diff → merge cycle
                    // against that state, so neither side loses changes.
                    if state.document_generation_for_session(&callback_path, session.vault)
                        != Some(base_generation)
                    {
                        state.request_save_for_session(
                            callback_path.clone(),
                            session.vault,
                            true,
                            cx,
                        );
                        if let Some(pending_session) = queued.pending_session
                            && pending_session != session.vault
                        {
                            state.request_save_for_session(
                                callback_path.clone(),
                                pending_session,
                                true,
                                cx,
                            );
                        }
                        return None;
                    }
                    let installed = state.replace_database_for_session(
                        &callback_path,
                        session.vault,
                        merged_for_document,
                        cx,
                    );
                    if installed && let Some(entries) = history_slot.take() {
                        state.append_sync_history_for(&callback_path, entries);
                    }

                    let durable = receipt.is_durable();
                    if let Some(error) = receipt.durability_error() {
                        let message =
                            format!("Merge was written, but directory sync failed: {error}");
                        state.apply_save_status_for_session(
                            &callback_path,
                            session.vault,
                            SaveStatus::Failed(message.clone()),
                            cx,
                        );
                        state.apply_sync_status_for_session(
                            &callback_path,
                            session,
                            SyncStatus::Failed(message),
                            cx,
                        );
                    } else if installed {
                        state.apply_save_status_for_session(
                            &callback_path,
                            session.vault,
                            SaveStatus::Saved,
                            cx,
                        );
                    }

                    if installed && durable && queued.pending_session != Some(session.vault) {
                        state.mark_vault_session_persisted(session.vault);
                    }
                    if let Some(pending_session) = queued.pending_session {
                        state.request_save_for_session(
                            callback_path.clone(),
                            pending_session,
                            true,
                            cx,
                        );
                        return None;
                    }

                    if durable && state.finish_deferred_lock_if_ready() {
                        cx.notify();
                        return None;
                    }

                    (installed && durable).then(|| receipt.bytes())
                })
                .ok()
                .flatten();

            let Some(published_bytes) = published_bytes else {
                return;
            };

            // Pure fast-forward: the merge only pulled remote-side changes,
            // so the merged DB already matches the server. Skip the upload
            // entirely (no redundant remote version) and just adopt the
            // remote ETag we merged against as our new baseline. Keep-alive
            // isn't lost: the pull that produced these bytes already
            // refreshed the token.
            if !needs_upload {
                let _ = this.update(cx, |state, cx| {
                    if !state.sync_session_is_current(&callback_path, session) {
                        return;
                    }
                    if let Some(config) = state.adopt_fast_forward_baseline(
                        &callback_path,
                        session,
                        if_match.clone(),
                        &published_bytes,
                    ) {
                        // Best effort: a failed write means the next start
                        // re-detects the difference and uploads once.
                        let _ = crate::sync::config::save(&config);
                    }
                    state.apply_sync_status_for_session(
                        &callback_path,
                        session,
                        SyncStatus::Synced {
                            at: chrono::Local::now(),
                            auto_merged,
                        },
                        cx,
                    );
                });
                return;
            }

            // Asked before the write, not after it. The merge itself had to
            // land: those bytes are already on disk, and refusing to install
            // them in memory would leave the two disagreeing. Sending them is
            // a different matter, and the relationship they belong to may
            // have ended while the save was running.
            let still_bound = this
                .update(cx, |state, _| {
                    state.sync_session_is_current(&callback_path, session)
                })
                .unwrap_or(false);
            if !still_bound {
                return;
            }

            // Phase 2: token refresh + upload. If anything in here fails,
            // the in-memory state is already aligned with disk (from phase
            // 1), so the user can dismiss the Failed status and keep
            // working without losing the merge.
            let mut task_config = config.clone();
            task_config.last_etag = if_match;
            let network_task = cx.background_spawn(async move {
                let token = crate::sync::service::ensure_provider_ready(&task_config, token)?;
                let outcome = crate::sync::service::upload_after_save(
                    &task_config,
                    &token,
                    &published_bytes,
                )?;
                let uploaded = crate::sync::config::local_revision(&published_bytes);
                Ok::<_, crate::sync::service::ServiceError>((outcome, token, uploaded))
            });

            let result = network_task.await;
            let _ = this.update(cx, |state, cx| {
                if !state.sync_session_is_current(&callback_path, session) {
                    return;
                }
                match result {
                Ok((outcome, fresh_token, uploaded_revision)) => {
                    state.with_sync_binding_mut_for_session(
                        &callback_path,
                        session,
                        |b| b.access_token = fresh_token,
                    );
                    use crate::sync::service::UploadAfterSave;
                    match outcome {
                        UploadAfterSave::Synced { new_etag, .. } => {
                            if let Some(config) = state.record_upload_baseline(
                                &callback_path,
                                session,
                                new_etag,
                                uploaded_revision,
                            ) {
                                let _ = crate::sync::config::save(&config);
                            }
                            state.apply_sync_status_for_session(
                                &callback_path,
                                session,
                                SyncStatus::Synced {
                                    at: chrono::Local::now(),
                                    auto_merged,
                                },
                                cx,
                            );
                            // Conflict overlay (when one was open) is bound
                            // to the active vault only; close it if the
                            // resolved vault is still active. Parked-vault
                            // merges never opened an overlay, so no-op.
                            if matches!(
                                &state.vault,
                                VaultStatus::Open { path, .. } if path.as_path() == callback_path
                            ) && matches!(state.overlay, Overlay::Conflict)
                            {
                                state.overlay = Overlay::None;
                                cx.notify();
                            }
                        }
                        UploadAfterSave::Conflict {
                            remote_bytes,
                            remote_etag,
                        } => {
                            // Third device wrote during resolution. Re-trigger
                            // the conflict flow against the freshly merged
                            // local + the new remote - for the same vault.
                            state.apply_sync_status_for_session(
                                &callback_path,
                                session,
                                SyncStatus::Syncing,
                                cx,
                            );
                            state.handle_remote_conflict_for(
                                &callback_path,
                                session,
                                remote_bytes,
                                remote_etag,
                                true,
                                cx,
                            );
                        }
                    }
                }
                Err(e) => {
                    // A refresh token that died mid-merge must surface as
                    // Reconnect (with the Azure reason), same as the plain
                    // sync paths - otherwise the user loses the one-click
                    // reconnect affordance and just sees a generic failure.
                    let status = state.note_sync_failure(&callback_path, e);
                    state.apply_sync_status_for_session(
                        &callback_path,
                        session,
                        status,
                        cx,
                    );
                }
            }
            });
        })
        .detach();
    }
}

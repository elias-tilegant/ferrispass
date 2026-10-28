//! Restoring a vault's sync binding after it is opened, the backoff after
//! a throttled push, upload baselines, and pushes that wait for the
//! restore.

use super::*;

impl AppState {
    /// Try to rebuild a `SyncBinding` for the just-opened vault from the
    /// on-disk sync config + the keychain refresh token. Runs in the
    /// background and is a no-op for local-only vaults. On
    /// `InvalidGrant` (refresh token revoked), surfaces
    /// `SyncStatus::Reconnect` so the user is prompted to re-authenticate
    /// via SyncSettings - we don't auto-disconnect, since that would
    /// silently delete their config.
    pub(super) fn try_restore_sync_binding(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let Some(session) = self.sync_session_for(&path) else {
            return;
        };
        // Defensive: if Connect just established a binding (during the
        // pick_kdbx_file → request_password → unlock flow), don't trash it.
        if self.sync.is_some() {
            return;
        }
        // Bail on the common case, a local-only vault, without waiting for
        // anything.
        if known_to_have_no_sync_config(crate::sync::config::config_path_for(&path)) {
            return;
        }

        self.apply_sync_status_for_session(&path, session, SyncStatus::Restoring, cx);

        let read_path = path.clone();
        let task = cx.background_spawn(async move {
            // Waiting out a busy lock belongs here and not on the UI thread:
            // the wait is a second of sleeping, and the caller is a vault
            // that has just been unlocked.
            //
            // An unreadable config is not a local-only vault. Swallowing the
            // error turned one into the other for the rest of the session,
            // silently, and the restore path has a Failed state with a Retry
            // for exactly this.
            let config = crate::sync::lock::retrying(
                || crate::sync::config::load(&read_path),
                crate::sync::config::ConfigError::is_busy,
            )?;
            let Some(mut config) = config else {
                return Ok(None);
            };
            let token = crate::sync::service::restore_provider(&mut config)?;
            let _ = crate::sync::config::save(&config);
            Ok::<_, crate::sync::service::ServiceError>(Some((config, token)))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| match result {
                // The config was there when we looked and gone when we read
                // it: nothing to restore, and nothing to report either.
                Ok(None) => {
                    state.apply_sync_status_for_session(
                        &path,
                        session,
                        SyncStatus::Disconnected,
                        cx,
                    );
                }
                Ok(Some(restored)) => {
                    state.finish_sync_binding_restore(&path, session, Ok(restored), cx);
                }
                Err(error) => {
                    state.finish_sync_binding_restore(&path, session, Err(error), cx);
                }
            });
        })
        .detach();
    }

    /// Completion half of `try_restore_sync_binding`. Splits into a context-
    /// free core plus the two effects that need one, so a test can run the
    /// real callback body with the vault parked.
    fn finish_sync_binding_restore(
        &mut self,
        path: &Path,
        session: SyncSession,
        result: Result<(SyncConfig, AccessToken), crate::sync::service::ServiceError>,
        cx: &mut Context<Self>,
    ) {
        let outcome = self.apply_sync_binding_restore(path, session, result);
        if outcome.repaint {
            cx.notify();
        }
        if outcome.bound {
            self.replay_push_awaiting_restore(path, cx);
        }
    }

    /// The decisions `finish_sync_binding_restore` makes, without a gpui
    /// context.
    ///
    /// Everything routes by session: the keychain refresh can take seconds,
    /// and the user may have parked this vault and unlocked another one
    /// meanwhile. Writing into the *active* slot here would hand this vault's
    /// SharePoint binding to a different vault, whose next save would then
    /// upload its bytes over this vault's remote copy.
    pub(super) fn apply_sync_binding_restore(
        &mut self,
        path: &Path,
        session: SyncSession,
        result: Result<(SyncConfig, AccessToken), crate::sync::service::ServiceError>,
    ) -> RestoreOutcome {
        match result {
            Ok((config, access_token)) => {
                let binding = SyncBinding {
                    config,
                    access_token,
                };
                if !self.rebind_sync_for_session(path, session, binding) {
                    // The vault was locked while the refresh ran. Keychain +
                    // on-disk config are untouched, so the next open restores
                    // cleanly; drop the result.
                    return RestoreOutcome::default();
                }
                // Idle, not Synced: restoring refreshes the token and
                // re-reads the local binding. It never contacts the server,
                // so claiming a fresh sync timestamp here hid pending local
                // edits behind a green pill.
                RestoreOutcome {
                    bound: true,
                    repaint: self.route_sync_status_for_session(path, session, SyncStatus::Idle),
                }
            }
            Err(crate::sync::service::ServiceError::Auth(
                crate::sync::auth::AuthError::InvalidGrant(detail),
            )) => RestoreOutcome {
                bound: false,
                repaint: self.route_sync_status_for_session(
                    path,
                    session,
                    SyncStatus::Reconnect { detail },
                ),
            },
            Err(error) => {
                // Transient (network, etc.) - park in Failed. Note that
                // without a binding `sync_now` bails silently, so nothing
                // retries on its own: the Retry button on the restore card
                // (`retry_sync_restore`) is the way out short of relocking
                // the vault.
                RestoreOutcome {
                    bound: false,
                    repaint: self.route_sync_status_for_session(
                        path,
                        session,
                        SyncStatus::Failed(error.to_string()),
                    ),
                }
            }
        }
    }

    /// Re-run the sync-binding restore for the active vault after a
    /// transient failure (network blip at unlock). Dispatched by the
    /// Retry button on the Sync settings restore card. Safe to call
    /// repeatedly: `try_restore_sync_binding` no-ops when a binding
    /// already exists or no config is on disk.
    /// Classify a sync failure and record a throttle if that is what it was.
    /// Both the plain push and the merge-commit path funnel through here, so
    /// the backoff cannot be armed on one and missed on the other.
    pub(super) fn note_sync_failure(
        &mut self,
        target: &Path,
        error: crate::sync::service::ServiceError,
    ) -> SyncStatus {
        if let Some(retry_after) = error.throttle_retry_after() {
            self.record_sync_throttle(target, retry_after);
            return SyncStatus::Failed(
                "The cloud provider is rate-limiting this account. \
                 Your vault is saved locally and FerrisPass will retry on its own."
                    .into(),
            );
        }
        if let Some(kind) = error.network_kind() {
            // The raw transport text named a URL and a crate rather than
            // anything the user could act on, and the UI used to recover this
            // case by matching on the start of that string. The kind comes
            // from the transport itself now, so the sentence can say which
            // way it failed without guessing.
            return SyncStatus::Failed(format!("{} Your vault is saved locally.", kind.hint()));
        }
        match error {
            crate::sync::service::ServiceError::Auth(
                crate::sync::auth::AuthError::InvalidGrant(detail),
            ) => SyncStatus::Reconnect { detail },
            other => SyncStatus::Failed(other.to_string()),
        }
    }

    /// Note that the provider throttled this vault and compute when it may be
    /// pushed again. Doubles on repeat throttles so a persistent limit is not
    /// met with a fixed-rate retry.
    pub(super) fn record_sync_throttle(&mut self, target: &Path, retry_after: Option<Duration>) {
        let attempts = self
            .sync_backoff
            .get(target)
            .map_or(0, |backoff| backoff.attempts)
            .saturating_add(1);
        let wait = retry_after.unwrap_or_else(|| {
            let doubled =
                SYNC_BACKOFF_MIN.saturating_mul(1u32 << attempts.min(4).saturating_sub(1));
            doubled.min(SYNC_BACKOFF_MAX)
        });
        self.sync_backoff.insert(
            target.to_path_buf(),
            SyncBackoff {
                until: SystemTime::now() + wait,
                attempts,
            },
        );
    }

    /// True while a throttled vault is still inside its backoff window.
    /// Uses wall-clock time on purpose: the provider's limit is wall-clock,
    /// and a sleeping laptop should come back past it, not owing the wait.
    pub(super) fn sync_is_backing_off(&self, target: &Path) -> bool {
        self.sync_backoff
            .get(target)
            .is_some_and(|backoff| SystemTime::now() < backoff.until)
    }

    pub(super) fn clear_sync_backoff(&mut self, target: &Path) {
        self.sync_backoff.remove(target);
    }

    /// True when this vault's file differs from what was last uploaded.
    ///
    /// `last_etag` records what the server held; `uploaded_local_revision`
    /// records what we sent. Only the pair can tell "the remote has not
    /// changed because nobody else wrote" from "the remote has not changed
    /// because our own write never happened".
    ///
    /// Unknown answers `true`. A configuration written before this field
    /// existed cannot say whether the file was ever sent, and answering "in
    /// sync" to a question nothing can answer is how an edit gets stranded
    /// while the pill says Synced. The cost is one upload per vault, once,
    /// after which the recorded revision answers properly.
    pub(super) fn local_is_ahead_of_the_cloud(target: &Path, config: &SyncConfig) -> bool {
        let Some(uploaded) = config.uploaded_local_revision.as_deref() else {
            return true;
        };
        // The file on disk, not the in-memory document: re-encrypting to
        // compare would cost an Argon2 pass per tick, and the saved file is
        // what an upload would send. Unsaved edits are not this function's
        // problem, their save chains a sync of its own.
        match std::fs::read(target) {
            Ok(bytes) => crate::sync::config::local_revision(&bytes) != uploaded,
            // Unreadable means the next push will fail and say so. Guessing
            // "ahead" here would upload nothing and report a failure twice.
            Err(_) => false,
        }
    }

    /// True when a finished save has somewhere to go.
    ///
    /// Not "has a live binding": a vault whose restore has not landed yet is
    /// still a synced vault, and gating on the binding meant such a save never
    /// reached `sync_now_for_path_inner` and so was never even queued. It was
    /// written locally and nothing ever sent it.
    ///
    /// A vault with no configuration at all, and one whose sign-in expired,
    /// are both excluded: there is nothing to push to, and holding the vault
    /// bytes for a push that cannot happen only keeps ciphertext alive.
    pub(super) fn save_should_reach_the_cloud(&self, target: &Path) -> bool {
        self.has_sync_binding_for(target)
            || matches!(
                self.sync_activity_for(target),
                Some(SyncActivity::Busy | SyncActivity::Failed)
            )
    }

    /// Adopt the merged file as this vault's new baseline after a pure
    /// fast-forward. Returns the configuration the caller should persist, or
    /// `None` when the vault was locked meanwhile.
    ///
    /// The file just changed without an upload, and every save re-encrypts
    /// with a fresh seed, so its digest will never match the last one we sent
    /// again. Recording the merged file is what says "local holds nothing the
    /// cloud lacks"; without it the next tick reads the difference as an
    /// unsent edit and uploads after every single pull.
    ///
    /// The disk write is the caller's, so a test can drive the decision
    /// without writing into the user's configuration directory.
    pub(super) fn adopt_fast_forward_baseline(
        &mut self,
        target: &Path,
        session: SyncSession,
        remote_etag: String,
        merged_bytes: &[u8],
    ) -> Option<SyncConfig> {
        let revision = crate::sync::config::local_revision(merged_bytes);
        let mut updated = None;
        self.with_sync_binding_mut_for_session(target, session, |binding| {
            binding.config.last_etag = remote_etag;
            binding.config.uploaded_local_revision = Some(revision);
            updated = Some(binding.config.clone());
        });
        updated
    }

    /// Record what the server now holds and what we sent it.
    ///
    /// The second half is the point: without it the next session cannot tell
    /// "already uploaded" from "saved here and never sent", and a config
    /// written before the field existed reads as ahead, which costs one
    /// upload rather than stranding an edit.
    ///
    /// The disk write is the caller's, so a test can drive the decision
    /// without writing into the user's configuration directory. Both call
    /// sites sit inside spawn closures no test can reach.
    pub(super) fn record_upload_baseline(
        &mut self,
        target: &Path,
        session: SyncSession,
        new_etag: String,
        uploaded_revision: String,
    ) -> Option<SyncConfig> {
        let mut updated = None;
        self.with_sync_binding_mut_for_session(target, session, |binding| {
            binding.config.last_etag = new_etag;
            binding.config.uploaded_local_revision = Some(uploaded_revision);
            updated = Some(binding.config.clone());
        });
        updated
    }

    /// Hold a push until this vault's binding restore lands.
    pub(super) fn queue_push_awaiting_restore(
        &mut self,
        target: &Path,
        request: QueuedSyncRequest,
    ) {
        // Replacing this vault's own entry is the intended coalesce: the
        // newest request carries the newest full snapshot. Other vaults'
        // entries stay, which is the whole point of keying by path.
        self.pushes_awaiting_restore
            .insert(target.to_path_buf(), request);
    }

    /// Claim the push that a missing binding parked for `restored`. Only that
    /// vault's entry is consumed: another vault's queued push is not this
    /// restore's to drop, it waits for its own. A request whose session has
    /// since been locked is discarded rather than returned, so its ciphertext
    /// does not outlive the session that produced it.
    pub(super) fn take_push_awaiting_restore(
        &mut self,
        restored: &Path,
    ) -> Option<QueuedSyncRequest> {
        let request = self.pushes_awaiting_restore.remove(restored)?;
        self.sync_session_is_current(restored, request.session)
            .then_some(request)
    }

    /// Dispatch the push that a missing binding parked for `restored`, now
    /// that one exists.
    fn replay_push_awaiting_restore(&mut self, restored: &Path, cx: &mut Context<Self>) {
        let Some(request) = self.take_push_awaiting_restore(restored) else {
            return;
        };
        self.sync_now_for_path_inner(
            restored,
            request.session,
            request.interactive,
            request.bytes,
            cx,
        );
    }

    pub fn retry_sync_restore(&mut self, cx: &mut Context<Self>) {
        let VaultStatus::Open { path, .. } = &self.vault else {
            return;
        };
        let path = path.clone();
        self.try_restore_sync_binding(path, cx);
    }
}

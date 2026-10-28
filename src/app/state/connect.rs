//! Sync actions: disconnecting, connecting to SharePoint or iCloud, the
//! file picker, publishing to iCloud, and reconnecting.

use super::*;

impl AppState {
    /// Tear down the current sync binding: drop the in-memory token, mark
    /// status as Disconnected, then in the background remove the keychain
    /// entry + sync-config file. UI updates immediately; the reserved cleanup
    /// runs before any newer Connect publication.
    pub fn disconnect_sync(&mut self, cx: &mut Context<Self>) {
        self.connect_operations.reserve_cleanup();
        let current_path = match &self.vault {
            VaultStatus::Open { path, .. } => Some(path.clone()),
            _ => None,
        };
        let config = self
            .sync
            .take()
            .map(|binding| binding.config)
            .or_else(|| self.reconnect_target.take())
            .or_else(|| {
                current_path
                    .as_deref()
                    .and_then(|path| crate::sync::config::load(path).ok().flatten())
            });
        self.end_sync_relationship(current_path.as_deref());
        // Activity log is tied to the connected sync - once the user
        // disconnects, the events refer to a relationship that no
        // longer exists. Clearing avoids stale "Updated from remote"
        // lines hanging around after a fresh Connect.
        self.sync_history.clear();
        cx.notify();
        let operation_gate = self.connect_operations.clone();
        let cleanup = cx.background_spawn(async move {
            let mut failure = None;
            operation_gate.run_reserved_cleanup(|| {
                if let Some(config) = config
                    && let Err(error) = crate::sync::lock::retrying(
                        || crate::sync::service::disconnect(&config),
                        crate::sync::service::ServiceError::is_busy,
                    )
                {
                    failure = Some(error.to_string());
                }
            });
            failure
        });
        cx.spawn(async move |this, cx| {
            // The local binding is gone either way. A refresh token or config
            // file that would not go is worth saying out loud: it was
            // swallowed, and the user had no way to know their token was
            // still in the keychain.
            if let Some(failure) = cleanup.await {
                let _ = this.update(cx, |state, cx| {
                    state.leave_pending_notice(format!(
                        "Disconnected, but some data could not be removed: {failure}"
                    ));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// User-driven *reconnect* for the active vault whose refresh token
    /// expired. Unlike `start_sharepoint_connect`, this does NOT run the
    /// provider-pick / file-picker flow - it reuses the active vault's
    /// existing on-disk `SyncConfig` and only swaps in a fresh token. We
    /// arm `reconnect_target` with the vault's path, open the Connect
    /// overlay straight onto the device-code step, and let the shared poll
    /// loop route the resulting token into `finish_reconnect` (rebind)
    /// instead of `Picking` (new download). No new local file is created.
    pub fn start_sharepoint_reconnect(&mut self, cx: &mut Context<Self>) {
        // Reconnect always targets the active vault - the Settings → Sync
        // card (where the Reconnect button lives) reflects `self.sync` /
        // `self.sync_status`, i.e. the active vault.
        let path = match &self.vault {
            VaultStatus::Open { path, .. } => path.clone(),
            _ => return,
        };
        // Load the existing config up front: it carries the drive/item ids we
        // rebind against, and reading it here lets us fail fast (before making
        // the user sign in) when there's nothing to reconnect. A miss
        // shouldn't happen - the Reconnect button only shows for a vault that
        // had a binding.
        let Ok(Some(config)) = crate::sync::config::load(&path) else {
            self.sync_status =
                SyncStatus::Failed("No cloud sync is configured for this vault.".into());
            cx.notify();
            return;
        };

        self.reconnect_target = Some(config);
        self.open_overlay(Overlay::Connect, cx);
        // Show a spinner (not the provider picker) while the device code is
        // requested - `start_sharepoint_connect` flips this to `SigningIn`
        // once the challenge arrives, then the poll loop's token branch sees
        // `reconnect_target` and rebinds (no file picker, no new download).
        self.connect_flow = Some(ConnectFlow::Authorizing);
        self.sync_status = SyncStatus::Connecting;
        cx.notify();
        self.start_sharepoint_connect(cx);
    }

    /// Step 1 of Connect: request a device code and kick off the polling
    /// loop. UI should observe `connect_flow` transitioning to
    /// `Some(SigningIn { .. })` and switch to the device-code screen.
    /// No URL/path is needed up front - the user picks a file *after*
    /// signing in (see `Picking`).
    pub fn start_sharepoint_connect(&mut self, cx: &mut Context<Self>) {
        if matches!(self.vault, VaultStatus::LockedPendingSave) || self.connect_flow.is_none() {
            return;
        }
        let generation = self.connect_operations.advance();
        let task = cx.background_spawn(async move { crate::sync::service::request_device_code() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| {
                if !state.connect_operations.is_current(generation)
                    || !matches!(
                        state.connect_flow,
                        Some(ConnectFlow::PickProvider | ConnectFlow::Authorizing)
                    )
                {
                    return;
                }
                match result {
                    Ok(challenge) => {
                        state.connect_flow = Some(ConnectFlow::SigningIn {
                            challenge: challenge.clone(),
                        });
                        cx.notify();
                        state.start_token_polling(challenge, generation, cx);
                    }
                    Err(e) => {
                        state.connect_flow = Some(ConnectFlow::Failed(e.to_string()));
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    pub fn start_icloud_connect(&mut self, cx: &mut Context<Self>) {
        if matches!(self.vault, VaultStatus::LockedPendingSave) || self.connect_flow.is_none() {
            return;
        }
        self.connect_operations.advance();
        self.connect_flow = Some(ConnectFlow::ICloudActions);
        self.sync_status = SyncStatus::Connecting;
        cx.notify();
    }

    pub fn can_publish_active_to_icloud(&self) -> bool {
        matches!(self.vault, VaultStatus::Open { .. }) && self.sync.is_none()
    }

    pub fn active_vault_file_name(&self) -> Option<String> {
        match &self.vault {
            VaultStatus::Open { path, .. } => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            _ => None,
        }
    }

    /// Background polling loop. Runs until token received, code expired,
    /// auth declined, or the user cancels (we observe `connect_flow`
    /// transitioning out of `SigningIn` between iterations).
    fn start_token_polling(
        &mut self,
        challenge: DeviceCodeChallenge,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let mut interval = challenge.interval;
            loop {
                // Cooperative cancel: if the user closed Connect (or moved
                // past SigningIn for any other reason), stop polling.
                let still_signing_in = this
                    .update(cx, |s, _| {
                        s.connect_operations.is_current(generation)
                            && matches!(s.connect_flow, Some(ConnectFlow::SigningIn { .. }))
                    })
                    .unwrap_or(false);
                if !still_signing_in {
                    return;
                }

                // Hard timeout: if the device-code expiry passed, give up.
                if std::time::SystemTime::now() > challenge.expires_at {
                    let _ = this.update(cx, |s, cx| {
                        if !s.connect_operations.is_current(generation) {
                            return;
                        }
                        let msg = "Device code expired before sign-in.".to_string();
                        s.connect_flow = Some(ConnectFlow::Failed(msg.clone()));
                        cx.notify();
                    });
                    return;
                }

                cx.background_executor().timer(interval).await;

                let challenge_clone = challenge.clone();
                let outcome = cx
                    .background_spawn(
                        async move { crate::sync::auth::poll_token(&challenge_clone) },
                    )
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
                            if !s.connect_operations.is_current(generation)
                                || !matches!(s.connect_flow, Some(ConnectFlow::SigningIn { .. }))
                            {
                                return;
                            }
                            // Reconnect path: rebind the existing vault's
                            // config with this fresh token instead of opening
                            // the file picker. `take()` disarms the target so
                            // a later plain Connect can't accidentally rebind.
                            if let Some(config) = s.reconnect_target.take() {
                                s.finish_reconnect(config, token, cx);
                                return;
                            }
                            // First-time connect: transition to picker
                            // (loading state); spawn the file-list fetch.
                            s.connect_flow = Some(ConnectFlow::Picking {
                                token: token.clone(),
                                results: Vec::new(),
                                query: String::new(),
                                loading: true,
                                error: None,
                            });
                            cx.notify();
                            s.start_kdbx_search(token, generation, cx);
                        });
                        return;
                    }
                    PollOutcome::Failed(e) => {
                        let msg = e.to_string();
                        let _ = this.update(cx, |s, cx| {
                            if !s.connect_operations.is_current(generation)
                                || !matches!(s.connect_flow, Some(ConnectFlow::SigningIn { .. }))
                            {
                                return;
                            }
                            s.connect_flow = Some(ConnectFlow::Failed(msg.clone()));
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
    fn start_kdbx_search(&mut self, token: AccessToken, generation: u64, cx: &mut Context<Self>) {
        let token_for_task = token.clone();
        let task =
            cx.background_spawn(
                async move { crate::sync::service::list_kdbx_files(&token_for_task) },
            );
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| {
                if !state.connect_operations.is_current(generation) {
                    return;
                }
                if let Some(ConnectFlow::Picking {
                    results,
                    loading,
                    error,
                    ..
                }) = &mut state.connect_flow
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

    /// Live-filter the picker as the user types. Cheap - runs against the
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
        if self.is_unlocked_path(&local_path) {
            self.connect_flow = Some(ConnectFlow::Failed(
                "That local vault is already open. Use the Vaults switcher to switch to it."
                    .to_string(),
            ));
            cx.notify();
            return;
        }
        if self.has_open_sync_remote(&hit.drive_id, &hit.item_id) {
            self.connect_flow = Some(ConnectFlow::Failed(
                "That SharePoint vault is already open. Use the Vaults switcher to switch to it."
                    .to_string(),
            ));
            cx.notify();
            return;
        }
        // The picker holds the access token; capture it before transitioning
        // out of Picking (which drops the token).
        let token = match &self.connect_flow {
            Some(ConnectFlow::Picking { token, .. }) => token.clone(),
            _ => return,
        };
        let generation = self.connect_operations.advance();
        self.connect_flow = Some(ConnectFlow::Downloading);
        cx.notify();

        let operation_gate = self.connect_operations.clone();
        let path_for_task = local_path.clone();
        let task = cx.background_spawn(async move {
            let result = crate::sync::service::prepare_connect_picked(&hit, token, &path_for_task)?;
            let Some(persisted) = operation_gate.commit_if_current(generation, || {
                crate::sync::service::persist_connect_picked(&result)
            }) else {
                return Ok(None);
            };
            persisted?;
            Ok::<_, crate::sync::service::ServiceError>(Some(result))
        });
        let final_path = local_path;
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| {
                let is_current = state.connect_operations.is_current(generation)
                    && matches!(state.connect_flow, Some(ConnectFlow::Downloading));
                match result {
                    Ok(Some(connect_result)) if is_current => {
                        state.connect_operations.advance();
                        state.connect_flow = None;
                        state.overlay = Overlay::None;
                        state.pending_sync = Some(PendingSync {
                            local_path: final_path.clone(),
                            binding: SyncBinding {
                                config: connect_result.config,
                                access_token: connect_result.access_token,
                            },
                        });
                        state.request_password(final_path, cx);
                        cx.notify();
                    }
                    // Publication crossed its commit point just before the
                    // user cancelled or locked. Keep the completed encrypted
                    // vault discoverable, but never restore the stale flow or
                    // retain its access token in AppState.
                    Ok(Some(_)) => state.push_recent(final_path, cx),
                    Ok(None) => {}
                    Err(error) => {
                        // A publication that kept the vault kept it as the
                        // only route to Disconnect. Leaving it out of
                        // Recents hides the one file that leads there, and a
                        // flow the user has already cancelled has no screen
                        // left to say so on.
                        if error.left_a_local_vault() {
                            state.push_recent(final_path, cx);
                        }
                        if is_current {
                            state.connect_flow = Some(ConnectFlow::Failed(error.to_string()));
                            cx.notify();
                        }
                    }
                }
            });
        })
        .detach();
    }

    /// Bind an existing user-selected iCloud Drive file to a new local copy.
    pub fn connect_icloud_existing(
        &mut self,
        remote_path: PathBuf,
        local_path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if self.is_unlocked_path(&local_path) {
            self.connect_flow = Some(ConnectFlow::Failed(
                "That local vault is already open.".into(),
            ));
            cx.notify();
            return;
        }
        if local_path
            .parent()
            .is_some_and(crate::sync::icloud::is_icloud_item)
        {
            self.connect_flow = Some(ConnectFlow::Failed(
                "Choose a local destination outside iCloud Drive; FerrisPass keeps a separate working copy."
                    .into(),
            ));
            cx.notify();
            return;
        }

        let generation = self.connect_operations.advance();
        self.connect_flow = Some(ConnectFlow::ICloudTransferring {
            message: "Downloading the encrypted vault from iCloud Drive…".into(),
        });
        cx.notify();
        let operation_gate = self.connect_operations.clone();
        let task_local_path = local_path.clone();
        let task = cx.background_spawn(async move {
            let result =
                crate::sync::service::prepare_icloud_connect(&remote_path, &task_local_path)?;
            let Some(persisted) = operation_gate.commit_if_current(generation, || {
                crate::sync::service::persist_connect_picked(&result)
            }) else {
                return Ok(None);
            };
            persisted?;
            Ok::<_, crate::sync::service::ServiceError>(Some(result))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| {
                let current = state.connect_operations.is_current(generation)
                    && matches!(
                        state.connect_flow,
                        Some(ConnectFlow::ICloudTransferring { .. })
                    );
                match result {
                    Ok(Some(result)) if current => {
                        state.connect_operations.advance();
                        state.connect_flow = None;
                        state.overlay = Overlay::None;
                        state.pending_sync = Some(PendingSync {
                            local_path: local_path.clone(),
                            binding: SyncBinding {
                                config: result.config,
                                access_token: result.access_token,
                            },
                        });
                        state.request_password(local_path.clone(), cx);
                    }
                    Ok(Some(_)) => state.push_recent(local_path.clone(), cx),
                    Ok(None) => {}
                    Err(error) if current => {
                        state.connect_flow = Some(ConnectFlow::Failed(error.to_string()));
                    }
                    Err(_) => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Copy the currently-open local vault to a new iCloud Drive location
    /// while retaining the local file as the canonical working copy.
    pub fn publish_active_to_icloud(&mut self, remote_path: PathBuf, cx: &mut Context<Self>) {
        let (local_path, session) = match (&self.vault, self.active_vault_session_id) {
            (VaultStatus::Open { path, .. }, Some(vault)) if self.sync.is_none() => (
                path.clone(),
                SyncSession {
                    vault,
                    binding: self.sync_binding_generation_for(path),
                },
            ),
            _ => return,
        };
        if crate::sync::icloud::is_icloud_item(&local_path) {
            self.connect_flow = Some(ConnectFlow::Failed(
                "The open vault already lives in iCloud Drive. Open it through the iCloud provider and choose a separate local working copy."
                    .into(),
            ));
            cx.notify();
            return;
        }
        let Some(reader) = self.current_bytes_reader_for_session(&local_path, session.vault) else {
            return;
        };
        let generation = self.connect_operations.advance();
        self.connect_flow = Some(ConnectFlow::ICloudTransferring {
            message: "Publishing the encrypted vault to iCloud Drive…".into(),
        });
        cx.notify();
        let operation_gate = self.connect_operations.clone();
        let task_local_path = local_path.clone();
        let task = cx.background_spawn(async move {
            let bytes = reader().map_err(|error| {
                crate::sync::service::ServiceError::LocalVault(error.to_string())
            })?;
            let Some(result) = operation_gate.commit_if_current(generation, || {
                crate::sync::service::publish_icloud_binding(&task_local_path, &remote_path, &bytes)
            }) else {
                return Ok(None);
            };
            Ok::<_, crate::sync::service::ServiceError>(Some(result?))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| {
                state.apply_icloud_publish_result(&local_path, session, generation, result);
                cx.notify();
            });
        })
        .detach();
    }

    /// Completion half of `publish_active_to_icloud`, kept out of the spawn
    /// closure so a test can run the real callback body against a state where
    /// the user switched vaults meanwhile.
    ///
    /// Everything routes by session, never into the active slot:
    /// `vault_session_is_current` is also true for a vault the user parked
    /// while the copy ran, and writing `state.sync` here handed this vault's
    /// iCloud binding to whichever vault happened to be on screen. That
    /// vault's next save then uploaded its own bytes over this vault's
    /// remote file.
    pub(super) fn apply_icloud_publish_result(
        &mut self,
        local_path: &Path,
        session: SyncSession,
        generation: u64,
        result: Result<Option<SyncConfig>, crate::sync::service::ServiceError>,
    ) {
        let current = self.connect_operations.is_current(generation)
            && self.sync_session_is_current(local_path, session);
        match result {
            Ok(Some(config)) if current => {
                self.connect_operations.advance();
                let binding = SyncBinding {
                    config,
                    access_token: AccessToken::provider_placeholder(),
                };
                if self.rebind_sync_for_session(local_path, session, binding) {
                    // Idle, not Synced: publishing wrote the file, but no
                    // revision check has run for this session yet.
                    self.route_sync_status_for_session(local_path, session, SyncStatus::Idle);
                }
                // The overlay belongs to the window, so it only closes when
                // this vault is still the one in front of the user.
                if self.is_active_vault(local_path) {
                    self.connect_flow = None;
                    self.overlay = Overlay::None;
                }
            }
            Ok(_) => {}
            Err(error) if current => {
                self.connect_flow = Some(ConnectFlow::Failed(error.to_string()));
                self.route_sync_status_for_session(
                    local_path,
                    session,
                    SyncStatus::Failed(error.to_string()),
                );
            }
            Err(_) => {}
        }
    }

    /// Finish a user-driven reconnect: rebind `config`'s vault with the
    /// freshly-acquired `token` - no new local file, no duplicate binding.
    /// `reconnect_rebind` (account match + keychain store + `authenticated_at`
    /// re-stamp) runs on a background task; the result is installed against
    /// whichever slot still holds the vault, followed by an immediate sync so
    /// the user lands back in a known-good state. Account mismatch / failure
    /// drops back to `Reconnect` so the card stays actionable.
    fn finish_reconnect(&mut self, config: SyncConfig, token: AccessToken, cx: &mut Context<Self>) {
        let path = config.local_path.clone();
        let Some(session) = self.sync_session_for(&path) else {
            return;
        };
        let generation = self.connect_operations.advance();
        // Close the device-code overlay right away - the rebind is quick and
        // headless. The status pill carries the progress from here.
        self.connect_flow = None;
        self.overlay = Overlay::None;
        self.apply_sync_status(&path, SyncStatus::Connecting, cx);

        let operation_gate = self.connect_operations.clone();
        let task = cx.background_spawn(async move {
            let config = crate::sync::service::prepare_reconnect_rebind(config, &token)?;
            let Some(persisted) = operation_gate.commit_if_current(generation, || {
                crate::sync::service::persist_reconnect_rebind(&config, &token)
            }) else {
                return Ok(None);
            };
            persisted?;
            Ok::<_, crate::sync::service::ServiceError>(Some((config, token)))
        });

        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| {
                if !state.connect_operations.is_current(generation)
                    || !state.sync_session_is_current(&path, session)
                {
                    return;
                }
                state.connect_operations.advance();
                match result {
                    Ok(Some((config, access_token))) => {
                        let binding = SyncBinding {
                            config,
                            access_token,
                        };
                        if state.rebind_sync_for_session(&path, session, binding) {
                            // Idle, not Synced: the rebind refreshed a token
                            // and touched no server, so a fresh sync timestamp
                            // here would be a green pill over a pending
                            // upload. The push below is what earns Synced, and
                            // it may queue behind an in-flight one or fail.
                            state.apply_sync_status_for_session(
                                &path,
                                session,
                                SyncStatus::Idle,
                                cx,
                            );
                            // Verify the new grant works and pull anything that
                            // landed remotely while the sign-in was dead.
                            state.sync_now_for_path_inner(&path, session, true, None, cx);
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        // Stay on the Reconnect card (AccountMismatch or transient
                        // failure) so the user can retry or read the reason.
                        state.apply_sync_status_for_session(
                            &path,
                            session,
                            SyncStatus::Reconnect {
                                detail: Some(e.to_string()),
                            },
                            cx,
                        );
                    }
                }
            });
        })
        .detach();
    }
}

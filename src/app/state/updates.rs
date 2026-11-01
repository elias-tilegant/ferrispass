//! The update check and install, and the What's New and About overlays.

use super::*;

impl AppState {
    /// Read-only access to the auto-update flow state. Drives the welcome
    /// banner + Settings → Updates row.
    pub fn update_status(&self) -> &UpdateStatus {
        &self.update_status
    }

    pub fn whats_new_info(&self) -> Option<&UpdateInfo> {
        self.whats_new_info.as_ref()
    }

    pub fn open_whats_new(&mut self, cx: &mut Context<Self>) {
        let Some(info) = self.whats_new_info.clone() else {
            return;
        };
        self.open_overlay(Overlay::WhatsNew { info }, cx);
    }

    pub fn open_about(&mut self, cx: &mut Context<Self>) {
        self.open_overlay(Overlay::About, cx);
    }

    /// Kick off a background update check. No-op when one is already in
    /// flight or a download is running. Transitions:
    ///
    /// - `Idle` (or `Failed`) → `Checking` → `Available(_) | Idle | Failed`
    ///
    /// Mirrors the `try_restore_sync_binding` pattern: blocking I/O on a
    /// background thread, UI mutations bounced back to the main loop via
    /// `cx.spawn` + `entity.update`.
    pub fn start_update_check(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.update_status,
            UpdateStatus::Checking
                | UpdateStatus::Downloading { .. }
                | UpdateStatus::ReadyToRestart(_)
        ) {
            return;
        }
        self.update_status = UpdateStatus::Checking;
        cx.notify();

        let task = cx.background_spawn(async move { crate::update::check() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| {
                state.update_status = match result {
                    Ok(Some(info)) => UpdateStatus::Available(info),
                    Ok(None) => UpdateStatus::Idle,
                    Err(e) => UpdateStatus::Failed(e.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// Download + install whatever update is currently advertised. Caller is
    /// expected to have verified `update_status() == Available(_)` before
    /// calling - we don't pre-check, the underlying library re-fetches the
    /// manifest as part of `download_and_install`.
    ///
    /// Progress is reported via shared atomics: the blocking download
    /// callback writes byte counters from the background thread, and a
    /// foreground poll loop translates them into `UpdateStatus::Downloading
    /// { progress }` updates roughly every 150ms. When the server omits
    /// `Content-Length` we keep `progress` at 0 - the UI then shows an
    /// indeterminate "Downloading…" rather than a fake percentage.
    pub fn install_update(&mut self, cx: &mut Context<Self>) {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        use std::time::Duration;

        if matches!(self.update_status, UpdateStatus::Downloading { .. }) {
            return;
        }
        let UpdateStatus::Available(info) = self.update_status.clone() else {
            return;
        };
        let expected = info.clone();
        self.update_status = UpdateStatus::Downloading {
            info,
            progress: 0.0,
        };
        cx.notify();

        let downloaded = Arc::new(AtomicU64::new(0));
        let total = Arc::new(AtomicU64::new(0));
        let done = Arc::new(AtomicBool::new(false));

        let downloaded_bg = downloaded.clone();
        let total_bg = total.clone();
        let done_bg = done.clone();
        let task = cx.background_spawn(async move {
            let result = crate::update::install(&expected, move |bytes, content_length| {
                downloaded_bg.store(bytes as u64, Ordering::Relaxed);
                if let Some(len) = content_length {
                    total_bg.store(len, Ordering::Relaxed);
                }
            });
            done_bg.store(true, Ordering::Relaxed);
            result
        });

        let downloaded_poll = downloaded.clone();
        let total_poll = total.clone();
        let done_poll = done.clone();
        cx.spawn(async move |this, cx| {
            loop {
                if done_poll.load(Ordering::Relaxed) {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                let bytes = downloaded_poll.load(Ordering::Relaxed);
                let len = total_poll.load(Ordering::Relaxed);
                if let Some(progress) =
                    (len > 0).then(|| (bytes as f32 / len as f32).clamp(0.0, 1.0))
                {
                    let _ = this.update(cx, |state, cx| {
                        if let UpdateStatus::Downloading { info, .. } = &state.update_status {
                            let info = info.clone();
                            state.update_status = UpdateStatus::Downloading { info, progress };
                            cx.notify();
                        }
                    });
                }
            }
        })
        .detach();

        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |state, cx| {
                state.update_status = match result {
                    Ok(info) => {
                        let _ = crate::update::save_pending_whats_new(&info);
                        state.whats_new_info = Some(info.clone());
                        UpdateStatus::ReadyToRestart(info)
                    }
                    Err(e) => UpdateStatus::Failed(e.to_string()),
                };
                cx.notify();
            });
        })
        .detach();
    }
}

//! UI-facing state machine for the update flow. `AppState` holds one of
//! these; the welcome banner and the Settings → Updates row both read it.
//!
//! Lives at the AppState layer (not inside an `Overlay` variant) because
//! the same status drives multiple surfaces in parallel — banner + settings
//! row + future menu-bar indicator — and they all need to stay in sync
//! across `cx.notify()`.

use super::info::UpdateInfo;

#[derive(Debug, Clone, Default, PartialEq)]
pub enum UpdateStatus {
    /// No check has run yet, or the last check returned "up to date".
    #[default]
    Idle,
    /// A background check is in flight. UI shows a subtle spinner in the
    /// Settings → Updates row, no global indicator (don't bother the user
    /// with passive activity).
    Checking,
    /// Server confirmed a newer release exists. UI surfaces the banner +
    /// "Install" button. User-action gates the actual download.
    Available(UpdateInfo),
    /// Download in progress after the user clicked Install. `progress` is
    /// 0.0..=1.0 if the underlying API reports it; otherwise stays at 0.0.
    Downloading { progress: f32 },
    /// New version is in place on disk; the running process needs to
    /// restart for it to take effect. UI prompts "Restart now".
    ReadyToRestart,
    /// Last attempt failed. The string is a human-readable reason; the
    /// flow drops back to `Idle` on the next user-initiated check.
    Failed(String),
}

impl UpdateStatus {
    /// Convenience for "should the welcome banner be shown right now".
    pub fn has_visible_update(&self) -> bool {
        matches!(
            self,
            UpdateStatus::Available(_) | UpdateStatus::Downloading { .. } | UpdateStatus::ReadyToRestart
        )
    }
}

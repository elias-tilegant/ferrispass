//! The `VaultSummary` the header and the sidebar show.

use super::*;

impl AppState {
    pub fn summary(&self) -> VaultSummary {
        let provider = self.sync.as_ref().map(|b| match b.config.provider {
            crate::sync::config::SyncProvider::SharePoint => "SharePoint".to_string(),
            crate::sync::config::SyncProvider::ICloudDrive => "iCloud Drive".to_string(),
        });
        let synced_at = sync_status_label(&self.sync_status);
        // Header dot tone tracks the live sync status - but only matters for
        // an open vault (the sidebar header is the only consumer and only
        // renders when a vault is open). Non-open / error arms stay Neutral.
        let sync_tone = sync_status_tone(&self.sync_status);
        let auto_merged = match &self.sync_status {
            SyncStatus::Synced { auto_merged, .. } if *auto_merged > 0 => Some(*auto_merged),
            _ => None,
        };

        match &self.vault {
            VaultStatus::Empty => VaultSummary {
                title: "No vault open".to_string(),
                subtitle: "Choose a KeePass database to begin.".to_string(),
                status: "Locked".to_string(),
                entries: 0,
                groups: 0,
                is_open: false,
                is_busy: false,
                sync_tone: SyncTone::Neutral,
                provider: None,
                synced_at: None,
                auto_merged: None,
            },
            VaultStatus::AwaitingPassword { path, .. } => VaultSummary {
                title: file_name(path),
                subtitle: path.display().to_string(),
                status: "Password required".to_string(),
                entries: 0,
                groups: 0,
                is_open: false,
                is_busy: false,
                sync_tone: SyncTone::Neutral,
                provider: provider.clone(),
                synced_at: synced_at.clone(),
                auto_merged,
            },
            VaultStatus::Opening { path } => VaultSummary {
                title: file_name(path),
                subtitle: "Decrypting database…".to_string(),
                status: "Opening".to_string(),
                entries: 0,
                groups: 0,
                is_open: false,
                is_busy: true,
                sync_tone: SyncTone::Neutral,
                provider: provider.clone(),
                synced_at: synced_at.clone(),
                auto_merged,
            },
            VaultStatus::Open { path, document, .. } => VaultSummary {
                title: file_name(path),
                subtitle: path.display().to_string(),
                // Derived, like `sync_tone` and `synced_at` right below. This
                // was hard-coded to "Synced" for every open vault, including
                // local-only ones and ones whose last sync had failed.
                status: sync_status_label(&self.sync_status)
                    .unwrap_or_else(|| "Local only".to_string()),
                entries: document.snapshot().entry_count,
                groups: document.snapshot().group_count.saturating_sub(1),
                is_open: true,
                is_busy: false,
                sync_tone,
                provider,
                synced_at,
                auto_merged,
            },
            VaultStatus::LockedPendingSave => {
                let (subtitle, status) = match &self.save_status {
                    SaveStatus::Failed(message) => (
                        format!("Local save failed: {message}"),
                        "Save failed".to_string(),
                    ),
                    _ => (
                        "Vault access is locked while the local save finishes.".to_string(),
                        "Locking".to_string(),
                    ),
                };
                VaultSummary {
                    title: "Vault locked".to_string(),
                    subtitle,
                    status,
                    entries: 0,
                    groups: 0,
                    is_open: false,
                    is_busy: true,
                    sync_tone: SyncTone::Neutral,
                    provider: None,
                    synced_at: None,
                    auto_merged: None,
                }
            }
            VaultStatus::Error { message, path } => VaultSummary {
                title: "Could not open vault".to_string(),
                // The reason, then where. `path` is always `Some` on this
                // path, so preferring it meant the user never saw why.
                subtitle: match path {
                    Some(path) => format!("{message}\n{}", path.display()),
                    None => message.clone(),
                },
                status: "Error".to_string(),
                entries: 0,
                groups: 0,
                is_open: false,
                is_busy: false,
                sync_tone: SyncTone::Neutral,
                provider: None,
                synced_at: None,
                auto_merged: None,
            },
        }
    }
}

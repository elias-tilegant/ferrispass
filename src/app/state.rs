use crate::domain::VaultSnapshot;
use gpui::Context;

#[derive(Debug, Default)]
pub struct AppState {
    vault: VaultStatus,
}

#[derive(Debug, Default)]
pub enum VaultStatus {
    #[default]
    Empty,
    Open(VaultSnapshot),
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultSummary {
    pub title: String,
    pub status: String,
    pub entries: usize,
    pub groups: usize,
    pub is_open: bool,
}

impl AppState {
    pub fn vault_status(&self) -> &VaultStatus {
        &self.vault
    }

    pub fn open_vault(&mut self, snapshot: VaultSnapshot, cx: &mut Context<Self>) {
        self.vault = VaultStatus::Open(snapshot);
        cx.notify();
    }

    pub fn fail_vault_open(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.vault = VaultStatus::Error(message.into());
        cx.notify();
    }

    pub fn close_vault(&mut self, cx: &mut Context<Self>) {
        self.vault = VaultStatus::Empty;
        cx.notify();
    }

    pub fn summary(&self) -> VaultSummary {
        match &self.vault {
            VaultStatus::Empty => VaultSummary {
                title: "No vault open".to_string(),
                status: "Locked".to_string(),
                entries: 0,
                groups: 0,
                is_open: false,
            },
            VaultStatus::Open(snapshot) => VaultSummary {
                title: snapshot.root.name.clone(),
                status: "Open".to_string(),
                entries: snapshot.entry_count,
                groups: snapshot.group_count,
                is_open: true,
            },
            VaultStatus::Error(message) => VaultSummary {
                title: message.clone(),
                status: "Error".to_string(),
                entries: 0,
                groups: 0,
                is_open: false,
            },
        }
    }
}

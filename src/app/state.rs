use crate::domain::{VaultEntry, VaultGroup, VaultSnapshot};
use gpui::Context;
use std::path::{Path, PathBuf};

#[derive(Debug, Default)]
pub struct AppState {
    vault: VaultStatus,
}

#[derive(Debug, Default)]
pub enum VaultStatus {
    #[default]
    Empty,
    AwaitingPassword {
        path: PathBuf,
        error: Option<String>,
    },
    Opening {
        path: PathBuf,
    },
    Open {
        path: PathBuf,
        snapshot: VaultSnapshot,
        selected_group_id: String,
        selected_entry_id: Option<String>,
        search_query: String,
    },
    Error {
        message: String,
        path: Option<PathBuf>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnlockPrompt {
    pub path: PathBuf,
    pub file_name: String,
    pub display_path: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultSummary {
    pub title: String,
    pub subtitle: String,
    pub status: String,
    pub entries: usize,
    pub groups: usize,
    pub is_open: bool,
    pub is_busy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultBrowserModel {
    pub root: VaultGroup,
    pub selected_group_id: String,
    pub selected_group_name: String,
    pub selected_entry_id: Option<String>,
    pub entries: Vec<VaultEntry>,
    pub selected_entry: Option<VaultEntry>,
    pub search_query: String,
    pub showing_search_results: bool,
}

impl AppState {
    pub fn vault_status(&self) -> &VaultStatus {
        &self.vault
    }

    pub fn request_password(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.vault = VaultStatus::AwaitingPassword { path, error: None };
        cx.notify();
    }

    pub fn set_unlock_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        if let VaultStatus::AwaitingPassword { error, .. } = &mut self.vault {
            *error = Some(message.into());
            cx.notify();
        }
    }

    pub fn begin_open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.vault = VaultStatus::Opening { path };
        cx.notify();
    }

    pub fn finish_open_attempt(
        &mut self,
        path: PathBuf,
        result: Result<VaultSnapshot, String>,
        cx: &mut Context<Self>,
    ) {
        if !matches!(&self.vault, VaultStatus::Opening { path: active } if active == &path) {
            return;
        }

        self.vault = match result {
            Ok(snapshot) => {
                let selected_group_id = snapshot.root.id.clone();
                let selected_entry_id = snapshot.root.entries.first().map(|entry| entry.id.clone());

                VaultStatus::Open {
                    path,
                    snapshot,
                    selected_group_id,
                    selected_entry_id,
                    search_query: String::new(),
                }
            }
            Err(message) => VaultStatus::AwaitingPassword {
                path,
                error: Some(message),
            },
        };
        cx.notify();
    }

    pub fn fail_vault_selection(
        &mut self,
        path: Option<PathBuf>,
        message: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        self.vault = VaultStatus::Error {
            message: message.into(),
            path,
        };
        cx.notify();
    }

    pub fn lock_vault(&mut self, cx: &mut Context<Self>) {
        self.vault = VaultStatus::Empty;
        cx.notify();
    }

    pub fn pending_unlock_path(&self) -> Option<PathBuf> {
        match &self.vault {
            VaultStatus::AwaitingPassword { path, .. } => Some(path.clone()),
            _ => None,
        }
    }

    pub fn unlock_prompt(&self) -> Option<UnlockPrompt> {
        match &self.vault {
            VaultStatus::AwaitingPassword { path, error } => Some(UnlockPrompt {
                path: path.clone(),
                file_name: file_name(path),
                display_path: path.display().to_string(),
                error: error.clone(),
            }),
            _ => None,
        }
    }

    pub fn select_group(&mut self, group_id: impl Into<String>, cx: &mut Context<Self>) {
        let group_id = group_id.into();

        let VaultStatus::Open {
            snapshot,
            selected_group_id,
            selected_entry_id,
            search_query,
            ..
        } = &mut self.vault
        else {
            return;
        };

        let Some(group) = snapshot.find_group(&group_id) else {
            return;
        };

        *selected_group_id = group_id;
        *selected_entry_id = group.entries.first().map(|entry| entry.id.clone());
        search_query.clear();
        cx.notify();
    }

    pub fn select_entry(&mut self, entry_id: impl Into<String>, cx: &mut Context<Self>) {
        let entry_id = entry_id.into();

        let VaultStatus::Open {
            snapshot,
            selected_entry_id,
            ..
        } = &mut self.vault
        else {
            return;
        };

        if snapshot.find_entry(&entry_id).is_some() {
            *selected_entry_id = Some(entry_id);
            cx.notify();
        }
    }

    pub fn vault_browser(&self) -> Option<VaultBrowserModel> {
        let VaultStatus::Open {
            snapshot,
            selected_group_id,
            selected_entry_id,
            search_query,
            ..
        } = &self.vault
        else {
            return None;
        };

        let selected_group = snapshot
            .find_group(selected_group_id)
            .unwrap_or(&snapshot.root);
        let showing_search_results = !search_query.trim().is_empty();

        let entries = if showing_search_results {
            let query = search_query.to_lowercase();
            snapshot
                .entries_recursive()
                .into_iter()
                .filter(|entry| {
                    entry.title.to_lowercase().contains(&query)
                        || entry.username.to_lowercase().contains(&query)
                        || entry.url.to_lowercase().contains(&query)
                })
                .cloned()
                .collect()
        } else {
            selected_group.entries.clone()
        };

        let selected_entry = selected_entry_id
            .as_deref()
            .and_then(|id| snapshot.find_entry(id))
            .cloned()
            .or_else(|| entries.first().cloned());

        Some(VaultBrowserModel {
            root: snapshot.root.clone(),
            selected_group_id: selected_group.id.clone(),
            selected_group_name: selected_group.name.clone(),
            selected_entry_id: selected_entry.as_ref().map(|entry| entry.id.clone()),
            entries,
            selected_entry,
            search_query: search_query.clone(),
            showing_search_results,
        })
    }

    pub fn summary(&self) -> VaultSummary {
        match &self.vault {
            VaultStatus::Empty => VaultSummary {
                title: "No vault open".to_string(),
                subtitle: "Choose a KeePass database to begin.".to_string(),
                status: "Locked".to_string(),
                entries: 0,
                groups: 0,
                is_open: false,
                is_busy: false,
            },
            VaultStatus::AwaitingPassword { path, .. } => VaultSummary {
                title: file_name(path),
                subtitle: path.display().to_string(),
                status: "Password required".to_string(),
                entries: 0,
                groups: 0,
                is_open: false,
                is_busy: false,
            },
            VaultStatus::Opening { path } => VaultSummary {
                title: file_name(path),
                subtitle: "Decrypting database...".to_string(),
                status: "Opening".to_string(),
                entries: 0,
                groups: 0,
                is_open: false,
                is_busy: true,
            },
            VaultStatus::Open { path, snapshot, .. } => VaultSummary {
                title: snapshot.root.name.clone(),
                subtitle: path.display().to_string(),
                status: "Open".to_string(),
                entries: snapshot.entry_count,
                groups: snapshot.group_count,
                is_open: true,
                is_busy: false,
            },
            VaultStatus::Error { message, path } => VaultSummary {
                title: "Could not open vault".to_string(),
                subtitle: path
                    .as_ref()
                    .map_or_else(|| message.clone(), |path| path.display().to_string()),
                status: "Error".to_string(),
                entries: 0,
                groups: 0,
                is_open: false,
                is_busy: false,
            },
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map_or_else(|| path.display().to_string(), ToString::to_string)
}

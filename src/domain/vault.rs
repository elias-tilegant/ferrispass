#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VaultSnapshot {
    pub root: VaultGroup,
    pub entry_count: usize,
    pub group_count: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VaultGroup {
    pub id: String,
    pub name: String,
    pub groups: Vec<VaultGroup>,
    pub entries: Vec<VaultEntry>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VaultEntry {
    pub id: String,
    pub title: String,
    pub username: String,
    pub url: String,
    pub has_password: bool,
}

impl VaultSnapshot {
    pub fn new(root: VaultGroup) -> Self {
        Self {
            entry_count: root.entry_count(),
            group_count: root.group_count(),
            root,
        }
    }
}

impl VaultGroup {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        groups: Vec<VaultGroup>,
        entries: Vec<VaultEntry>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            groups,
            entries,
        }
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
            + self
                .groups
                .iter()
                .map(VaultGroup::entry_count)
                .sum::<usize>()
    }

    pub fn group_count(&self) -> usize {
        1 + self
            .groups
            .iter()
            .map(VaultGroup::group_count)
            .sum::<usize>()
    }
}

impl VaultEntry {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        username: impl Into<String>,
        url: impl Into<String>,
        has_password: bool,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            username: username.into(),
            url: url.into(),
            has_password,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{VaultEntry, VaultGroup, VaultSnapshot};

    #[test]
    fn snapshot_counts_nested_entries_and_groups() {
        let root = VaultGroup::new(
            "root",
            "Root",
            vec![VaultGroup::new(
                "work",
                "Work",
                Vec::new(),
                vec![VaultEntry::new("entry-2", "Git", "elias", "", true)],
            )],
            vec![VaultEntry::new("entry-1", "Mail", "elias", "", true)],
        );

        let snapshot = VaultSnapshot::new(root);

        assert_eq!(snapshot.entry_count, 2);
        assert_eq!(snapshot.group_count, 2);
    }
}

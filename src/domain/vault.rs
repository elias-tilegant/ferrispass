#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VaultSnapshot {
    pub root: VaultGroup,
    pub entry_count: usize,
    pub group_count: usize,
    /// Group id of the database's Recycle Bin, if one exists. Used by the
    /// Trash sidebar to surface deleted entries and by the detail panel to
    /// branch the action footer (Restore / Delete forever vs. Edit / Delete).
    pub recycle_bin_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultGroup {
    pub id: String,
    pub name: String,
    pub groups: Vec<VaultGroup>,
    pub entries: Vec<VaultEntry>,
    /// Mirrors the KeePass `IsExpanded` flag — round-trips through every
    /// other client (KeePassXC, KeePass2) so the user's collapse state
    /// in our sidebar follows them across machines via sync. Defaults to
    /// `true` for groups we synthesize ourselves (test fixtures, fresh
    /// vaults) so the tree opens up by default.
    pub is_expanded: bool,
    /// Custom-icon bytes pulled from the KeePass `custom_icons` table
    /// when the group has `Icon::Custom(_)`. Same shape as
    /// `VaultEntry::favicon.image` — we reuse the `FaviconImage` newtype
    /// because it's just decoded image bytes ready for `gpui::img()`,
    /// regardless of whether the source is an entry or a group.
    /// `None` for groups using a built-in icon or no icon at all.
    pub icon: Option<FaviconImage>,
}

impl Default for VaultGroup {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            groups: Vec::new(),
            entries: Vec::new(),
            is_expanded: true,
            icon: None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VaultEntry {
    pub id: String,
    pub title: String,
    pub username: String,
    pub url: String,
    pub notes: String,
    pub has_password: bool,
    pub password_length: usize,
    pub has_otp: bool,
    pub updated: Option<String>,
    pub tags: Vec<String>,
    pub starred: bool,
    pub favicon: Favicon,
    pub strength: Strength,
    pub group_path: Vec<String>,
    /// `true` when this entry sits inside the Recycle Bin group. Lets the UI
    /// swap the action footer (Restore + Delete forever) without having to
    /// re-walk the group tree per render.
    pub in_recycle_bin: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Favicon {
    pub letter: String,
    pub palette_index: u8,
    /// Custom icon bytes pulled from the KeePass database's
    /// `custom_icons` table when the entry has `Icon::Custom(_)`. The
    /// renderer prefers this over the synthesized letter when present.
    pub image: Option<FaviconImage>,
}

/// Decoded, format-tagged custom icon ready to hand to GPUI's `img()`.
/// Wrapped in `Arc` so cloning a `VaultEntry` (visible-list cache, drag
/// previews, render snapshots) is a refcount bump, and so the GPUI image
/// cache — keyed off the inner `Image::id` (hash of bytes) — can dedupe
/// across re-renders without us rebuilding the wrapper each frame.
#[derive(Clone, Debug)]
pub struct FaviconImage(pub std::sync::Arc<gpui::Image>);

impl PartialEq for FaviconImage {
    fn eq(&self, other: &Self) -> bool {
        self.0.id() == other.0.id()
    }
}

impl Eq for FaviconImage {}

impl Default for Favicon {
    fn default() -> Self {
        Self {
            letter: "·".to_string(),
            palette_index: 0,
            image: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Strength {
    Weak,
    Fair,
    #[default]
    Strong,
}

impl Strength {
    pub fn label(self) -> &'static str {
        match self {
            Strength::Weak => "Weak",
            Strength::Fair => "Fair",
            Strength::Strong => "Strong",
        }
    }

    pub fn fill_segments(self, total: usize) -> usize {
        match self {
            Strength::Weak => (total / 3).max(1),
            Strength::Fair => (total * 2 / 3).max(2),
            Strength::Strong => total.saturating_sub(1).max(1),
        }
    }

    pub fn from_password_length(length: usize) -> Self {
        match length {
            0..=7 => Strength::Weak,
            8..=11 => Strength::Fair,
            _ => Strength::Strong,
        }
    }
}

impl VaultSnapshot {
    pub fn new(root: VaultGroup) -> Self {
        Self {
            entry_count: root.entry_count(),
            group_count: root.group_count(),
            root,
            recycle_bin_id: None,
        }
    }

    pub fn find_group(&self, id: &str) -> Option<&VaultGroup> {
        self.root.find_group(id)
    }

    pub fn find_entry(&self, id: &str) -> Option<&VaultEntry> {
        self.root.find_entry(id)
    }

    pub fn entries_recursive(&self) -> Vec<&VaultEntry> {
        self.root.entries_recursive()
    }

    pub fn entries_starred(&self) -> Vec<&VaultEntry> {
        self.entries_recursive()
            .into_iter()
            .filter(|entry| entry.starred)
            .collect()
    }

    pub fn entries_with_tag(&self, tag: &str) -> Vec<&VaultEntry> {
        self.entries_recursive()
            .into_iter()
            .filter(|entry| entry.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)))
            .collect()
    }

    /// Entries that have a TOTP secret configured. Drives the sidebar's
    /// "2FA enabled" filter — derived from the real `has_otp` bit, not
    /// from a tag, so it stays accurate regardless of how the user
    /// (or another KeePass client) labels their entries.
    pub fn entries_with_otp(&self) -> Vec<&VaultEntry> {
        self.entries_recursive()
            .into_iter()
            .filter(|entry| entry.has_otp)
            .collect()
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
            is_expanded: true,
            icon: None,
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

    pub fn find_group(&self, id: &str) -> Option<&VaultGroup> {
        if self.id == id {
            return Some(self);
        }

        self.groups.iter().find_map(|group| group.find_group(id))
    }

    pub fn find_entry(&self, id: &str) -> Option<&VaultEntry> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .or_else(|| self.groups.iter().find_map(|group| group.find_entry(id)))
    }

    pub fn entries_recursive(&self) -> Vec<&VaultEntry> {
        let mut entries = Vec::new();
        self.collect_entries(&mut entries);
        entries
    }

    fn collect_entries<'a>(&'a self, entries: &mut Vec<&'a VaultEntry>) {
        entries.extend(self.entries.iter());

        for group in &self.groups {
            group.collect_entries(entries);
        }
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
            notes: String::new(),
            has_password,
            password_length: 0,
            has_otp: false,
            updated: None,
            tags: Vec::new(),
            starred: false,
            favicon: Favicon::default(),
            strength: Strength::default(),
            group_path: Vec::new(),
            in_recycle_bin: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Favicon, Strength, VaultEntry, VaultGroup, VaultSnapshot};

    fn entry(id: &str, title: &str) -> VaultEntry {
        VaultEntry::new(id, title, "alice", "", true)
    }

    #[test]
    fn snapshot_counts_nested_entries_and_groups() {
        let root = VaultGroup::new(
            "root",
            "Root",
            vec![VaultGroup::new(
                "work",
                "Work",
                Vec::new(),
                vec![entry("entry-2", "Git")],
            )],
            vec![entry("entry-1", "Mail")],
        );

        let snapshot = VaultSnapshot::new(root);

        assert_eq!(snapshot.entry_count, 2);
        assert_eq!(snapshot.group_count, 2);
    }

    #[test]
    fn finds_nested_groups_and_entries() {
        let root = VaultGroup::new(
            "root",
            "Root",
            vec![VaultGroup::new(
                "work",
                "Work",
                Vec::new(),
                vec![entry("entry-2", "Git")],
            )],
            vec![entry("entry-1", "Mail")],
        );

        let snapshot = VaultSnapshot::new(root);

        assert_eq!(
            snapshot.find_group("work").map(|group| group.name.as_str()),
            Some("Work")
        );
        assert_eq!(
            snapshot
                .find_entry("entry-2")
                .map(|entry| entry.title.as_str()),
            Some("Git")
        );
        assert_eq!(snapshot.entries_recursive().len(), 2);
    }

    #[test]
    fn starred_entries_collected() {
        let mut starred = entry("entry-1", "Mail");
        starred.starred = true;
        let root = VaultGroup::new(
            "root",
            "Root",
            Vec::new(),
            vec![starred, entry("entry-2", "Git")],
        );
        let snapshot = VaultSnapshot::new(root);

        let pinned = snapshot.entries_starred();
        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].title, "Mail");
    }

    #[test]
    fn strength_thresholds() {
        assert_eq!(Strength::from_password_length(0), Strength::Weak);
        assert_eq!(Strength::from_password_length(8), Strength::Fair);
        assert_eq!(Strength::from_password_length(20), Strength::Strong);
    }

    #[test]
    fn favicon_default_letter_is_dot() {
        let e = entry("e", "Mail");
        assert_eq!(e.favicon, Favicon::default());
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct VaultSnapshot {
    pub root: VaultGroup,
    pub entry_count: usize,
    pub group_count: usize,
    /// Group id of the database's Recycle Bin, if one exists. Used by the
    /// Trash sidebar to surface deleted entries and by the detail panel to
    /// branch the action footer (Restore / Delete forever vs. Edit / Delete).
    pub recycle_bin_id: Option<String>,
}

impl std::fmt::Debug for VaultSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VaultSnapshot")
            .field("entry_count", &self.entry_count)
            .field("group_count", &self.group_count)
            .field("has_recycle_bin", &self.recycle_bin_id.is_some())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct VaultGroup {
    pub id: String,
    pub name: String,
    pub groups: Vec<VaultGroup>,
    pub entries: Vec<VaultEntry>,
    /// Mirrors the KeePass `IsExpanded` flag - round-trips through every
    /// other client (KeePassXC, KeePass2) so the user's collapse state
    /// in our sidebar follows them across machines via sync. Defaults to
    /// `true` for groups we synthesize ourselves (test fixtures, fresh
    /// vaults) so the tree opens up by default.
    pub is_expanded: bool,
    /// Custom-icon bytes pulled from the KeePass `custom_icons` table
    /// when the group has `Icon::Custom(_)`. Same shape as
    /// `VaultEntry::favicon.image` - we reuse the `FaviconImage` newtype
    /// because it's just decoded image bytes ready for `gpui::img()`,
    /// regardless of whether the source is an entry or a group.
    /// `None` for groups using a built-in icon or no icon at all.
    pub icon: Option<FaviconImage>,
    /// True for the Recycle Bin itself and for every group below it.
    /// KeePass deletes a group by moving the whole subtree into the bin, so
    /// this has to be inherited: without it a group two levels down reads as
    /// live and its entries are offered for editing instead of restoring.
    pub in_recycle_bin: bool,
}

impl std::fmt::Debug for VaultGroup {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VaultGroup")
            .field("id", &self.id)
            .field("name_chars", &self.name.chars().count())
            .field("group_count", &self.groups.len())
            .field("entry_count", &self.entries.len())
            .field("is_expanded", &self.is_expanded)
            .field("has_icon", &self.icon.is_some())
            .field("in_recycle_bin", &self.in_recycle_bin)
            .finish()
    }
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
            in_recycle_bin: false,
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
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
    /// KeePass `AutoType/Enabled` - `false` excludes the entry from hotkey
    /// matching entirely (KeePass semantics; explicit in-app typing of a
    /// selected entry is unaffected).
    pub auto_type_enabled: bool,
    /// User-authored KeePass `AutoType/Association/Window` patterns
    /// (`*`/`?` wildcards). An explicit association is a trustworthy hotkey
    /// match signal precisely because the user wrote the pattern themselves.
    pub auto_type_windows: Vec<String>,
    /// Arbitrary key-value pairs stored on the KeePass entry beyond the
    /// six standard fields (Title/UserName/Password/URL/Notes/otp). Used
    /// by KeePassXC's "Additional attributes" UI and by our launcher
    /// detection (`SAP_CONN`, etc.). Cleartext in the snapshot - same
    /// trust zone as the cleartext password - but the `protected` flag
    /// is preserved so we can re-write via `set_protected` on save and
    /// mask the value in any read-only display.
    pub custom_fields: Vec<CustomField>,
}

impl std::fmt::Debug for VaultEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VaultEntry")
            .field("id", &self.id)
            .field("title_chars", &self.title.chars().count())
            .field("has_username", &!self.username.is_empty())
            .field("has_url", &!self.url.is_empty())
            .field("has_notes", &!self.notes.is_empty())
            .field("has_password", &self.has_password)
            .field("password_length", &self.password_length)
            .field("has_otp", &self.has_otp)
            .field("tag_count", &self.tags.len())
            .field("starred", &self.starred)
            .field("strength", &self.strength)
            .field("group_depth", &self.group_path.len())
            .field("in_recycle_bin", &self.in_recycle_bin)
            .field("custom_field_count", &self.custom_fields.len())
            .finish()
    }
}

/// One non-standard attribute on a KeePass entry. `protected` mirrors the
/// `Protected="True"` XML attribute - KeePassXC writes secrets (e.g.
/// alternate passwords) with this flag and we must round-trip it so
/// nothing silently downgrades from secret to plain on save.
/// One row of the sidebar's tag list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TagRow {
    /// Lowercased spelling. Selection and the row's element id use this, so
    /// neither moves when a differently-cased spelling appears or leaves.
    pub key: String,
    /// What to show. Follows the vault's own spelling and may change.
    pub label: String,
    /// How many live entries carry this tag, counted once per entry.
    pub entry_count: usize,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct CustomField {
    pub key: String,
    pub value: String,
    pub protected: bool,
}

impl std::fmt::Debug for CustomField {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CustomField")
            .field("key_chars", &self.key.chars().count())
            .field("has_value", &!self.value.is_empty())
            .field("protected", &self.protected)
            .finish()
    }
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
/// cache - keyed off the inner `Image::id` (hash of bytes) - can dedupe
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

    /// Every entry in the vault, deleted ones included. Only the CLI's
    /// `--include-trash` and the merge diff want this; user-facing lists use
    /// [`Self::live_entries`].
    pub fn entries_recursive(&self) -> Vec<&VaultEntry> {
        self.root.entries_recursive()
    }

    pub fn live_entries(&self) -> Vec<&VaultEntry> {
        self.root.live_entries_recursive()
    }

    /// Everything the Recycle Bin holds, at any depth.
    pub fn trashed_entries(&self) -> Vec<&VaultEntry> {
        self.root.trashed_entries_recursive()
    }

    /// Groups sitting directly inside the Recycle Bin: the units a user
    /// restores. Their subgroups travel with them, so they are not listed
    /// separately.
    pub fn trashed_groups(&self) -> &[VaultGroup] {
        self.recycle_bin_id
            .as_deref()
            .and_then(|id| self.root.find_group(id))
            .map_or(&[], |bin| bin.groups.as_slice())
    }

    pub fn entries_starred(&self) -> Vec<&VaultEntry> {
        self.live_entries()
            .into_iter()
            .filter(|entry| entry.starred)
            .collect()
    }

    pub fn entries_with_tag(&self, tag: &str) -> Vec<&VaultEntry> {
        self.live_entries()
            .into_iter()
            .filter(|entry| entry.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)))
            .collect()
    }

    /// Every tag the live entries actually carry, with how many use it,
    /// alphabetical. Excludes the `Favorite` marker, which the star and the
    /// Favorites view already represent.
    ///
    /// The sidebar used to render a hard-coded "Personal" and "Work"
    /// regardless of the vault, so clicking either in a vault that had never
    /// heard of them produced an empty list with no explanation.
    ///
    /// Grouped case-insensitively to match `entries_with_tag`: with
    /// case-sensitive rows, "Work" and "work" appeared as two entries whose
    /// counts were each too low, and clicking either one selected both sets.
    ///
    /// Each row carries the lowercase key it was grouped by, which is what
    /// selection and the row's identity use, and a label to display. The
    /// label is the alphabetically first spelling in the vault and can
    /// therefore change as entries come and go; the key it is shown under
    /// cannot, so a selected tag stays selected.
    ///
    /// Counts are of entries, not of occurrences. An entry tagged both "Work"
    /// and "work" is one entry, and the count has to agree with what
    /// selecting the row lists.
    pub fn tags(&self) -> Vec<TagRow> {
        let mut rows: std::collections::BTreeMap<String, TagRow> =
            std::collections::BTreeMap::new();
        for entry in self.live_entries() {
            let mut counted: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
            for tag in &entry.tags {
                if tag.eq_ignore_ascii_case(crate::keepass::FAVORITE_TAG) {
                    continue;
                }
                let key = tag.to_lowercase();
                let row = rows.entry(key.clone()).or_insert_with(|| TagRow {
                    key,
                    label: tag.clone(),
                    entry_count: 0,
                });
                if tag < &row.label {
                    row.label = tag.clone();
                }
                if counted.insert(tag.to_lowercase()) {
                    row.entry_count += 1;
                }
            }
        }
        rows.into_values().collect()
    }

    /// Entries that have a TOTP secret configured. Drives the sidebar's
    /// "2FA enabled" filter - derived from the real `has_otp` bit, not
    /// from a tag, so it stays accurate regardless of how the user
    /// (or another KeePass client) labels their entries.
    pub fn entries_with_otp(&self) -> Vec<&VaultEntry> {
        self.live_entries()
            .into_iter()
            .filter(|entry| entry.has_otp)
            .collect()
    }

    /// Walk the tree once and return the counts the sidebar header
    /// chips render every frame. Replaces calling `entries_starred().len()`
    /// + `entries_with_otp().len()` from the renderer, which allocated
    ///
    /// two `Vec<&VaultEntry>`s and walked the tree twice on every tick.
    pub fn library_counts(&self) -> LibraryCounts {
        fn walk(group: &VaultGroup, counts: &mut LibraryCounts) {
            for entry in &group.entries {
                if entry.in_recycle_bin {
                    continue;
                }
                if entry.starred {
                    counts.starred += 1;
                }
                if entry.has_otp {
                    counts.with_otp += 1;
                }
            }
            for child in &group.groups {
                if !child.in_recycle_bin {
                    walk(child, counts);
                }
            }
        }
        let mut counts = LibraryCounts::default();
        walk(&self.root, &mut counts);
        counts
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LibraryCounts {
    pub starred: usize,
    pub with_otp: usize,
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
            in_recycle_bin: false,
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

    /// Every entry below this group, deleted ones included. Callers that
    /// render a user-facing list want [`Self::live_entries_recursive`] or
    /// [`Self::trashed_entries_recursive`] instead.
    pub fn entries_recursive(&self) -> Vec<&VaultEntry> {
        let mut entries = Vec::new();
        self.collect_entries(&mut entries);
        entries
    }

    /// Every entry below this group that is not in the Recycle Bin.
    pub fn live_entries_recursive(&self) -> Vec<&VaultEntry> {
        let mut entries = Vec::new();
        self.collect_entries_in_bin(false, &mut entries);
        entries
    }

    /// Every entry below this group that is in the Recycle Bin, however
    /// deeply nested. The Trash view needs the whole subtree: deleting a
    /// group moves its entries down one level, out of the bin's direct
    /// children.
    pub fn trashed_entries_recursive(&self) -> Vec<&VaultEntry> {
        let mut entries = Vec::new();
        self.collect_entries_in_bin(true, &mut entries);
        entries
    }

    fn collect_entries<'a>(&'a self, entries: &mut Vec<&'a VaultEntry>) {
        entries.extend(self.entries.iter());

        for group in &self.groups {
            group.collect_entries(entries);
        }
    }

    fn collect_entries_in_bin<'a>(&'a self, want_trashed: bool, out: &mut Vec<&'a VaultEntry>) {
        out.extend(
            self.entries
                .iter()
                .filter(|entry| entry.in_recycle_bin == want_trashed),
        );
        for group in &self.groups {
            // Collecting live entries can prune the bin subtree outright.
            // Collecting trashed ones cannot prune anything: the bin is
            // reachable only through live ancestors.
            if want_trashed || !group.in_recycle_bin {
                group.collect_entries_in_bin(want_trashed, out);
            }
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
            auto_type_enabled: true,
            auto_type_windows: Vec::new(),
            custom_fields: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CustomField, Favicon, Strength, TagRow, VaultEntry, VaultGroup, VaultSnapshot};

    fn entry(id: &str, title: &str) -> VaultEntry {
        VaultEntry::new(id, title, "alice", "", true)
    }

    /// The sidebar rendered a hard-coded "Personal" and "Work" regardless of
    /// what the vault held, so clicking either in a vault that had never used
    /// them produced an empty list with no explanation.
    #[test]
    fn tags_come_from_the_vault_with_their_counts() {
        let mut work = entry("a", "Bank");
        work.tags = vec!["Work".into(), "Favorite".into()];
        let mut also_work = entry("b", "Payroll");
        also_work.tags = vec!["Work".into()];
        let mut archive = entry("c", "Old");
        archive.tags = vec!["Archive".into()];
        archive.in_recycle_bin = true;

        let root = VaultGroup {
            id: "root".into(),
            name: "Root".into(),
            entries: vec![work, also_work, archive],
            ..VaultGroup::default()
        };
        let snapshot = VaultSnapshot::new(root);

        assert_eq!(
            snapshot.tags(),
            vec![TagRow {
                key: "work".into(),
                label: "Work".into(),
                entry_count: 2,
            }],
            "counted, alphabetical, without the Favorite marker the star owns, \
             and without tags only a deleted entry carries"
        );
    }

    /// Tag selection has always matched case-insensitively, so listing rows
    /// case-sensitively produced two sidebar entries for one tag, each with
    /// too low a count, and either row then selected both sets.
    #[test]
    fn tags_spelled_differently_are_one_row() {
        let mut upper = entry("a", "Bank");
        upper.tags = vec!["Work".into()];
        let mut lower = entry("b", "Payroll");
        lower.tags = vec!["work".into()];
        let mut mixed = entry("c", "Travel");
        mixed.tags = vec!["WoRk".into()];

        let root = VaultGroup {
            id: "root".into(),
            name: "Root".into(),
            entries: vec![upper, lower, mixed],
            ..VaultGroup::default()
        };
        let snapshot = VaultSnapshot::new(root);

        assert_eq!(
            snapshot.tags(),
            vec![TagRow {
                key: "work".into(),
                label: "WoRk".into(),
                entry_count: 3,
            }]
        );
        assert_eq!(
            snapshot.entries_with_tag("WoRk").len(),
            3,
            "the row's count matches what selecting it shows"
        );
    }

    /// The count is of entries, not of tag occurrences. An entry carrying two
    /// spellings of one tag is one entry, and selecting the row lists it once.
    #[test]
    fn an_entry_carrying_two_spellings_counts_once() {
        let mut both = entry("a", "Bank");
        both.tags = vec!["Work".into(), "work".into()];
        let mut one = entry("b", "Payroll");
        one.tags = vec!["WORK".into()];

        let root = VaultGroup {
            id: "root".into(),
            name: "Root".into(),
            entries: vec![both, one],
            ..VaultGroup::default()
        };
        let snapshot = VaultSnapshot::new(root);

        let rows = snapshot.tags();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].entry_count, 2);
        assert_eq!(
            snapshot.entries_with_tag(&rows[0].key).len(),
            rows[0].entry_count,
            "the count has to agree with what selecting the row lists"
        );
    }

    /// The row's identity is the lowercase key, not the label. The label
    /// follows the vault's own spelling, so tying selection to it would move
    /// a selected row out from under the user when an entry is added.
    #[test]
    fn the_row_key_survives_a_new_spelling_appearing() {
        let mut upper = entry("a", "Bank");
        upper.tags = vec!["Work".into()];
        let root = VaultGroup {
            id: "root".into(),
            name: "Root".into(),
            entries: vec![upper.clone()],
            ..VaultGroup::default()
        };
        let before = VaultSnapshot::new(root).tags();

        let mut lower = entry("b", "Payroll");
        lower.tags = vec!["Work".into()];
        // An alphabetically earlier spelling arrives and takes the label.
        let mut earlier = entry("c", "Travel");
        earlier.tags = vec!["WORK".into()];
        let root = VaultGroup {
            id: "root".into(),
            name: "Root".into(),
            entries: vec![upper, lower, earlier],
            ..VaultGroup::default()
        };
        let after = VaultSnapshot::new(root).tags();

        assert_eq!(before[0].key, after[0].key, "the identity does not move");
        assert_eq!(before[0].label, "Work");
        assert_eq!(after[0].label, "WORK", "the label follows the vault");
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

    #[test]
    fn debug_output_omits_decrypted_vault_content() {
        let sentinels = [
            "group-secret",
            "title-secret",
            "username-secret",
            "url-secret",
            "notes-secret",
            "tag-secret",
            "custom-key-secret",
            "custom-value-secret",
        ];
        let custom = CustomField {
            key: sentinels[6].into(),
            value: sentinels[7].into(),
            protected: true,
        };
        let mut vault_entry = entry("entry-id", sentinels[1]);
        vault_entry.username = sentinels[2].into();
        vault_entry.url = sentinels[3].into();
        vault_entry.notes = sentinels[4].into();
        vault_entry.tags = vec![sentinels[5].into()];
        vault_entry.custom_fields = vec![custom.clone()];
        let snapshot = VaultSnapshot::new(VaultGroup::new(
            "root-id",
            sentinels[0],
            Vec::new(),
            vec![vault_entry.clone()],
        ));

        let rendered = format!(
            "{snapshot:?} {:?} {vault_entry:?} {custom:?}",
            snapshot.root
        );

        for sentinel in sentinels {
            assert!(!rendered.contains(sentinel), "debug leaked {sentinel}");
        }
    }
}

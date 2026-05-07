use crate::{
    domain::{CustomField, Favicon, FaviconImage, Strength, VaultEntry, VaultGroup, VaultSnapshot},
    keepass::VaultDocument,
};
use chrono::{NaiveDateTime, Utc};
use gpui::{Image, ImageFormat};
use keepass::{
    Database, DatabaseKey,
    db::{DatabaseOpenError, EntryId, EntryRef, GroupId, GroupRef, fields},
};

/// The six KeePass-standard string fields. Anything stored on `Entry.fields`
/// outside this set is surfaced as a custom field — that's the same line
/// KeePassXC draws between "main attributes" and "Additional attributes".
pub(crate) const STANDARD_FIELDS: &[&str] = &[
    fields::TITLE,
    fields::USERNAME,
    fields::PASSWORD,
    fields::URL,
    fields::NOTES,
    fields::OTP,
];
use std::{
    collections::hash_map::DefaultHasher,
    fs::File,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::Arc,
};

pub struct KeePassRepository;

impl KeePassRepository {
    pub fn open_with_password(
        path: impl AsRef<Path>,
        password: &str,
    ) -> Result<VaultDocument, DatabaseOpenError> {
        Self::open(path, password, None)
    }

    pub fn open(
        path: impl AsRef<Path>,
        password: &str,
        keyfile: Option<&Path>,
    ) -> Result<VaultDocument, DatabaseOpenError> {
        let mut file = File::open(path.as_ref())?;
        let mut key = DatabaseKey::new();
        if !password.is_empty() {
            key = key.with_password(password);
        }
        if let Some(keyfile_path) = keyfile {
            let mut keyfile = File::open(keyfile_path)?;
            key = key.with_keyfile(&mut keyfile)?;
        }

        let database = Database::open(&mut file, key)?;
        let snapshot = snapshot_from_database(&database);

        Ok(VaultDocument::new(
            database,
            snapshot,
            password.to_string(),
            keyfile.map(PathBuf::from),
        ))
    }

    /// Decrypt a kdbx blob held entirely in memory. Used by the sync flow
    /// when we get conflict bytes from SharePoint and need to diff them
    /// against the in-memory local database — no temp file required.
    pub fn open_bytes(
        bytes: &[u8],
        password: &str,
        keyfile: Option<&Path>,
    ) -> Result<VaultDocument, DatabaseOpenError> {
        let mut cursor = std::io::Cursor::new(bytes);
        let mut key = DatabaseKey::new();
        if !password.is_empty() {
            key = key.with_password(password);
        }
        if let Some(keyfile_path) = keyfile {
            let mut keyfile = File::open(keyfile_path)?;
            key = key.with_keyfile(&mut keyfile)?;
        }
        let database = Database::open(&mut cursor, key)?;
        let snapshot = snapshot_from_database(&database);
        Ok(VaultDocument::new(
            database,
            snapshot,
            password.to_string(),
            keyfile.map(PathBuf::from),
        ))
    }

    /// Resolve a sibling key file path next to the database file (e.g. `Personal.kdbx` →
    /// `Personal.keyx`). Returns the path only if such a file exists. Used as a courtesy
    /// suggestion in the unlock screen; the real choice is on the user.
    pub fn suggested_keyfile(database: &Path) -> Option<PathBuf> {
        for ext in ["keyx", "key"] {
            let candidate = database.with_extension(ext);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        None
    }
}

pub(crate) fn snapshot_from_database(database: &Database) -> VaultSnapshot {
    let now = Utc::now().naive_utc();
    let root = database.root();
    let recycle_bin_id = database.recycle_bin().map(|g| g.id().to_string());
    let recycle_bin_id_for_traversal = recycle_bin_id.clone();
    let mut snap = VaultSnapshot::new(group_from_ref(
        &root,
        &mut Vec::new(),
        now,
        recycle_bin_id_for_traversal.as_deref(),
    ));
    snap.recycle_bin_id = recycle_bin_id;
    snap
}

/// Walk every group in `database` and return the one whose stringified id
/// matches `id_string`. O(N) over groups; fine for N ≤ a few hundred. Used
/// when round-tripping a domain `String` id back into keepass-rs's typed
/// `GroupId` (`from_uuid` is `pub(crate)` upstream so we can't construct
/// directly).
pub(crate) fn find_group_id(database: &Database, id_string: &str) -> Option<GroupId> {
    database
        .iter_all_groups()
        .find(|g| g.id().to_string() == id_string)
        .map(|g| g.id())
}

pub(crate) fn find_entry_id(database: &Database, id_string: &str) -> Option<EntryId> {
    database
        .iter_all_entries()
        .find(|e| e.id().to_string() == id_string)
        .map(|e| e.id())
}

fn group_from_ref(
    group: &GroupRef<'_>,
    parent_path: &mut Vec<String>,
    now: NaiveDateTime,
    recycle_bin_id: Option<&str>,
) -> VaultGroup {
    let name = non_empty(&group.name, "Root");
    parent_path.push(name.clone());

    let group_id_str = group.id().to_string();
    let in_bin = recycle_bin_id.is_some_and(|bin| bin == group_id_str);

    let mut groups = group
        .groups()
        .map(|child| group_from_ref(&child, parent_path, now, recycle_bin_id))
        .collect::<Vec<_>>();
    groups.sort_by_key(|child| child.name.to_lowercase());

    let mut entries = group
        .entries()
        .map(|entry| entry_from_ref(&entry, parent_path, now, in_bin))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.title.to_lowercase());

    parent_path.pop();

    let is_expanded = group.is_expanded;
    // Same custom-icon handling as entries: if the group has
    // `Icon::Custom(_)`, decode the bytes once at snapshot time and
    // hand the renderer a ready-to-go `gpui::Image`. Built-in icon
    // ids fall through to `None` and the sidebar uses its default.
    let icon = group
        .custom_icon()
        .and_then(|custom| favicon_image_from_bytes(&custom.data));
    VaultGroup {
        id: group_id_str,
        name,
        groups,
        entries,
        is_expanded,
        icon,
    }
}

fn entry_from_ref(
    entry: &EntryRef<'_>,
    group_path: &[String],
    now: NaiveDateTime,
    in_recycle_bin: bool,
) -> VaultEntry {
    let title = non_empty(entry.get_title().unwrap_or_default(), "Untitled");
    let username = entry.get_username().unwrap_or_default().to_string();
    let url = entry.get_url().unwrap_or_default().to_string();
    let notes = entry.get("Notes").unwrap_or_default().to_string();
    let password = entry.get_password().unwrap_or_default();
    let has_password = !password.is_empty();
    let password_length = password.chars().count();
    let has_otp = entry
        .get_raw_otp_value()
        .is_some_and(|otp| !otp.trim().is_empty());

    let updated = entry
        .times
        .last_modification
        .map(|stamp| relative_time(now, stamp));

    let hash = stable_hash(&title);

    let tags = entry.tags.clone();
    // Favorites are surfaced as a tag so the state round-trips through
    // any KeePass client. Recognised case-insensitively so vaults that
    // already use "favorite" / "Favorites" / etc. just work; canonical
    // casing on write lives in `keepass::document::FAVORITE_TAG`.
    let starred = tags
        .iter()
        .any(|t| t.eq_ignore_ascii_case(crate::keepass::document::FAVORITE_TAG));
    let strength = if has_password {
        Strength::from_password_length(password_length)
    } else {
        Strength::Weak
    };
    let mut favicon = synthesize_favicon(&title, &url, hash);
    if let Some(custom) = entry.custom_icon()
        && let Some(image) = favicon_image_from_bytes(&custom.data)
    {
        favicon.image = Some(image);
    }

    let custom_fields = collect_custom_fields(entry);

    VaultEntry {
        id: entry.id().to_string(),
        title,
        username,
        url,
        notes,
        has_password,
        password_length,
        has_otp,
        updated,
        tags,
        starred,
        favicon,
        strength,
        group_path: group_path
            .iter()
            .skip(1) // skip the synthetic Root segment
            .cloned()
            .collect(),
        in_recycle_bin,
        custom_fields,
    }
}

/// Snapshot all non-standard string fields off the entry, sorted by key
/// for stable rendering — KeePass's XML doesn't pin field order, so we
/// can't trust whatever order the parser hands us.
pub(crate) fn collect_custom_fields(entry: &EntryRef<'_>) -> Vec<CustomField> {
    let mut fields_out: Vec<CustomField> = entry
        .fields
        .iter()
        .filter(|(k, _)| !STANDARD_FIELDS.contains(&k.as_str()))
        .map(|(k, v)| CustomField {
            key: k.clone(),
            value: v.get().clone(),
            protected: v.is_protected(),
        })
        .collect();
    fields_out.sort_by(|a, b| a.key.cmp(&b.key));
    fields_out
}

fn synthesize_favicon(title: &str, url: &str, hash: u64) -> Favicon {
    let source = if !title.is_empty() { title } else { url };
    let letter = source
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_ascii_uppercase().to_string())
        .unwrap_or_else(|| "·".to_string());

    Favicon {
        letter,
        palette_index: (hash % FAVICON_PALETTE_SIZE) as u8,
        image: None,
    }
}

/// Sniff a custom-icon blob's format from its leading magic bytes and decode
/// it eagerly into a renderable `gpui::Image`. KeePass stores the bytes
/// verbatim — clients put PNG, JPEG, ICO, etc. in there with no metadata —
/// so we sniff once at DB-load time and cache the `Arc<Image>`; later
/// renders just bump the refcount. Returns `None` when the buffer is empty
/// or the format can't be identified (the renderer falls back to the
/// synthesized letter favicon).
fn favicon_image_from_bytes(bytes: &[u8]) -> Option<FaviconImage> {
    let format = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        ImageFormat::Png
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        ImageFormat::Jpeg
    } else if bytes.starts_with(b"GIF8") {
        ImageFormat::Gif
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        ImageFormat::Webp
    } else if bytes.starts_with(b"BM") {
        ImageFormat::Bmp
    } else if bytes.starts_with(&[0x00, 0x00, 0x01, 0x00]) {
        ImageFormat::Ico
    } else {
        return None;
    };
    Some(FaviconImage(Arc::new(Image::from_bytes(
        format,
        bytes.to_vec(),
    ))))
}

const FAVICON_PALETTE_SIZE: u64 = 12;

fn stable_hash<T: Hash + ?Sized>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn relative_time(now: NaiveDateTime, then: NaiveDateTime) -> String {
    if then > now {
        return "moments ago".to_string();
    }
    let delta = now - then;
    let seconds = delta.num_seconds().max(0);
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    let weeks = days / 7;
    let months = days / 30;
    let years = days / 365;

    match (years, months, weeks, days, hours, minutes) {
        (y, _, _, _, _, _) if y >= 1 => plural(y, "year"),
        (_, m, _, _, _, _) if m >= 1 => plural(m, "month"),
        (_, _, w, _, _, _) if w >= 1 => plural(w, "week"),
        (_, _, _, d, _, _) if d >= 1 => plural(d, "day"),
        (_, _, _, _, h, _) if h >= 1 => plural(h, "hour"),
        (_, _, _, _, _, m) if m >= 1 => plural(m, "minute"),
        _ => "moments ago".to_string(),
    }
}

fn plural(count: i64, unit: &str) -> String {
    if count == 1 {
        format!("1 {unit} ago")
    } else {
        format!("{count} {unit}s ago")
    }
}

fn non_empty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favicon_letter_is_first_alpha() {
        let fav = synthesize_favicon("Figma", "", 0);
        assert_eq!(fav.letter, "F");
    }

    #[test]
    fn favicon_letter_falls_back_to_url() {
        let fav = synthesize_favicon("", "github.com", 0);
        assert_eq!(fav.letter, "G");
    }

    #[test]
    fn favicon_palette_index_within_bounds() {
        for hash in 0..200u64 {
            let fav = synthesize_favicon("Sample", "", hash);
            assert!((fav.palette_index as u64) < FAVICON_PALETTE_SIZE);
        }
    }

    #[test]
    fn relative_time_formats_recent() {
        let now = chrono::NaiveDate::from_ymd_opt(2026, 4, 27)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        let then = now - chrono::Duration::minutes(5);
        assert_eq!(relative_time(now, then), "5 minutes ago");
    }

    #[test]
    fn relative_time_formats_weeks() {
        let now = chrono::NaiveDate::from_ymd_opt(2026, 4, 27)
            .unwrap()
            .and_hms_opt(12, 0, 0)
            .unwrap();
        let then = now - chrono::Duration::days(15);
        assert_eq!(relative_time(now, then), "2 weeks ago");
    }
}

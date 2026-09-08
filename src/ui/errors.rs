//! Domain errors turned into sentences a user can act on.
//!
//! The `thiserror` `Display` strings stay in the lowercase, no-period style
//! the CLI prints them in. The UI needs whole sentences and its own
//! vocabulary, and it needs both to be identical wherever the same failure
//! surfaces, so the mapping lives here rather than at each call site.

use crate::keepass::MutationError;
use keepass::db::DatabaseOpenError;

/// What went wrong with a vault mutation, phrased for a toast.
pub fn mutation_message(error: &MutationError) -> String {
    match error {
        MutationError::GroupNotFound => "That group no longer exists.".into(),
        MutationError::EntryNotFound => "That entry no longer exists.".into(),
        MutationError::RecycleBinUnavailable => {
            "This database has no Trash, so nothing can be deleted into it.".into()
        }
        MutationError::GroupNameEmpty => "A group needs a name.".into(),
        MutationError::CannotDeleteRoot => "The root group cannot be deleted.".into(),
        MutationError::CannotDeleteRecycleBin => "The Trash itself cannot be deleted.".into(),
        MutationError::CannotMoveRoot => "The root group cannot be moved.".into(),
        MutationError::CannotMoveRecycleBin => "The Trash itself cannot be moved.".into(),
        MutationError::WouldCreateCycle => "A group cannot be moved inside itself.".into(),
    }
}

/// Why a vault would not open, phrased for the unlock screen.
///
/// The unlock screen rendered `DatabaseOpenError`'s own `Display`, which is
/// the upstream crate talking to a Rust developer. The overwhelmingly common
/// case, a mistyped password, arrived as a cryptography or format error and
/// read like a corrupt file.
pub fn open_message(error: &DatabaseOpenError) -> String {
    match error {
        // A wrong password fails the HMAC before anything is parsed, so it
        // surfaces as a key, cryptography or format error depending on where
        // the mismatch lands. A user cannot tell those apart and does not
        // need to: the action is the same.
        DatabaseOpenError::Key(_)
        | DatabaseOpenError::Cryptography(_)
        | DatabaseOpenError::Format(_) => "Wrong master password or key file.".to_string(),
        // Not "cannot open KDBX 3": it can, and does. What it cannot open
        // is the pre-release KeePass 2 format, and a header whose version
        // will not parse at all.
        DatabaseOpenError::UnsupportedVersion | DatabaseOpenError::VersionParse(_) => {
            "FerrisPass cannot read this KeePass format. Convert the database in KeePassXC."
                .to_string()
        }
        DatabaseOpenError::UnexpectedEof => {
            "This vault file is truncated. Restore it from a backup or from your cloud provider."
                .to_string()
        }
        DatabaseOpenError::Io(error) => match error.kind() {
            std::io::ErrorKind::NotFound => "That vault file no longer exists.".to_string(),
            std::io::ErrorKind::PermissionDenied => {
                "FerrisPass is not allowed to read that file.".to_string()
            }
            _ => format!("Could not read the vault file: {}", error.kind()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The overwhelmingly common failure is a mistyped password, and it
    /// arrives as whichever low-level error the mismatch happened to trip.
    /// All of them have to read as "wrong password", not as "corrupt file".
    #[test]
    fn a_truncated_file_points_at_a_backup() {
        assert!(
            open_message(&DatabaseOpenError::UnexpectedEof).contains("truncated"),
            "a short read is not a wrong password"
        );
    }

    #[test]
    fn an_old_format_says_which_format() {
        assert!(open_message(&DatabaseOpenError::UnsupportedVersion).contains("KeePassXC"));
    }

    #[test]
    fn a_missing_file_says_so() {
        let missing = DatabaseOpenError::Io(std::io::Error::from(std::io::ErrorKind::NotFound));
        assert_eq!(open_message(&missing), "That vault file no longer exists.");
    }

    /// Every message is a sentence, and none of them says "Recycle Bin":
    /// the UI calls it Trash everywhere, including in its errors.
    #[test]
    fn every_mutation_message_is_a_sentence_using_the_ui_vocabulary() {
        for error in [
            MutationError::GroupNotFound,
            MutationError::EntryNotFound,
            MutationError::RecycleBinUnavailable,
            MutationError::GroupNameEmpty,
            MutationError::CannotDeleteRoot,
            MutationError::CannotDeleteRecycleBin,
            MutationError::CannotMoveRoot,
            MutationError::CannotMoveRecycleBin,
            MutationError::WouldCreateCycle,
        ] {
            let message = mutation_message(&error);
            assert!(message.ends_with('.'), "{error:?} is not a sentence");
            assert!(
                message.starts_with(|c: char| c.is_uppercase()),
                "{error:?} does not start a sentence"
            );
            assert!(
                !message.contains("Recycle Bin"),
                "{error:?} leaks the KeePass term into the UI"
            );
        }
    }
}

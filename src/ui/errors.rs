//! Domain errors turned into sentences a user can act on.
//!
//! The `thiserror` `Display` strings stay in the lowercase, no-period style
//! the CLI prints them in. The UI needs whole sentences and its own
//! vocabulary, and it needs both to be identical wherever the same failure
//! surfaces, so the mapping lives here rather than at each call site.

use crate::keepass::MutationError;

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

#[cfg(test)]
mod tests {
    use super::*;

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

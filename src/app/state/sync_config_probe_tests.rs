use super::*;

/// The probe decides only whether to skip the restore entirely, so
/// anything short of "the file is not there" has to fall through to the
/// read that can report a problem. Treating an unresolvable path as an
/// absent config turned a bound vault into a local-only one for the rest
/// of the session, without a word.
#[test]
fn only_a_definite_missing_file_counts_as_no_sync_config() {
    let dir = tempfile::tempdir().expect("tempdir");

    let absent = dir.path().join("absent.json");
    assert!(known_to_have_no_sync_config(Ok(absent)));

    let present = dir.path().join("present.json");
    std::fs::write(&present, "{}").expect("write");
    assert!(!known_to_have_no_sync_config(Ok(present)));

    assert!(!known_to_have_no_sync_config(Err(
        crate::sync::config::ConfigError::NoSupportDir("$HOME not set".into())
    )));
}

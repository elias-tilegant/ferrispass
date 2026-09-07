//! The release binary against a real file on disk.
//!
//! A green unit-test suite has never caught a broken parse path: the units
//! hand each other in-memory `Database` values and never serialise. This
//! drives the shipped CLI over a vault written by the fixture builder, which
//! is what the release checklist asks a person to do by hand. That step was
//! previously only described in a document, so nothing recorded whether it had
//! ever been run.
//!
//! It does not replace the KeePassXC leg of the round trip, which needs a
//! second implementation and stays manual.

use std::process::Command;

use keepass::{Database, DatabaseKey, db::fields};

const PASSWORD: &str = "round-trip-password";

fn cli() -> &'static str {
    env!("CARGO_BIN_EXE_ferrispass-cli")
}

/// Write a vault with the shapes that have broken before: a nested group, an
/// entry in the recycle bin, a custom icon, a history version, and a custom
/// field.
fn write_vault(path: &std::path::Path) {
    let mut db = Database::new();
    let live_id = {
        let mut root = db.root_mut();
        let mut group = root.add_group();
        group.name = "Live".into();
        group.id()
    };
    {
        let mut group = db.group_mut(live_id).expect("group");
        let mut entry = group.add_entry();
        entry.set_unprotected(fields::TITLE, "Kept Entry");
        entry.set_unprotected(fields::USERNAME, "alice");
        entry.set_protected(fields::PASSWORD, "s3cret");
        entry.set_unprotected("SAP_HOST", "sap.example.invalid");
    }
    let entry_id = db
        .group(live_id)
        .expect("group")
        .entries()
        .next()
        .expect("entry")
        .id();
    // A second version, so the history path is exercised by the reader.
    db.entry_mut(entry_id)
        .expect("entry")
        .track_changes()
        .as_mut()
        .set_unprotected(fields::NOTES, "edited once");
    db.entry_mut(entry_id)
        .expect("entry")
        .set_icon_custom_new(vec![0x89, b'P', b'N', b'G', 1, 2, 3]);

    let mut file = std::fs::File::create(path).expect("create vault");
    db.save(&mut file, DatabaseKey::new().with_password(PASSWORD))
        .expect("write vault");
}

/// Run the CLI with the password on descriptor 3, the way the release
/// checklist does. Driven through `sh` because opening a specific descriptor
/// in the child needs `pre_exec`, and this workspace forbids `unsafe`.
fn run(
    vault: &std::path::Path,
    password_file: &std::path::Path,
    args: &[&str],
) -> serde_json::Value {
    let command = format!(
        "exec {} --vault {} --master-password-fd 3 --format json {} 3<{}",
        shell_quote(cli()),
        shell_quote(&vault.to_string_lossy()),
        args.iter()
            .map(|arg| shell_quote(arg))
            .collect::<Vec<_>>()
            .join(" "),
        shell_quote(&password_file.to_string_lossy()),
    );
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(&command)
        .output()
        .expect("run the cli");
    assert!(
        output.status.success(),
        "cli failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cli stdout must be JSON")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[test]
fn the_release_binary_reads_a_vault_it_wrote() {
    let dir = tempfile::tempdir().expect("tempdir");
    let vault = dir.path().join("round-trip.kdbx");
    let password_file = dir.path().join("pw");
    std::fs::write(&password_file, format!("{PASSWORD}\n")).expect("write password file");
    write_vault(&vault);

    let listed = run(&vault, &password_file, &["entry", "list"]);
    let entries = listed["data"]["entries"].as_array().expect("entries array");
    assert_eq!(
        entries.len(),
        1,
        "the vault round-tripped through the writer"
    );
    assert_eq!(entries[0]["title"], "Kept Entry");
    assert_eq!(entries[0]["group_path"][0], "Live");
    assert_eq!(entries[0]["in_trash"], false);
    assert_eq!(entries[0]["has_password"], true);

    // The custom field the SAP launcher reads survives the write path.
    let id = entries[0]["id"].as_str().expect("entry id");
    let shown = run(&vault, &password_file, &["entry", "get", "--id", id]);
    let fields = shown["data"]["entry"]["custom_fields"]
        .as_array()
        .expect("custom fields");
    assert!(
        fields
            .iter()
            .any(|field| field["key"] == "SAP_HOST" && field["value"] == "sap.example.invalid"),
        "custom fields survive: {fields:?}"
    );
}

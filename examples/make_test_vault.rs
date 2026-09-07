//! Fixture builder for manual verification.
//!
//! The app has no "create vault" flow yet and the CLI has no create command,
//! so there is otherwise no way to produce a vault to test against. Writes a
//! KDBX 4 file with one group and one entry, which the CLI and the app can
//! then open, mutate and round-trip against KeePassXC.
//!
//! ```sh
//! cargo run --example make_test_vault -- /tmp/test.kdbx hunter2
//! ```
use keepass::{Database, DatabaseKey, db::fields};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(password)) = (args.next(), args.next()) else {
        eprintln!("usage: make_test_vault <path> <master-password>");
        std::process::exit(2);
    };

    let mut db = Database::new();
    {
        let mut root = db.root_mut();
        let mut group = root.add_group();
        group.name = "Live".into();
        let mut entry = group.add_entry();
        entry.set_unprotected(fields::TITLE, "Kept Entry");
        entry.set_unprotected(fields::USERNAME, "alice");
        entry.set_protected(fields::PASSWORD, "s3cret");
    }

    let key = DatabaseKey::new().with_password(&password);
    let mut file = std::fs::File::create(&path).expect("create vault file");
    db.save(&mut file, key).expect("write vault");
    println!("wrote {path}");
}

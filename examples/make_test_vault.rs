//! Fixture builder for manual verification.
//!
//! The app has no "create vault" flow yet and the CLI has no create command,
//! so there is otherwise no way to produce a vault to test against. Writes a
//! KDBX 4 file with one group and one entry, which the CLI and the app can
//! then open, mutate and round-trip against KeePassXC.
//!
//! The password is never an argument. It would sit in the shell history and
//! in every process listing on the machine for as long as the command ran,
//! and a fixture vault is still a vault: the point of the round trip is to
//! open it again later.
//!
//! ```sh
//! cargo run --example make_test_vault -- /tmp/test.kdbx      # prompts
//! cargo run --example make_test_vault -- /tmp/test.kdbx 3<pw # or from an fd
//! ```
use std::io::Read as _;

use keepass::{Database, DatabaseKey, db::fields};
use zeroize::Zeroizing;

/// Where a scripted run may hand us the password. Matches the CLI's
/// `--master-password-fd` so both halves of the release check are driven the
/// same way.
const SECRET_FD: u32 = 3;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: make_test_vault <path>   (password on fd 3, or prompted)");
        std::process::exit(2);
    };
    if args.next().is_some() {
        eprintln!("the master password is read from fd 3 or prompted, never from the command line");
        std::process::exit(2);
    }
    let password = read_password();

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

/// Read fd 3 when the caller opened it, otherwise prompt without echo.
/// Opening it through `/dev/fd` is the same path the CLI's
/// `--master-password-fd` takes, and it keeps this example inside the
/// workspace's no-unsafe rule.
fn read_password() -> Zeroizing<String> {
    if let Ok(mut file) = std::fs::File::open(format!("/dev/fd/{SECRET_FD}")) {
        let mut value = Zeroizing::new(String::new());
        file.read_to_string(&mut value).expect("read fd 3");
        while value.ends_with(['\n', '\r']) {
            value.pop();
        }
        return value;
    }
    Zeroizing::new(rpassword::prompt_password("Master password: ").expect("read password"))
}

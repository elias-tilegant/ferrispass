//! Decrypt a `.kdbx` and dump the inner cleartext XML to stdout.
//!
//! Usage:
//!
//! ```sh
//! cargo run --example dump_xml -- /path/to/file.kdbx
//! ```
//!
//! Then it prompts for the master password (or read it from `STC_KDBX_PW`
//! env var). The password never leaves your machine. Output is XML —
//! grep for `<…/>` empty tags to spot interop-breaking serializations.

use std::io::{self, BufRead, Write};

fn main() {
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            eprintln!("usage: cargo run --example dump_xml -- <kdbx-path>");
            std::process::exit(2);
        }
    };

    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        eprintln!("could not read {path}: {e}");
        std::process::exit(1);
    });

    // Prefer env var (so password isn't visible in shell history) but fall
    // back to a stdin prompt for convenience.
    let password = std::env::var("STC_KDBX_PW").unwrap_or_else(|_| {
        eprint!("master password: ");
        let _ = io::stderr().flush();
        let mut buf = String::new();
        io::stdin().lock().read_line(&mut buf).expect("stdin");
        buf.trim_end_matches('\n').to_string()
    });

    let mut key = keepass::DatabaseKey::new();
    if !password.is_empty() {
        key = key.with_password(&password);
    }

    let xml = keepass::debug_decrypt_to_xml(&bytes, &key).unwrap_or_else(|e| {
        eprintln!("decrypt failed: {e}");
        std::process::exit(1);
    });

    // Write raw bytes to stdout so the XML's encoding (UTF-8) round-trips.
    let mut out = io::stdout().lock();
    out.write_all(&xml).expect("stdout");
    out.write_all(b"\n").expect("stdout");
}

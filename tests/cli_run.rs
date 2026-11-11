//! `ferrispass-cli run` as a process, against a vault on disk.
//!
//! Each test writes a vault with one entry, starts the CLI with the master
//! password on descriptor 9 and has it start `/bin/sh` with secrets of that
//! entry on descriptors. The CLI runs with an empty home folder of its own,
//! so it reads no settings and no Touch ID enrolment of the Mac running the
//! tests.
//!
//! Driven through `sh` like `tests/vault_round_trip.rs`: opening a specific
//! descriptor in the CLI's process needs `pre_exec`, and tests may not use
//! `unsafe`.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use keepass::{Database, DatabaseKey, db::fields};

const MASTER_PASSWORD: &str = "cli-run-master-password";
const PASSWORD: &str = "password-on-a-descriptor";
const API_KEY: &str = "custom-value-on-a-descriptor";
/// The usual example secret from RFC 6238 tools; any code it gives is six
/// digits.
const OTP: &str = "otpauth://totp/FerrisPass:test?secret=JBSWY3DPEHPK3PXP&issuer=FerrisPass";

/// What the leak test looks for, in full and in every part of four
/// characters.
const SENTINEL: &str = "sfd-sentinel-9Qz!2026";

fn cli() -> &'static str {
    env!("CARGO_BIN_EXE_ferrispass-cli")
}

/// A vault with one entry, the file holding its master password, and an empty
/// home folder for the CLI.
struct Fixture {
    dir: tempfile::TempDir,
    vault: PathBuf,
    master_password: PathBuf,
    id: String,
}

impl Fixture {
    fn new(password: &str) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let vault = dir.path().join("vault.kdbx");
        let master_password = dir.path().join("master");
        std::fs::write(&master_password, format!("{MASTER_PASSWORD}\n")).expect("write password");
        std::fs::create_dir(dir.path().join("home")).expect("create the home folder");

        let mut db = Database::new();
        let id = {
            let mut root = db.root_mut();
            let mut entry = root.add_entry();
            entry.set_unprotected(fields::TITLE, "Service");
            entry.set_unprotected(fields::USERNAME, "agent");
            entry.set_protected(fields::PASSWORD, password);
            entry.set_protected("otp", OTP);
            entry.set_protected("API_KEY", API_KEY);
            entry.set_unprotected("NOTE", "not a secret");
            entry.id().to_string()
        };
        let mut file = std::fs::File::create(&vault).expect("create vault");
        db.save(&mut file, DatabaseKey::new().with_password(MASTER_PASSWORD))
            .expect("write vault");
        Self {
            dir,
            vault,
            master_password,
            id,
        }
    }

    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    /// Runs the CLI with `args` after the global options, in JSON or human
    /// format.
    fn cli(&self, format: &str, args: &[&str]) -> Outcome {
        let command = format!(
            "exec {} --vault {} --master-password-fd 9 --format {format} {} 9<{}",
            shell_quote(cli()),
            shell_quote(&self.vault.to_string_lossy()),
            args.iter()
                .map(|arg| shell_quote(arg))
                .collect::<Vec<_>>()
                .join(" "),
            shell_quote(&self.master_password.to_string_lossy()),
        );
        let output = Command::new("/bin/sh")
            .arg("-c")
            .arg(&command)
            .env("HOME", self.home())
            .stdin(Stdio::null())
            .output()
            .expect("run the cli");
        Outcome {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    /// `run --id <the entry> <secret_fds> -- /bin/sh -c <script> <args>`.
    fn run(&self, secret_fds: &[&str], script: &str, args: &[&str]) -> Outcome {
        let mut all = vec!["run", "--id", &self.id];
        for spec in secret_fds {
            all.push("--secret-fd");
            all.push(spec);
        }
        all.extend(["--", "/bin/sh", "-c", script]);
        all.extend(args);
        self.cli("json", &all)
    }
}

#[derive(Debug)]
struct Outcome {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Outcome {
    /// The `code` of the JSON error envelope on stderr.
    fn error_code(&self) -> String {
        let line = self
            .stderr
            .lines()
            .find(|line| line.starts_with('{'))
            .unwrap_or_else(|| panic!("no JSON error on stderr: {self:?}"));
        let envelope: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|err| panic!("{err}: {line}"));
        envelope["error"]["code"]
            .as_str()
            .unwrap_or_else(|| panic!("no error code: {line}"))
            .to_owned()
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

fn assert_ok(outcome: &Outcome) {
    assert_eq!(outcome.code, Some(0), "{outcome:?}");
}

#[test]
fn run_passes_the_password_on_fd_3() {
    let fixture = Fixture::new(PASSWORD);
    let outcome = fixture.run(&["3:password"], "cat <&3", &[]);
    assert_ok(&outcome);
    assert_eq!(outcome.stdout, PASSWORD);
    assert_eq!(outcome.stderr, "");
}

#[test]
fn run_passes_totp_and_custom_fields() {
    let fixture = Fixture::new(PASSWORD);
    let outcome = fixture.run(
        &["3:totp", "4:custom:API_KEY"],
        "cat <&3; printf '|'; cat <&4",
        &[],
    );
    assert_ok(&outcome);
    let (code, custom) = outcome
        .stdout
        .split_once('|')
        .unwrap_or_else(|| panic!("{outcome:?}"));
    assert!(
        code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()),
        "a one-time code is six digits: {code:?}"
    );
    assert_eq!(custom, API_KEY);

    // A custom field that is not protected is no secret, as with
    // `entry secret`.
    let outcome = fixture.run(&["3:custom:NOTE"], "cat <&3", &[]);
    assert_eq!(outcome.code, Some(6), "{outcome:?}");
    assert_eq!(outcome.error_code(), "not_secret");
}

#[test]
fn run_forwards_the_exit_code() {
    let fixture = Fixture::new(PASSWORD);
    let outcome = fixture.run(&["3:password"], "cat <&3 >/dev/null; exit 42", &[]);
    assert_eq!(outcome.code, Some(42), "{outcome:?}");

    // Ended by a signal: 128 plus its number, as a shell reports it.
    let outcome = fixture.run(&["3:password"], "kill -TERM $$", &[]);
    assert_eq!(outcome.code, Some(128 + 15), "{outcome:?}");
}

#[test]
fn run_refuses_unknown_fields_with_exit_6() {
    let fixture = Fixture::new(PASSWORD);
    for field in ["3:username", "3:title", "3:", "3:custom", "3:Password"] {
        let outcome = fixture.run(&[field], "echo started", &[]);
        assert_eq!(outcome.code, Some(6), "{field}: {outcome:?}");
        assert_eq!(outcome.error_code(), "invalid_field", "{field}");
        assert_eq!(outcome.stdout, "", "{field}: the program must not start");
    }
}

/// The program prints its environment and its arguments, then what it reads
/// on descriptor 3. The password is in the last part only.
#[test]
fn run_never_puts_the_secret_in_argv_or_env() {
    let fixture = Fixture::new(PASSWORD);
    let outcome = fixture.run(
        &["3:password"],
        "env; echo \"$0 $*\"; printf '%s' '--descriptor--'; cat <&3",
        &["program-name", "first", "second"],
    );
    assert_ok(&outcome);
    let (seen, on_the_descriptor) = outcome
        .stdout
        .split_once("--descriptor--")
        .unwrap_or_else(|| panic!("{outcome:?}"));
    assert!(seen.contains("program-name first second"), "{seen}");
    assert!(
        seen.contains("HOME="),
        "the environment was printed: {seen}"
    );
    assert!(
        !seen.contains(PASSWORD),
        "the password is in the program's environment or arguments: {seen}"
    );
    assert_eq!(on_the_descriptor, PASSWORD);
}

/// Whether `text` holds `SENTINEL` or any four characters in a row of it.
fn part_of_sentinel(text: &str) -> Option<&'static str> {
    (0..=SENTINEL.len() - 4)
        .map(|from| &SENTINEL[from..from + 4])
        .find(|part| text.contains(part))
}

/// The password is the sentinel many times over, more than a pipe holds, so
/// a program that does not read it makes the write fail. On every way `run`
/// can go wrong, nothing the CLI or the program prints holds any part of it.
#[test]
fn run_never_leaks_the_secret_into_output_or_errors() {
    let password = SENTINEL.repeat(10_000);
    let fixture = Fixture::new(&password);
    let missing = "/nonexistent/ferrispass-cli-run-test";
    let id = fixture.id.clone();
    let cases: [(&str, Vec<&str>, Option<i32>); 7] = [
        (
            "the program is missing",
            vec![
                "run",
                "--id",
                &id,
                "--secret-fd",
                "3:password",
                "--",
                missing,
            ],
            Some(7),
        ),
        (
            "the program exits without reading",
            vec![
                "run",
                "--id",
                &id,
                "--secret-fd",
                "3:password",
                "--",
                "/bin/sh",
                "-c",
                "exit 0",
            ],
            Some(0),
        ),
        (
            "the descriptor is above 255",
            vec![
                "run",
                "--id",
                &id,
                "--secret-fd",
                "256:password",
                "--",
                "/bin/sh",
            ],
            Some(6),
        ),
        (
            "the descriptor is below 3",
            vec![
                "run",
                "--id",
                &id,
                "--secret-fd",
                "1:password",
                "--",
                "/bin/sh",
            ],
            Some(6),
        ),
        (
            "the descriptor is named twice",
            vec![
                "run",
                "--id",
                &id,
                "--secret-fd",
                "3:password",
                "--secret-fd",
                "3:totp",
                "--",
                "/bin/sh",
            ],
            Some(6),
        ),
        (
            "the field is named twice",
            vec![
                "run",
                "--id",
                &id,
                "--secret-fd",
                "3:password",
                "--secret-fd",
                "4:password",
                "--",
                "/bin/sh",
            ],
            Some(6),
        ),
        (
            "the field is unknown",
            vec![
                "run",
                "--id",
                &id,
                "--secret-fd",
                "3:secret",
                "--",
                "/bin/sh",
            ],
            Some(6),
        ),
    ];
    for (what, args, code) in &cases {
        for format in ["json", "human"] {
            let outcome = fixture.cli(format, args);
            assert_eq!(outcome.code, *code, "{what}, {format}: {outcome:?}");
            for (stream, text) in [("stdout", &outcome.stdout), ("stderr", &outcome.stderr)] {
                assert_eq!(
                    part_of_sentinel(text),
                    None,
                    "{what}, {format}: {stream} holds part of the secret: {text}"
                );
            }
            if *what == "the program exits without reading" {
                assert!(
                    outcome
                        .stderr
                        .contains(&format!("of {} bytes", password.len())),
                    "{what}, {format}: the short write is reported as a count: {outcome:?}"
                );
            }
        }
    }
}

/// `entry secret --reveal` with stdout on a pipe, as in a script.
#[test]
fn reveal_is_refused_when_stdout_is_not_a_terminal() {
    let fixture = Fixture::new(PASSWORD);
    let id = fixture.id.clone();
    let outcome = fixture.cli(
        "json",
        &[
            "entry", "secret", "--id", &id, "--field", "password", "--reveal",
        ],
    );
    assert_eq!(outcome.code, Some(6), "{outcome:?}");
    assert_eq!(outcome.error_code(), "reveal_needs_terminal");
    assert_eq!(outcome.stdout, "", "nothing on stdout");
    assert!(!outcome.stderr.contains(PASSWORD), "{outcome:?}");
}

#[test]
fn reveal_error_points_to_run() {
    let fixture = Fixture::new(PASSWORD);
    let id = fixture.id.clone();
    let outcome = fixture.cli(
        "human",
        &[
            "entry", "secret", "--id", &id, "--field", "password", "--reveal",
        ],
    );
    assert_eq!(outcome.code, Some(6), "{outcome:?}");
    assert!(
        outcome.stderr.contains("run --secret-fd"),
        "the error says what to use instead: {outcome:?}"
    );
}

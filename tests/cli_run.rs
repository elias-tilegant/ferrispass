//! `ferrispass-cli run` as a process, against a vault on disk.
//!
//! Each test writes a vault with one entry, starts the CLI with the master
//! password on descriptor 9 and has it start `/bin/sh` with secrets of that
//! entry on descriptors, or with `--env` in variables. The CLI runs with an
//! empty home folder of its own,
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

    /// A path in the fixture's folder.
    fn path(&self, name: &str) -> String {
        self.dir.path().join(name).to_string_lossy().into_owned()
    }

    /// The shell line that starts the CLI with `args` after the global
    /// options, in JSON or human format, with the master password file open
    /// on descriptor `master_fd`, or with that descriptor closed.
    fn command_line(&self, format: &str, master_fd: u32, open: bool, args: &[&str]) -> String {
        let redirect = if open {
            format!(
                "{master_fd}<{}",
                shell_quote(&self.master_password.to_string_lossy())
            )
        } else {
            format!("{master_fd}<&-")
        };
        format!(
            "exec {} --vault {} --master-password-fd {master_fd} --format {format} {} {redirect}",
            shell_quote(cli()),
            shell_quote(&self.vault.to_string_lossy()),
            args.iter()
                .map(|arg| shell_quote(arg))
                .collect::<Vec<_>>()
                .join(" "),
        )
    }

    /// `/bin/sh -c <line>` with the fixture's home folder and no stdin.
    fn shell(&self, line: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(line)
            .env("HOME", self.home())
            .stdin(Stdio::null());
        command
    }

    /// Runs `line` to the end.
    fn output(&self, line: &str) -> Outcome {
        let output = self.shell(line).output().expect("run the cli");
        Outcome {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    /// Runs the CLI with `args` after the global options, in JSON or human
    /// format, with the master password on descriptor 9.
    fn cli(&self, format: &str, args: &[&str]) -> Outcome {
        self.output(&self.command_line(format, 9, true, args))
    }

    /// `run --id <the entry> <secret_fds> -- /bin/sh -c <script> <args>`.
    fn run(&self, secret_fds: &[&str], script: &str, args: &[&str]) -> Outcome {
        self.cli("json", &self.run_args(secret_fds, &[], script, args))
    }

    /// The arguments for `run --id <the entry> <secret_fds> <env> -- /bin/sh
    /// -c <script> <args>`, each of `env` after an `--env`.
    fn run_args<'a>(
        &'a self,
        secret_fds: &[&'a str],
        env: &[&'a str],
        script: &'a str,
        args: &[&'a str],
    ) -> Vec<&'a str> {
        let mut all = vec!["run", "--id", &self.id];
        for spec in secret_fds {
            all.extend(["--secret-fd", spec]);
        }
        for spec in env {
            all.extend(["--env", spec]);
        }
        all.extend(["--", "/bin/sh", "-c", script]);
        all.extend(args);
        all
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

    // With `--env` for another field, the password on the descriptor is
    // still in neither, and the variable holds only its own field.
    let outcome = fixture.cli(
        "json",
        &fixture.run_args(
            &["3:password"],
            &["FP_CODE=totp"],
            "env; echo \"$0 $*\"; printf '%s' '--descriptor--'; cat <&3",
            &["program-name", "first"],
        ),
    );
    assert_ok(&outcome);
    let (seen, on_the_descriptor) = outcome
        .stdout
        .split_once("--descriptor--")
        .unwrap_or_else(|| panic!("{outcome:?}"));
    assert!(seen.contains("program-name first"), "{seen}");
    assert!(
        !seen.contains(PASSWORD),
        "the password is in the program's environment or arguments: {seen}"
    );
    let code = seen
        .lines()
        .find_map(|line| line.strip_prefix("FP_CODE="))
        .unwrap_or_else(|| panic!("no FP_CODE in the environment: {seen}"));
    assert!(
        code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()),
        "FP_CODE holds the one-time code: {code:?}"
    );
    assert!(
        !seen.contains(&format!("program-name first {code}")),
        "the code is in the program's arguments: {seen}"
    );
    assert_eq!(on_the_descriptor, PASSWORD);
}

/// The CLI reads the master password from descriptor 9, which the shell
/// opened on a file. A program that had that descriptor could seek it back to
/// the start and read the master password, as the `zsh` line does when it
/// can. The program gets only its `--secret-fd` descriptors, and one on the
/// same number as the master password still gets its secret.
#[test]
fn run_keeps_the_master_password_descriptor_from_the_program() {
    let fixture = Fixture::new(PASSWORD);
    let outcome = fixture.run(
        &["3:password"],
        "cat <&3; \
         if { true <&9; } 2>/dev/null; then printf '|9 is open'; else printf '|9 is closed'; fi; \
         /bin/zsh -c 'zmodload zsh/system && sysseek -u 9 0 && cat <&9' 2>/dev/null; \
         printf '|end'",
        &[],
    );
    assert_ok(&outcome);
    assert!(
        !outcome.stdout.contains(MASTER_PASSWORD),
        "the program read the master password: {outcome:?}"
    );
    assert_eq!(outcome.stdout, format!("{PASSWORD}|9 is closed|end"));

    let outcome = fixture.run(&["9:password"], "cat <&9", &[]);
    assert_ok(&outcome);
    assert_eq!(outcome.stdout, PASSWORD);
}

/// The child closes the master password's descriptor, which is safe only
/// while it is open in the CLI. One that is not open stops `run` before the
/// vault is unlocked, with exit code 6, not 3 for a failed unlock.
#[test]
fn run_refuses_a_master_password_descriptor_that_is_not_open() {
    let fixture = Fixture::new(PASSWORD);
    let line = fixture.command_line(
        "json",
        9,
        false,
        &fixture.run_args(&["3:password"], &[], "echo started", &[]),
    );
    let outcome = fixture.output(&line);
    assert_eq!(outcome.code, Some(6), "{outcome:?}");
    assert_eq!(outcome.error_code(), "invalid_input");
    assert!(
        outcome.stderr.contains("descriptor 9 is not open"),
        "{outcome:?}"
    );
    assert_eq!(outcome.stdout, "", "the program must not start");
}

/// The warning `--env` prints for `name`.
fn env_warning(name: &str) -> String {
    format!(
        "warning: {name} is set only for this program, not exported, and readable by \
         programs running as your user"
    )
}

/// The program prints its environment. The password is in exactly one line,
/// the variable `--env` names, and nowhere else.
#[test]
fn env_fallback_sets_only_the_named_variable() {
    let fixture = Fixture::new(PASSWORD);
    let outcome = fixture.cli(
        "json",
        &fixture.run_args(&[], &["FP_SECRET=password"], "env", &[]),
    );
    assert_ok(&outcome);
    let holding: Vec<&str> = outcome
        .stdout
        .lines()
        .filter(|line| line.contains(PASSWORD))
        .collect();
    assert_eq!(holding, [format!("FP_SECRET={PASSWORD}")], "{outcome:?}");
    assert!(
        outcome.stdout.contains("HOME="),
        "the environment was printed: {outcome:?}"
    );
}

/// `--env` puts the secret in the program's environment, and only there:
/// not in its arguments, not on the CLI's stdout or stderr, not in the
/// warning, and not in an error after the vault was unlocked.
#[test]
fn the_env_fallback_leaks_the_secret_nowhere_else() {
    let fixture = Fixture::new(SENTINEL);
    let argv = fixture.path("argv.txt");
    let env = fixture.path("env.txt");
    let script = format!(
        "printf '%s\\n' \"$0\" \"$@\" > {}; printenv FP_SECRET > {}",
        shell_quote(&argv),
        shell_quote(&env)
    );
    for format in ["json", "human"] {
        let outcome = fixture.cli(
            format,
            &fixture.run_args(
                &[],
                &["FP_SECRET=password"],
                &script,
                &["program-name", "first"],
            ),
        );
        assert_ok(&outcome);
        let argv_seen = std::fs::read_to_string(&argv).expect("the program wrote argv.txt");
        assert_eq!(argv_seen, "program-name\nfirst\n", "{format}");
        assert_eq!(part_of_sentinel(&argv_seen), None, "{format}: {argv_seen}");
        let env_seen = std::fs::read_to_string(&env).expect("the program wrote env.txt");
        assert_eq!(env_seen, format!("{SENTINEL}\n"), "{format}");
        assert_eq!(outcome.stdout, "", "{format}");
        assert_eq!(
            outcome.stderr.trim_end(),
            env_warning("FP_SECRET"),
            "{format}"
        );
        assert_eq!(
            part_of_sentinel(&outcome.stderr),
            None,
            "{format}: {outcome:?}"
        );

        // Unlocked, the value read, and then the program is missing.
        let outcome = fixture.cli(
            format,
            &[
                "run",
                "--id",
                &fixture.id,
                "--env",
                "FP_SECRET=password",
                "--",
                "/nonexistent/ferrispass-cli-run-test",
            ],
        );
        assert_eq!(outcome.code, Some(7), "{format}: {outcome:?}");
        assert!(
            outcome.stderr.starts_with(&env_warning("FP_SECRET")),
            "{format}: {outcome:?}"
        );
        if format == "json" {
            assert_eq!(outcome.error_code(), "run_failed");
        }
        for (stream, text) in [("stdout", &outcome.stdout), ("stderr", &outcome.stderr)] {
            assert_eq!(
                part_of_sentinel(text),
                None,
                "{format}: {stream} holds part of the secret: {text}"
            );
        }
    }
}

/// One warning line per variable, on stderr, before the program starts; with
/// `--format json` too, since stderr is not the JSON envelope.
#[test]
fn env_fallback_prints_the_warning_once() {
    let fixture = Fixture::new(PASSWORD);
    for format in ["json", "human"] {
        let outcome = fixture.cli(
            format,
            &fixture.run_args(&[], &["FP_SECRET=password"], "printf started >&2", &[]),
        );
        assert_ok(&outcome);
        assert_eq!(
            outcome.stderr,
            format!("{}\nstarted", env_warning("FP_SECRET")),
            "{format}: the warning comes once and before the program"
        );

        let outcome = fixture.cli(
            format,
            &fixture.run_args(
                &["3:custom:API_KEY"],
                &["FP_SECRET=password", "FP_CODE=totp"],
                "cat <&3 >/dev/null",
                &[],
            ),
        );
        assert_ok(&outcome);
        assert_eq!(
            outcome.stderr,
            format!("{}\n{}\n", env_warning("FP_SECRET"), env_warning("FP_CODE")),
            "{format}: one line per variable, none for the descriptor"
        );
    }
}

/// Whether `text` holds `SENTINEL` or any four characters in a row of it.
fn part_of_sentinel(text: &str) -> Option<&'static str> {
    (0..=SENTINEL.len() - 4)
        .map(|from| &SENTINEL[from..from + 4])
        .find(|part| text.contains(part))
}

/// The password is the sentinel many times over, more than a pipe holds, so
/// a program that does not read it makes the write fail. On every way `run`
/// can go wrong, with the password on a descriptor or in a variable, nothing
/// the CLI or the program prints holds any part of it.
#[test]
fn run_never_leaks_the_secret_into_output_or_errors() {
    let password = SENTINEL.repeat(10_000);
    let fixture = Fixture::new(&password);
    let missing = "/nonexistent/ferrispass-cli-run-test";
    let id = fixture.id.clone();
    let env_cases = [
        (
            "the program is missing, with --env",
            vec!["run", "--id", &id, "--env", "FP_PW=password", "--", missing],
            Some(7),
        ),
        (
            "the program exits, with --env",
            vec![
                "run",
                "--id",
                &id,
                "--env",
                "FP_PW=password",
                "--",
                "/bin/sh",
                "-c",
                "exit 0",
            ],
            Some(0),
        ),
        (
            "the field goes on a descriptor and into a variable",
            vec![
                "run",
                "--id",
                &id,
                "--secret-fd",
                "3:password",
                "--env",
                "FP_PW=password",
                "--",
                "/bin/sh",
            ],
            Some(6),
        ),
        (
            "the variable name is invalid",
            vec!["run", "--id", &id, "--env", "1X=password", "--", "/bin/sh"],
            Some(6),
        ),
        (
            "the variable is named twice",
            vec![
                "run",
                "--id",
                &id,
                "--env",
                "FP_PW=password",
                "--env",
                "FP_PW=totp",
                "--",
                "/bin/sh",
            ],
            Some(6),
        ),
        (
            "the field in --env is unknown",
            vec!["run", "--id", &id, "--env", "FP_PW=secret", "--", "/bin/sh"],
            Some(6),
        ),
    ];
    let fd_cases = [
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
    for (what, args, code) in fd_cases.iter().chain(&env_cases) {
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

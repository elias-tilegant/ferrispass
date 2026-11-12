//! Headless, machine-readable FerrisPass command line interface.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
};

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

use crate::{
    biometric::{BiometricRegistry, BiometricStore, RetrieveOptions},
    domain::{CustomField, VaultEntry, VaultGroup},
    keepass::{EntryDraft, KeePassRepository, VaultDocument},
};

const SCHEMA: &str = "ferrispass-cli/v1";

/// The Touch ID reason for every command but `run`, which names what it
/// releases instead.
const UNLOCK_REASON: &str = "Unlock FerrisPass vault";

#[derive(Parser)]
#[command(
    name = "ferrispass-cli",
    version,
    about = "Secure headless access to FerrisPass vaults"
)]
struct Cli {
    #[arg(long, global = true, value_name = "FILE")]
    vault: Option<PathBuf>,
    #[arg(long, global = true, value_name = "FILE")]
    key_file: Option<PathBuf>,
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Human)]
    format: OutputFormat,
    #[arg(long, global = true, conflicts_with = "touch_id", value_name = "FD")]
    master_password_fd: Option<u32>,
    #[arg(long, global = true)]
    touch_id: bool,
    /// Never use an enrolled Touch ID identity, even on a terminal. For a
    /// script that must not block on a biometric prompt.
    #[arg(long, global = true, conflicts_with = "touch_id")]
    no_touch_id: bool,
    #[arg(long, global = true, requires = "touch_id")]
    allow_device_passcode: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
enum OutputFormat {
    Human,
    Json,
}

#[derive(Subcommand)]
enum Command {
    Vault {
        #[command(subcommand)]
        command: VaultCommand,
    },
    Group {
        #[command(subcommand)]
        command: GroupCommand,
    },
    Entry {
        #[command(subcommand)]
        command: EntryCommand,
    },
    Sync {
        #[command(subcommand)]
        command: SyncCommand,
    },
    /// Launch an entry in its native external application.
    Launch {
        #[command(subcommand)]
        command: LaunchCommand,
    },
    /// Start a program with secrets of one entry on file descriptors, never
    /// in its arguments or a file, and in its environment only with --env.
    Run(RunArgs),
}

#[derive(Args)]
struct RunArgs {
    /// Exact KeePass entry UUID whose fields the program gets.
    #[arg(long)]
    id: String,
    /// FD:FIELD, for example 3:password. The program reads FIELD on
    /// descriptor FD, 3 to 255. FIELD is password, totp or custom:KEY. Repeat
    /// for more fields, each on its own descriptor.
    #[arg(
        long = "secret-fd",
        value_name = "FD:FIELD",
        required_unless_present = "env"
    )]
    secret_fd: Vec<String>,
    /// NAME=FIELD, for example API_TOKEN=password, for a program that cannot
    /// read a descriptor: the program gets FIELD in the environment variable
    /// NAME. Only this program gets it, but programs running as your user
    /// can read another program's environment, so run prints a warning for
    /// each. Prefer --secret-fd.
    #[arg(long = "env", value_name = "NAME=FIELD")]
    env: Vec<String>,
    /// The program and its arguments, after `--`. It is started directly,
    /// not through a shell.
    #[arg(last = true, required = true, value_name = "PROGRAM")]
    command: Vec<OsString>,
}

#[derive(Subcommand)]
enum LaunchCommand {
    /// Start a logged-in SAP GUI session from an entry's SAP_* fields.
    Sap(LaunchEntry),
}

#[derive(Args)]
struct LaunchEntry {
    /// Exact KeePass entry UUID to launch.
    #[arg(long)]
    id: String,
}

#[derive(Subcommand)]
enum SyncCommand {
    Status,
    Now(SyncNow),
}

#[derive(Args)]
struct SyncNow {
    #[arg(long)]
    commit: bool,
    #[arg(long, requires = "commit")]
    plan_token: Option<String>,
    #[arg(long, default_value_t = 0)]
    input_fd: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SyncResolutions {
    resolutions: Vec<SyncResolution>,
    /// The database's own settings are one decision for the whole file, not
    /// one per object, so they get a field of their own rather than a
    /// resolution with a made-up id.
    #[serde(default)]
    metadata: Option<ResolutionSide>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SyncResolution {
    /// Exactly one of these. A conflict is about an entry or about a group,
    /// and the two are separate id spaces.
    #[serde(default)]
    entry_id: Option<String>,
    #[serde(default)]
    group_id: Option<String>,
    keep: ResolutionSide,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ResolutionSide {
    Local,
    Remote,
}

#[derive(Subcommand)]
enum VaultCommand {
    Info,
}

#[derive(Subcommand)]
enum GroupCommand {
    List,
    Create(GroupCreate),
    Rename(GroupRename),
    Move(GroupMove),
    Trash(MutationId),
    Restore(MutationId),
}

#[derive(Args)]
struct GroupCreate {
    #[arg(long)]
    parent_id: String,
    #[arg(long)]
    name: String,
    #[arg(long)]
    commit: bool,
}
#[derive(Args)]
struct GroupRename {
    #[arg(long)]
    id: String,
    #[arg(long)]
    name: String,
    #[arg(long)]
    commit: bool,
}
#[derive(Args)]
struct GroupMove {
    #[arg(long)]
    id: String,
    #[arg(long)]
    group_id: String,
    #[arg(long)]
    commit: bool,
}
#[derive(Args)]
struct MutationId {
    #[arg(long)]
    id: String,
    #[arg(long)]
    commit: bool,
}

#[derive(Subcommand)]
enum EntryCommand {
    List(EntryList),
    Search(EntrySearch),
    Get(EntryId),
    Secret(EntrySecret),
    Create(EntryCreate),
    Update(EntryUpdate),
    Move(EntryMove),
    Favorite(EntryFavorite),
    Trash(MutationId),
    Restore(MutationId),
}

#[derive(Args)]
struct EntryList {
    #[arg(long)]
    group_id: Option<String>,
    #[arg(long)]
    include_trash: bool,
}
#[derive(Args)]
struct EntrySearch {
    query: String,
    #[arg(long)]
    include_trash: bool,
}
#[derive(Args)]
struct EntryId {
    #[arg(long)]
    id: String,
}
#[derive(Args)]
struct EntrySecret {
    #[arg(long)]
    id: String,
    #[arg(long)]
    field: String,
    #[arg(long)]
    reveal: bool,
}
#[derive(Args)]
struct EntryCreate {
    #[arg(long)]
    group_id: String,
    #[arg(long, default_value_t = 0)]
    input_fd: u32,
    #[arg(long)]
    commit: bool,
}
#[derive(Args)]
struct EntryUpdate {
    #[arg(long)]
    id: String,
    #[arg(long, default_value_t = 0)]
    input_fd: u32,
    #[arg(long)]
    commit: bool,
}
#[derive(Args)]
struct EntryMove {
    #[arg(long)]
    id: String,
    #[arg(long)]
    group_id: String,
    #[arg(long)]
    commit: bool,
}
#[derive(Args)]
struct EntryFavorite {
    #[arg(long)]
    id: String,
    #[arg(long, action = clap::ArgAction::Set)]
    set: bool,
    #[arg(long)]
    commit: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryInput {
    #[serde(default)]
    title: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    otp: String,
    #[serde(default)]
    custom_fields: Vec<CustomFieldInput>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CustomFieldInput {
    key: String,
    value: String,
    #[serde(default)]
    protected: bool,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct EntryPatch {
    #[serde(default, deserialize_with = "patch_value")]
    title: Patch<String>,
    #[serde(default, deserialize_with = "patch_value")]
    username: Patch<String>,
    #[serde(default, deserialize_with = "patch_value")]
    password: Patch<String>,
    #[serde(default, deserialize_with = "patch_value")]
    url: Patch<String>,
    #[serde(default, deserialize_with = "patch_value")]
    notes: Patch<String>,
    #[serde(default, deserialize_with = "patch_value")]
    otp: Patch<String>,
    #[serde(default, deserialize_with = "patch_value")]
    tags: Patch<Vec<String>>,
    #[serde(default, deserialize_with = "patch_value")]
    custom_fields: Patch<Vec<CustomFieldInput>>,
}

#[derive(Default)]
enum Patch<T> {
    #[default]
    Missing,
    Clear,
    Set(T),
}
fn patch_value<'de, D, T>(d: D) -> Result<Patch<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(match Option::<T>::deserialize(d)? {
        Some(v) => Patch::Set(v),
        None => Patch::Clear,
    })
}

#[derive(Debug)]
struct CliError {
    code: &'static str,
    message: String,
    exit: i32,
}
impl CliError {
    fn new(code: &'static str, exit: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            exit,
            message: message.into(),
        }
    }
}

pub fn run() -> i32 {
    // Only the GUI ever swept the launch tempdir, so a CLI `launch` killed
    // mid-flight left a 0600 file holding a cleartext password behind with
    // nothing to collect it. Cheap, and it runs before any vault is opened.
    crate::launch::sweeper::sweep_stale(std::time::Duration::from_secs(60));
    // Before any command runs, so `launch` cannot stage a cleartext payload
    // into a window where Ctrl+C still kills the process outright.
    INTERRUPT_CLEANUP_ARMED.store(arm_interrupt_cleanup(), Ordering::Release);

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let _ = error.print();
            return 2;
        }
    };
    // `run` prints nothing of its own on success: stdout belongs to the
    // program it starts, and the exit code is the program's.
    let outcome = match &cli.command {
        Command::Run(args) => execute_run(&cli, args),
        _ => execute(&cli).map(|out| {
            print_success(cli.format, out);
            0
        }),
    };
    match outcome {
        Ok(code) => code,
        Err(error) => {
            print_error(cli.format, &error);
            error.exit
        }
    }
}

fn execute(cli: &Cli) -> Result<Value, CliError> {
    let vault = cli
        .vault
        .as_deref()
        .ok_or_else(|| CliError::new("missing_vault", 2, "--vault is required"))?;
    if matches!(
        &cli.command,
        Command::Sync {
            command: SyncCommand::Status
        }
    ) {
        return sync_status(vault);
    }
    let mut document = open_vault(
        cli,
        vault,
        UNLOCK_REASON,
        crate::biometric::default_store,
        crate::biometric::registry::load_or_default,
    )?;
    match &cli.command {
        Command::Vault {
            command: VaultCommand::Info,
        } => Ok(vault_info(&document)),
        Command::Group { command } => execute_group(command, &mut document, vault),
        Command::Entry { command } => execute_entry(command, &mut document, vault),
        Command::Launch { command } => execute_launch(command, &document),
        Command::Sync {
            command: SyncCommand::Now(args),
        } => execute_sync(args, &mut document, vault),
        Command::Sync {
            command: SyncCommand::Status,
        }
        | Command::Run(_) => unreachable!(),
    }
}

/// Unlocks the vault and opens it. `reason` is what a Touch ID prompt
/// says; `store` and `registry` are where Touch ID comes from, so a test can
/// put a fake there.
fn open_vault(
    cli: &Cli,
    vault: &Path,
    reason: &str,
    store: impl Fn() -> Arc<dyn BiometricStore>,
    registry: impl FnOnce() -> BiometricRegistry,
) -> Result<VaultDocument, CliError> {
    // The CLI cannot ask, so it honours what the user confirmed in the app
    // and otherwise reports the cost question as a plain failure.
    crate::keepass::limits::set_confirmed_kdf_costs(
        &crate::app::settings::load().confirmed_kdf_costs,
    );
    // Local to this function, so it is wiped as soon as the vault is open
    // rather than living through the rest of the command: a `launch` waits
    // ten seconds for the target app, a `sync` makes network calls, and a
    // `run` waits for its program. The GUI does the same, and SECURITY.md
    // says so for both.
    let password = unlock_password(cli, reason, store, registry)?;
    KeePassRepository::open(vault, &password, cli.key_file.as_deref())
        .map_err(|e| CliError::new("unlock_failed", 3, e.to_string()))
}

/// `run`: starts the program with the released fields on their descriptors,
/// and with `--env` in their variables, and returns its exit code. What the
/// command line asks for is checked before the vault is unlocked, and the
/// vault is closed again before the program starts, so only the released
/// values outlive the unlock: those on descriptors until their writers are
/// done, those for variables until the program starts.
#[cfg(unix)]
fn execute_run(cli: &Cli, args: &RunArgs) -> Result<i32, CliError> {
    run_program(
        cli,
        args,
        crate::biometric::default_store,
        crate::biometric::registry::load_or_default,
    )
}

/// [`execute_run`] with the Touch ID store and registry as parameters.
#[cfg(unix)]
fn run_program(
    cli: &Cli,
    args: &RunArgs,
    store: impl Fn() -> Arc<dyn BiometricStore>,
    registry: impl FnOnce() -> BiometricRegistry,
) -> Result<i32, CliError> {
    use std::os::unix::ffi::OsStrExt as _;

    let vault = cli
        .vault
        .as_deref()
        .ok_or_else(|| CliError::new("missing_vault", 2, "--vault is required"))?;
    let entry = uuid::Uuid::parse_str(&args.id)
        .map_err(|_| CliError::new("invalid_input", 6, "--id must be an entry UUID"))?;
    let released = released_fields(args)?;
    let (program, arguments) = args
        .command
        .split_first()
        .ok_or_else(|| CliError::new("invalid_input", 6, "run needs a program after --"))?;
    // The descriptor the master password comes from is the caller's and has
    // no close-on-exec, so the program would inherit it and could read the
    // master password again from a file. The child closes it, and that is
    // safe only while it is open here, so a closed one stops `run` before
    // anything is unlocked. It then stays open, as `spawn_with_secret_fds`
    // needs, until this process exits.
    crate::platform::secret_fd::check_open(cli.master_password_fd).map_err(run_error)?;
    let reason = run_reason(program, vault, &entry, &released);
    let (on_descriptors, in_environment) = {
        let document = open_vault(cli, vault, &reason, store, registry)?;
        released_values(&document, &entry.hyphenated().to_string(), &released)?
    };
    let mut command = std::process::Command::new(program);
    command.args(arguments);
    // `Command` copies each value into an environment block of its own, which
    // nothing can wipe; the program's copy is its own as well. Only the
    // buffers here are wiped, when `in_environment` is dropped.
    for (name, value) in &in_environment {
        command.env(name, OsStr::from_bytes(value));
    }
    drop(in_environment);
    // A warning that cannot be written stops `run` here, before the program
    // starts, with the exit code for an I/O failure.
    for spec in &released.env {
        writeln!(
            std::io::stderr(),
            "warning: {} is set only for this program, not exported, and readable by \
             programs running as your user",
            spec.name
        )
        .map_err(|error| {
            let message = format!(
                "could not write the warning for --env, so the program did not start: {error}"
            );
            CliError::new("run_failed", 7, message)
        })?;
    }
    // From here until the program has been reaped, SIGINT and SIGTERM go on
    // to the program and this process keeps waiting, so a signal to the CLI
    // ends the program the way the program chooses and `run` still returns
    // its exit code. This replaces the Ctrl+C cleanup `launch` needs, which
    // has nothing to do here, and puts it back afterwards.
    let signals = crate::platform::secret_fd::forward_signals().map_err(run_error)?;
    let running = crate::platform::secret_fd::spawn_with_secret_fds(
        command,
        on_descriptors,
        cli.master_password_fd.as_slice(),
        Some(signals),
    )
    .map_err(run_error)?;
    let finished = running.wait().map_err(run_error)?;
    // Numbers only: which descriptor, how many bytes of how many.
    for undelivered in &finished.undelivered {
        eprintln!("warning: {undelivered}");
    }
    Ok(exit_code(finished.status))
}

#[cfg(not(unix))]
fn execute_run(_cli: &Cli, _args: &RunArgs) -> Result<i32, CliError> {
    Err(CliError::new(
        "unsupported",
        6,
        "run is unsupported on this platform",
    ))
}

/// What `run` releases: the fields `--secret-fd` puts on descriptors, then
/// the ones `--env` puts into variables, each in the order given.
#[cfg(unix)]
struct Released {
    fds: Vec<(u32, SecretField)>,
    env: Vec<EnvSpec>,
}

/// One `--env NAME=FIELD`.
#[cfg(unix)]
struct EnvSpec {
    name: String,
    field: SecretField,
}

#[cfg(unix)]
impl Released {
    /// Every released field, descriptors first, then variables.
    fn fields(&self) -> impl Iterator<Item = &SecretField> {
        self.fds
            .iter()
            .map(|(_, field)| field)
            .chain(self.env.iter().map(|spec| &spec.field))
    }
}

/// The fields `--secret-fd` and `--env` release, checked before anything is
/// unlocked: the syntax, the field names, each field at most once whichever
/// way it goes, each variable at most once and with a name a shell would
/// take, and the descriptors as the platform module wants them.
#[cfg(unix)]
fn released_fields(args: &RunArgs) -> Result<Released, CliError> {
    let invalid = |message: &'static str| CliError::new("invalid_input", 6, message);
    let mut released = Released {
        fds: Vec::with_capacity(args.secret_fd.len()),
        env: Vec::with_capacity(args.env.len()),
    };
    for spec in &args.secret_fd {
        let (target, field) = spec
            .split_once(':')
            .ok_or_else(|| invalid("--secret-fd takes FD:FIELD, for example 3:password"))?;
        let target: u32 = target
            .parse()
            .map_err(|_| invalid("the FD in --secret-fd must be a number from 3 to 255"))?;
        let field = SecretField::parse(field)?;
        if released.fields().any(|earlier| *earlier == field) {
            return Err(invalid("each field can go to the program only once"));
        }
        released.fds.push((target, field));
    }
    for spec in &args.env {
        let (name, field) = spec
            .split_once('=')
            .ok_or_else(|| invalid("--env takes NAME=FIELD, for example API_TOKEN=password"))?;
        if !is_variable_name(name) {
            return Err(invalid(
                "the NAME in --env has to start with a letter or _ and have only \
                 letters, digits and _",
            ));
        }
        let field = SecretField::parse(field)?;
        if released.fields().any(|earlier| *earlier == field) {
            return Err(invalid("each field can go to the program only once"));
        }
        if released.env.iter().any(|earlier| earlier.name == name) {
            return Err(invalid("each variable can be set only once"));
        }
        released.env.push(EnvSpec {
            name: name.to_owned(),
            field,
        });
    }
    crate::platform::secret_fd::check_targets(released.fds.iter().map(|(target, _)| *target))
        .map_err(|error| CliError::new("invalid_input", 6, error.to_string()))?;
    Ok(released)
}

/// `[A-Za-z_][A-Za-z0-9_]*`, the names a POSIX shell takes for variables.
#[cfg(unix)]
fn is_variable_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// The Touch ID reason for `run`: what goes to which program, from which
/// entry and vault. Everything in it is checked here, so a script cannot
/// write its own words into the dialog: the program's and the vault's file
/// names only when [`safe_name`] lets them through, the entry as the
/// canonical form of its UUID, and each field as a fixed phrase, whether it
/// goes on a descriptor or into a variable. A custom field's key, a
/// variable's name, an entry title or any other text from the command line
/// never gets in. Touch ID runs before the vault is open, so the entry's
/// title is not known yet.
#[cfg(unix)]
fn run_reason(program: &OsStr, vault: &Path, entry: &uuid::Uuid, released: &Released) -> String {
    format!(
        "Give {} {} of entry {} from {}",
        safe_name(Path::new(program), "a program"),
        released_phrase(released),
        entry.hyphenated(),
        safe_name(vault, "a vault"),
    )
}

/// The file name of `path` when it is 1 to 40 characters from `A-Z`, `a-z`,
/// `0-9`, `.`, `_` and `-`; otherwise `fallback`.
#[cfg(unix)]
fn safe_name<'a>(path: &'a Path, fallback: &'a str) -> &'a str {
    path.file_name()
        .and_then(OsStr::to_str)
        .filter(|name| {
            (1..=40).contains(&name.len())
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        })
        .unwrap_or(fallback)
}

/// Every released field as a fixed phrase, in the order of
/// [`Released::fields`]. Custom fields are counted, not named: "a custom
/// field", "two custom fields", at the place of the first one.
#[cfg(unix)]
fn released_phrase(released: &Released) -> String {
    let custom = released
        .fields()
        .filter(|field| matches!(field, SecretField::Custom(_)))
        .count();
    let mut phrases = Vec::new();
    let mut custom_named = false;
    for field in released.fields() {
        match field {
            SecretField::Password => phrases.push("the password".to_owned()),
            SecretField::Totp => phrases.push("the one-time code".to_owned()),
            SecretField::Custom(_) if !custom_named => {
                custom_named = true;
                phrases.push(custom_fields(custom));
            }
            SecretField::Custom(_) => {}
        }
    }
    match phrases.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// "a custom field", "two custom fields", up to ten in words, then digits.
#[cfg(unix)]
fn custom_fields(count: usize) -> String {
    const FROM_TWO: [&str; 9] = [
        "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    ];
    if count == 1 {
        return "a custom field".to_owned();
    }
    match count.checked_sub(2).and_then(|at| FROM_TWO.get(at)) {
        Some(word) => format!("{word} custom fields"),
        None => format!("{count} custom fields"),
    }
}

/// The released values: each with the descriptor it goes to, and each with
/// the variable it goes into.
#[cfg(unix)]
type ReleasedValues = (
    Vec<(u32, Zeroizing<Vec<u8>>)>,
    Vec<(String, Zeroizing<Vec<u8>>)>,
);

/// The values of the released fields, moved into wiped buffers without a
/// copy.
#[cfg(unix)]
fn released_values(
    document: &VaultDocument,
    id: &str,
    released: &Released,
) -> Result<ReleasedValues, CliError> {
    if document.snapshot().find_entry(id).is_none() {
        return Err(not_found());
    }
    let value = |field: &SecretField| -> Result<Zeroizing<Vec<u8>>, CliError> {
        let mut value = secret_value(document, id, field)?;
        Ok(Zeroizing::new(std::mem::take(&mut *value).into_bytes()))
    };
    let on_descriptors = released
        .fds
        .iter()
        .map(|(target, field)| Ok((*target, value(field)?)))
        .collect::<Result<_, CliError>>()?;
    let in_environment = released
        .env
        .iter()
        .map(|spec| Ok((spec.name.clone(), value(&spec.field)?)))
        .collect::<Result<_, CliError>>()?;
    Ok((on_descriptors, in_environment))
}

#[cfg(unix)]
fn run_error(error: crate::platform::secret_fd::SecretFdError) -> CliError {
    use crate::platform::secret_fd::SecretFdError;
    match error {
        SecretFdError::BelowThree(_)
        | SecretFdError::AboveLimit(_)
        | SecretFdError::Duplicate(_)
        | SecretFdError::NotOpen(_) => CliError::new("invalid_input", 6, error.to_string()),
        _ => CliError::new("run_failed", 7, error.to_string()),
    }
}

/// The program's exit code, or 128 plus the signal that ended it, as a shell
/// reports it.
#[cfg(unix)]
fn exit_code(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt as _;
    status
        .code()
        .or_else(|| status.signal().map(|signal| 128 + signal))
        .unwrap_or(1)
}

fn execute_launch(command: &LaunchCommand, document: &VaultDocument) -> Result<Value, CliError> {
    let (target, args) = match command {
        LaunchCommand::Sap(args) => (crate::launch::LaunchTarget::Sap, args),
    };
    let entry = document
        .snapshot()
        .find_entry(&args.id)
        .cloned()
        .ok_or_else(not_found)?;
    if entry.in_recycle_bin {
        return Err(CliError::new(
            "entry_in_trash",
            6,
            "entries in the Recycle Bin cannot be launched",
        ));
    }
    let password = document.password_for_entry(&args.id);
    let context = crate::launch::LaunchContext {
        entry: &entry,
        password: password.as_deref().map(String::as_str),
        custom_fields: &entry.custom_fields,
    };
    if !INTERRUPT_CLEANUP_ARMED.load(Ordering::Acquire) {
        // Not fatal: the launch still works and the file is still unlinked on
        // the ordinary path. But the user is about to have a cleartext
        // password on disk for ten seconds with only the next start's sweep
        // behind it, and that is worth one line on stderr rather than
        // silence. stdout stays reserved for the JSON envelope.
        eprintln!(
            "warning: could not install the interrupt handler; \
             Ctrl+C during the launch will leave the staged file until the next start"
        );
    }
    let handle = crate::launch::launch(target, context).map_err(launch_error)?;

    // `open` returns after handing the file to Launch Services, before SAP GUI
    // necessarily reads it. Keep ownership (and thus the 0600 payload) alive
    // for the same minimum grace period used by the GUI settings clamp.
    //
    // Ctrl+C during that window skips `Drop`, and the file holds a cleartext
    // password. The startup sweep in `run` only collects it on the next
    // FerrisPass start, which may be days later, so the signal unlinks it now.
    std::thread::sleep(std::time::Duration::from_secs(10));
    drop(handle);
    Ok(json!({"launched":true,"target":target.id(),"entry_id":args.id}))
}

/// Install the interrupt handler, before anything can be staged.
///
/// The handler asks the launch module which payloads this process currently
/// owns, rather than being told after the fact: the file exists from the
/// moment it is created, and anything that registers it afterwards leaves a
/// window in which a cleartext password is on disk and the default SIGINT
/// disposition still applies.
///
/// `ctrlc` runs the closure on a thread of its own rather than inside the
/// signal context, so unlinking a file there is allowed. It can be installed
/// once per process, which suits a binary that stages one payload per run.
/// `run` stages nothing; while it waits for its program, the handler from
/// `secret_fd::forward_signals` takes SIGINT instead, and this one is put
/// back once the program has ended.
///
/// Returns whether cleanup is armed. A failure is not fatal, but it is not
/// silent either: the sixty-second sweep at the next start becomes the only
/// backstop, and `launch` says so before it stages anything.
/// Whether [`arm_interrupt_cleanup`] succeeded, so `launch` can warn before
/// it writes a password to disk with no interrupt cleanup behind it.
static INTERRUPT_CLEANUP_ARMED: AtomicBool = AtomicBool::new(false);

fn arm_interrupt_cleanup() -> bool {
    ctrlc::set_handler(|| {
        remove_staged_launch_files();
        // 128 + SIGINT, the shell's convention for "killed by signal 2".
        std::process::exit(130);
    })
    .is_ok()
}

/// Unlink every payload this process still owns.
///
/// Deliberately not the whole launch directory: another FerrisPass process
/// stages its own files there and this one does not own those.
fn remove_staged_launch_files() {
    for path in crate::launch::tempfile::live_staged_files() {
        // Best effort: the grace period may have unlinked it already, and
        // naming the path in an error would defeat the point of removing it.
        let _ = std::fs::remove_file(path);
    }
}

fn launch_error(error: crate::launch::LaunchError) -> CliError {
    match error {
        crate::launch::LaunchError::MissingField(field) => CliError::new(
            "invalid_launch_profile",
            6,
            format!("missing required field: {field}"),
        ),
        crate::launch::LaunchError::NoPassword => {
            CliError::new("invalid_launch_profile", 6, "entry has no password")
        }
        crate::launch::LaunchError::UnsupportedTarget(target) => CliError::new(
            "launch_unsupported",
            6,
            format!("{target} launch is unsupported on this platform"),
        ),
        crate::launch::LaunchError::UnsupportedCharacter { field } => CliError::new(
            "invalid_launch_profile",
            6,
            format!("the {field} field contains a character this connection file cannot carry"),
        ),
        crate::launch::LaunchError::Io(error) => CliError::new(
            "launch_failed",
            7,
            format!("launch failed: {}", error.kind()),
        ),
    }
}

fn sync_status(vault: &Path) -> Result<Value, CliError> {
    let canonical =
        std::fs::canonicalize(vault).map_err(|e| CliError::new("io", 7, e.to_string()))?;
    let config = crate::sync::config::load(&canonical)
        .map_err(sync_error)?
        .ok_or_else(|| {
            CliError::new(
                "sync_not_configured",
                6,
                "vault is not connected to a sync provider",
            )
        })?;
    Ok(json!({
        "provider": config.provider.id(),
        "provider_name": config.provider.display_name(),
        "account": config.account_email,
        "remote_url": config.remote_url,
        "configured": true,
        "network_checked": false
    }))
}

fn execute_sync(
    args: &SyncNow,
    document: &mut VaultDocument,
    vault: &Path,
) -> Result<Value, CliError> {
    let canonical =
        std::fs::canonicalize(vault).map_err(|e| CliError::new("io", 7, e.to_string()))?;
    let mut config = crate::sync::lock::retrying(
        || crate::sync::config::load(&canonical),
        crate::sync::config::ConfigError::is_busy,
    )
    .map_err(|error| {
        if error.is_busy() {
            busy_error()
        } else {
            sync_error(error)
        }
    })?
    .ok_or_else(|| {
        CliError::new(
            "sync_not_configured",
            6,
            "vault is not connected to a sync provider",
        )
    })?;
    // What the binding looked like before this command touched it.
    // `restore_provider` refreshes an iCloud bookmark in memory, so comparing
    // our own copy against the file later would report a change nobody made.
    let baseline = config.clone();
    let token = crate::sync::service::restore_provider(&mut config).map_err(sync_service_error)?;
    let local_bytes = document
        .read_current_bytes()
        .map_err(|e| CliError::new("local_revision_changed", 5, e.to_string()))?;
    let (remote_bytes, remote_etag) =
        crate::sync::service::download_remote(&config, &token).map_err(sync_error)?;
    // Through the key the open vault already holds, not the password: that
    // is what the GUI does, and it is what lets the password itself be wiped
    // as soon as the vault is open rather than kept for the whole command.
    let remote =
        crate::keepass::repository::parse_database_bytes(&remote_bytes, document.database_key())
            .map_err(|e| CliError::new("remote_unlock_failed", 3, e.to_string()))?;
    let report = crate::keepass::merge::diff(document.database(), &remote);
    let conflicts: Vec<Value> = report
        .conflicts
        .iter()
        .map(|conflict| {
            let fields: Vec<&str> = conflict
                .fields
                .iter()
                .filter(|field| field.differs)
                .map(|field| field.label.as_ref())
                .collect();
            json!({"entry_id":conflict.id,"fields":fields})
        })
        .collect();
    let group_conflicts: Vec<Value> = report
        .group_conflicts
        .iter()
        .map(|conflict| {
            let fields: Vec<&str> = conflict
                .fields
                .iter()
                .filter(|field| field.differs)
                .map(|field| field.label.as_ref())
                .collect();
            json!({
                "group_id":conflict.id,
                "name":conflict.name,
                "path":conflict.path,
                "fields":fields,
            })
        })
        .collect();
    let metadata_conflict = report.metadata_conflict.as_ref().map(|conflict| {
        let fields: Vec<&str> = conflict
            .fields
            .iter()
            .filter(|field| field.differs)
            .map(|field| field.label.as_ref())
            .collect();
        json!({ "fields": fields })
    });
    let plan_token = sync_plan_token(&local_bytes, &remote_etag, &report);
    let needs_upload = report.has_local_contribution();
    let remote_changes = report.remote_only.len()
        + report
            .auto_resolved
            .iter()
            .filter(|r| matches!(r.winner, crate::keepass::merge::Side::Remote))
            .count();
    if !args.commit {
        let undecided =
            !conflicts.is_empty() || !group_conflicts.is_empty() || metadata_conflict.is_some();
        return Ok(json!({
            "status":if undecided {"conflict"} else {"ready"},
            "committed":false,
            "plan_token":plan_token,
            "would_upload":needs_upload || undecided,
            "remote_changes":remote_changes,
            "conflicts":conflicts,
            "group_conflicts":group_conflicts,
            "metadata_conflict":metadata_conflict
        }));
    }
    let supplied = args
        .plan_token
        .as_deref()
        .ok_or_else(|| CliError::new("plan_token_required", 6, "--commit requires --plan-token"))?;
    if supplied != plan_token {
        return Err(CliError::new(
            "stale_sync_plan",
            5,
            "local or remote revision changed; create a new sync plan",
        ));
    }
    let picks = validated_resolutions(args.input_fd, &report)?;
    let merged_count = report.remote_only.len() + report.auto_resolved.len();
    let merged = crate::keepass::merge::apply_picks(document.database(), &remote, &picks, &report)
        .map_err(|e| CliError::new("merge_failed", 5, e.to_string()))?;
    // Ask the merged result, not the report that predicted it. Every
    // predicate here is a claim about what `apply_picks` will do, and a claim
    // that falls short means writing the pre-merge bytes over a remote that
    // held the only copy of something. The comparison is one deep equality
    // over two in-memory databases, next to nothing beside the KDF this
    // command has already paid for.
    let needs_local_save = &merged != document.database();
    let upload_bytes = if needs_local_save {
        let receipt = document
            .save_payload_for_database(merged.clone())
            .save_to(vault)
            .map_err(|e| CliError::new("save_failed", 7, e.to_string()))?;
        document.replace_database(merged);
        receipt.bytes()
    } else {
        local_bytes
    };
    config.last_etag = remote_etag;
    // What the file on disk now holds. The GUI compares this against the file
    // on its next tick to tell "nobody else wrote" from "our own write never
    // happened", so a CLI sync that left it stale made the next GUI tick
    // upload for no reason. It is recorded whether or not we upload: after a
    // pure pull the merged file is the new baseline, and after an upload it
    // is what we sent.
    config.uploaded_local_revision = Some(crate::sync::config::local_revision(&upload_bytes));
    let resolved_upload = needs_upload || report.needs_a_decision();
    // Nothing left to send to a relationship that has ended. Asked here so a
    // binding the user disconnected while the merge ran costs no transfer,
    // and asked again below, because only the second answer is adjacent to
    // the write it guards.
    //
    // The comparison is against the binding as it was when this command
    // started, not against our copy: `restore_provider` refreshes an iCloud
    // bookmark in memory, and comparing that would report a change nobody
    // else made.
    locked_briefly(|| binding_is_still(&canonical, &baseline))?;
    if resolved_upload {
        match crate::sync::service::upload_after_save(&config, &token, &upload_bytes)
            .map_err(sync_error)?
        {
            crate::sync::service::UploadAfterSave::Synced { new_etag, .. } => {
                config.last_etag = new_etag;
            }
            crate::sync::service::UploadAfterSave::Conflict { .. } => {
                return Err(CliError::new(
                    "remote_changed_during_sync",
                    5,
                    "remote vault changed again; rerun sync",
                ));
            }
        }
    }
    // The re-check and the write are one operation: between them is the only
    // window where the app could bind this vault elsewhere and have this
    // command write over it. The upload deliberately sits outside, because
    // the app waits this lock out on a fixed budget and a transfer can
    // outlast any budget worth giving it.
    locked_briefly(|| {
        binding_is_still(&canonical, &baseline)?;
        crate::sync::config::save_unlocked(&config).map_err(sync_error)
    })?;
    let resolved = picks.entries.len() + picks.groups.len() + usize::from(picks.metadata.is_some());
    Ok(
        json!({"status":"synced","committed":true,"uploaded":resolved_upload,"merged":merged_count,"resolved":resolved}),
    )
}

/// Take the cross-process sync lock for one read or one write, and report a
/// failure to take it as itself: a script that met another FerrisPass process
/// can wait and rerun, and one that met a broken lock file cannot.
fn locked_briefly<T>(f: impl FnOnce() -> Result<T, CliError>) -> Result<T, CliError> {
    match crate::sync::lock::held(crate::sync::lock::BATCH, f) {
        Ok(outcome) => outcome,
        Err(crate::sync::lock::Unavailable::Contended) => Err(CliError::new(
            "sync_busy",
            5,
            "another FerrisPass process is using the sync configuration; the \
             merge is saved locally, rerun sync to send it",
        )),
        Err(crate::sync::lock::Unavailable::Broken(error)) => Err(sync_error(error)),
    }
}

/// Whether the vault is still bound to the relationship this command started
/// from. Reads without the lock, so every caller is already holding it.
fn binding_is_still(
    canonical: &Path,
    baseline: &crate::sync::config::SyncConfig,
) -> Result<(), CliError> {
    let current = crate::sync::config::load_unlocked(canonical).map_err(sync_error)?;
    if current.is_some_and(|current| crate::sync::config::same_relationship(&current, baseline)) {
        return Ok(());
    }
    Err(CliError::new(
        "sync_binding_changed",
        5,
        "the vault's sync binding changed while this command ran; the merge \
         is saved locally, rerun sync to send it",
    ))
}

fn sync_plan_token(
    local: &[u8],
    remote_etag: &str,
    report: &crate::keepass::merge::ConflictReport,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"ferrispass-cli-sync-plan-v1\0");
    hasher.update(Sha256::digest(local));
    hasher.update(remote_etag.as_bytes());
    for conflict in &report.conflicts {
        hasher.update(b"entry\0");
        hasher.update(conflict.id.as_bytes());
        for field in conflict.fields.iter().filter(|field| field.differs) {
            hasher.update(field.label.as_bytes());
        }
    }
    // Groups belong in the token for the same reason entries do: the commit
    // that quotes this token must be answering the plan the user was shown.
    for conflict in &report.group_conflicts {
        hasher.update(b"group\0");
        hasher.update(conflict.id.as_bytes());
        for field in conflict.fields.iter().filter(|field| field.differs) {
            hasher.update(field.label.as_bytes());
        }
    }
    // Same reason again: a decision about the database settings is part of
    // the plan the user answered.
    if let Some(conflict) = &report.metadata_conflict {
        hasher.update(b"metadata\0");
        for field in conflict.fields.iter().filter(|field| field.differs) {
            hasher.update(field.label.as_bytes());
        }
    }
    format!(
        "v1:{}",
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn validated_resolutions(
    fd: u32,
    report: &crate::keepass::merge::ConflictReport,
) -> Result<crate::keepass::merge::Resolutions, CliError> {
    if !report.needs_a_decision() {
        return Ok(crate::keepass::merge::Resolutions::default());
    }
    let input: SyncResolutions = read_json_fd(fd)?;
    let expected_entries: std::collections::HashSet<String> =
        report.conflicts.iter().map(|c| c.id.clone()).collect();
    let expected_groups: std::collections::HashSet<String> = report
        .group_conflicts
        .iter()
        .map(|c| c.id.clone())
        .collect();
    validate_resolution_input(
        input,
        &expected_entries,
        &expected_groups,
        report.metadata_conflict.is_some(),
    )
}

fn validate_resolution_input(
    input: SyncResolutions,
    expected_entries: &std::collections::HashSet<String>,
    expected_groups: &std::collections::HashSet<String>,
    expects_metadata: bool,
) -> Result<crate::keepass::merge::Resolutions, CliError> {
    let invalid = |message: &'static str| CliError::new("invalid_resolution", 6, message);
    let mut picks = crate::keepass::merge::Resolutions::default();
    for resolution in input.resolutions {
        let side = match resolution.keep {
            ResolutionSide::Local => crate::keepass::merge::Side::Local,
            ResolutionSide::Remote => crate::keepass::merge::Side::Remote,
        };
        let (map, expected, id) = match (resolution.entry_id, resolution.group_id) {
            (Some(id), None) => (&mut picks.entries, expected_entries, id),
            (None, Some(id)) => (&mut picks.groups, expected_groups, id),
            _ => {
                return Err(invalid(
                    "each resolution needs exactly one of entry_id or group_id",
                ));
            }
        };
        if !expected.contains(&id) {
            return Err(invalid("resolution contains an unknown UUID"));
        }
        if map.insert(id, side).is_some() {
            return Err(invalid("resolution contains a duplicate UUID"));
        }
    }
    match (input.metadata, expects_metadata) {
        (Some(side), true) => {
            picks.metadata = Some(match side {
                ResolutionSide::Local => crate::keepass::merge::Side::Local,
                ResolutionSide::Remote => crate::keepass::merge::Side::Remote,
            });
        }
        (Some(_), false) => {
            return Err(invalid(
                "resolution decides database settings that are not in conflict",
            ));
        }
        (None, _) => {}
    }
    if picks.entries.len() != expected_entries.len()
        || picks.groups.len() != expected_groups.len()
        || picks.metadata.is_some() != expects_metadata
    {
        return Err(CliError::new(
            "incomplete_resolution",
            6,
            "every conflict requires exactly one resolution",
        ));
    }
    Ok(picks)
}

fn sync_error(error: impl std::fmt::Display) -> CliError {
    CliError::new("sync_failed", 7, error.to_string())
}

/// The same, but keeping `sync_busy` distinguishable: a script that meets
/// another FerrisPass process should be able to wait and try again, and a
/// generic failure code tells it to give up instead.
fn sync_service_error(error: crate::sync::service::ServiceError) -> CliError {
    if error.is_busy() {
        return busy_error();
    }
    sync_error(error)
}

fn busy_error() -> CliError {
    CliError::new(
        "sync_busy",
        5,
        "another FerrisPass process is using the sync configuration; try again",
    )
}

fn unlock_password(
    cli: &Cli,
    reason: &str,
    store: impl Fn() -> Arc<dyn BiometricStore>,
    registry: impl FnOnce() -> BiometricRegistry,
) -> Result<Zeroizing<String>, CliError> {
    if let Some(fd) = cli.master_password_fd {
        return read_secret_fd(fd);
    }
    let vault = cli
        .vault
        .as_deref()
        .ok_or_else(|| CliError::new("missing_vault", 2, "--vault is required"))?;
    let canonical =
        std::fs::canonicalize(vault).map_err(|e| CliError::new("io", 7, e.to_string()))?;
    let registry = registry();
    let enrollment = registry.get(&canonical).or_else(|| registry.get(vault));
    // Biometry needs a person in front of the machine. Preferring it whenever
    // an enrolment existed meant a scripted invocation raised an OS prompt and
    // blocked on it for up to two minutes with nobody there to answer, which
    // made agent runs nondeterministic. Ask only when explicitly requested, or
    // when stdin is a terminal and so someone is plausibly watching.
    let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let wants_biometrics = cli.touch_id
        || (!cli.no_touch_id && interactive && enrollment.is_some() && store().is_available());
    if wants_biometrics {
        let enrollment = enrollment.ok_or_else(|| {
            CliError::new(
                "touch_id_not_enrolled",
                3,
                "Touch ID is not enrolled for this vault",
            )
        })?;
        return store()
            .retrieve(
                &enrollment.id,
                reason,
                RetrieveOptions {
                    allow_device_passcode: cli.allow_device_passcode,
                },
            )
            .map_err(|e| CliError::new("touch_id_failed", 3, e.to_string()));
    }
    rpassword::prompt_password("Master password: ")
        .map(Zeroizing::new)
        .map_err(|e| CliError::new("password_input_failed", 3, e.to_string()))
}

fn read_secret_fd(fd: u32) -> Result<Zeroizing<String>, CliError> {
    if fd < 3 {
        return Err(CliError::new(
            "unsafe_fd",
            6,
            "secret file descriptors must be 3 or greater",
        ));
    }
    #[cfg(unix)]
    {
        let mut value = Zeroizing::new(String::new());
        File::open(format!("/dev/fd/{fd}"))
            .and_then(|mut file| file.read_to_string(&mut value).map(|_| ()))
            .map_err(|e| CliError::new("password_input_failed", 3, e.to_string()))?;
        while value.ends_with(['\n', '\r']) {
            value.pop();
        }
        Ok(value)
    }
    #[cfg(not(unix))]
    {
        let _ = fd;
        Err(CliError::new(
            "unsupported",
            6,
            "file-descriptor secrets are unsupported on this platform",
        ))
    }
}

fn read_json_fd<T: for<'de> Deserialize<'de>>(fd: u32) -> Result<T, CliError> {
    let mut input = Zeroizing::new(String::new());
    if fd == 0 {
        std::io::stdin().read_to_string(&mut input)
    } else {
        #[cfg(unix)]
        {
            File::open(format!("/dev/fd/{fd}")).and_then(|mut f| f.read_to_string(&mut input))
        }
        #[cfg(not(unix))]
        {
            return Err(CliError::new(
                "unsupported",
                6,
                "input file descriptors are unsupported",
            ));
        }
    }
    .map_err(|e| CliError::new("input_failed", 7, e.to_string()))?;
    serde_json::from_str(&input).map_err(|e| CliError::new("invalid_input", 6, e.to_string()))
}

fn vault_info(doc: &VaultDocument) -> Value {
    let s = doc.snapshot();
    json!({"entries":s.entry_count,"groups":s.group_count,"root_id":s.root.id,"root_name":s.root.name,"has_recycle_bin":s.recycle_bin_id.is_some()})
}

fn group_json(group: &VaultGroup, parent_id: Option<&str>) -> Value {
    json!({"id":group.id,"name":group.name,"parent_id":parent_id,"entries":group.entries.len(),"groups":group.groups.len()})
}
fn collect_groups(group: &VaultGroup, parent: Option<&str>, out: &mut Vec<Value>) {
    out.push(group_json(group, parent));
    for child in &group.groups {
        collect_groups(child, Some(&group.id), out);
    }
}
fn entry_json(entry: &VaultEntry) -> Value {
    let custom: Vec<Value> = entry.custom_fields.iter().map(|f| json!({"key":f.key,"protected":f.protected,"value":if f.protected { Value::Null } else { Value::String(f.value.clone()) }})).collect();
    json!({"id":entry.id,"title":entry.title,"username":entry.username,"url":entry.url,"notes":entry.notes,"tags":entry.tags,"starred":entry.starred,"updated":entry.updated,"group_path":entry.group_path,"in_trash":entry.in_recycle_bin,"has_password":entry.has_password,"has_otp":entry.has_otp,"custom_fields":custom})
}

fn execute_group(
    cmd: &GroupCommand,
    doc: &mut VaultDocument,
    vault: &Path,
) -> Result<Value, CliError> {
    match cmd {
        GroupCommand::List => {
            let mut groups = Vec::new();
            collect_groups(&doc.snapshot().root, None, &mut groups);
            Ok(json!({"groups":groups}))
        }
        GroupCommand::Create(a) => mutation(doc, vault, a.commit, "group.create", |d| {
            d.create_group(&a.parent_id, &a.name)
                .map(|id| json!({"id":id}))
        }),
        GroupCommand::Rename(a) => mutation(doc, vault, a.commit, "group.rename", |d| {
            d.rename_group(&a.id, &a.name).map(|_| json!({"id":a.id}))
        }),
        GroupCommand::Move(a) => mutation(doc, vault, a.commit, "group.move", |d| {
            d.move_group(&a.id, &a.group_id)
                .map(|_| json!({"id":a.id,"group_id":a.group_id}))
        }),
        GroupCommand::Trash(a) => mutation(doc, vault, a.commit, "group.trash", |d| {
            d.delete_group(&a.id).map(|_| json!({"id":a.id}))
        }),
        GroupCommand::Restore(a) => mutation(doc, vault, a.commit, "group.restore", |d| {
            d.restore_group(&a.id).map(|_| json!({"id":a.id}))
        }),
    }
}

fn execute_entry(
    cmd: &EntryCommand,
    doc: &mut VaultDocument,
    vault: &Path,
) -> Result<Value, CliError> {
    match cmd {
        EntryCommand::List(a) => {
            let entries: Vec<Value> = if let Some(id) = &a.group_id {
                doc.snapshot()
                    .find_group(id)
                    .ok_or_else(not_found)?
                    .entries
                    .iter()
                    .filter(|e| a.include_trash || !e.in_recycle_bin)
                    .map(entry_json)
                    .collect()
            } else {
                doc.snapshot()
                    .entries_recursive()
                    .into_iter()
                    .filter(|e| a.include_trash || !e.in_recycle_bin)
                    .map(entry_json)
                    .collect()
            };
            Ok(json!({"entries":entries}))
        }
        EntryCommand::Search(a) => {
            let q = a.query.to_lowercase();
            let entries = doc
                .snapshot()
                .entries_recursive()
                .into_iter()
                .filter(|e| {
                    (a.include_trash || !e.in_recycle_bin)
                        && [&e.title, &e.username, &e.url]
                            .iter()
                            .any(|v| v.to_lowercase().contains(&q))
                })
                .map(entry_json)
                .collect::<Vec<_>>();
            Ok(json!({"entries":entries}))
        }
        EntryCommand::Get(a) => {
            Ok(json!({"entry":entry_json(doc.snapshot().find_entry(&a.id).ok_or_else(not_found)?)}))
        }
        EntryCommand::Secret(a) => secret(doc, a),
        EntryCommand::Create(a) => {
            let input: EntryInput = read_json_fd(a.input_fd)?;
            let draft = input.into_draft()?;
            mutation(doc, vault, a.commit, "entry.create", |d| {
                d.create_entry(&a.group_id, &draft)
                    .map(|id| json!({"id":id}))
            })
        }
        EntryCommand::Update(a) => {
            let patch: EntryPatch = read_json_fd(a.input_fd)?;
            let (draft, tags) = patched_draft(doc, &a.id, patch)?;
            mutation(doc, vault, a.commit, "entry.update", |d| {
                d.update_entry(&a.id, &draft)?;
                if let Some(tags) = tags {
                    d.set_entry_tags(&a.id, tags)?;
                }
                Ok(json!({"id":a.id}))
            })
        }
        EntryCommand::Move(a) => mutation(doc, vault, a.commit, "entry.move", |d| {
            d.move_entry(&a.id, &a.group_id)
                .map(|_| json!({"id":a.id,"group_id":a.group_id}))
        }),
        EntryCommand::Favorite(a) => mutation(doc, vault, a.commit, "entry.favorite", |d| {
            let current = d
                .snapshot()
                .find_entry(&a.id)
                .ok_or(crate::keepass::MutationError::EntryNotFound)?
                .starred;
            if current != a.set {
                d.toggle_starred(&a.id)?;
            }
            Ok(json!({"id":a.id,"starred":a.set}))
        }),
        EntryCommand::Trash(a) => mutation(doc, vault, a.commit, "entry.trash", |d| {
            d.delete_entry(&a.id).map(|_| json!({"id":a.id}))
        }),
        EntryCommand::Restore(a) => mutation(doc, vault, a.commit, "entry.restore", |d| {
            d.restore_entry(&a.id).map(|_| json!({"id":a.id}))
        }),
    }
}

fn not_found() -> CliError {
    CliError::new("not_found", 4, "entry or group not found")
}
fn mutation<F>(
    doc: &mut VaultDocument,
    vault: &Path,
    commit: bool,
    operation: &str,
    apply: F,
) -> Result<Value, CliError>
where
    F: FnOnce(&mut VaultDocument) -> Result<Value, crate::keepass::MutationError>,
{
    let target = apply(doc).map_err(|e| CliError::new("validation_failed", 6, e.to_string()))?;
    let mut warnings = Vec::new();
    if commit {
        let receipt = doc.save_payload().save_to(vault).map_err(|e| {
            CliError::new(
                "save_failed",
                if matches!(e, crate::keepass::SaveError::ExternalModification(_)) {
                    5
                } else {
                    7
                },
                e.to_string(),
            )
        })?;
        if let Some(w) = receipt.durability_error() {
            warnings.push(w.to_string());
        }
    }
    Ok(json!({"operation":operation,"committed":commit,"target":target,"warnings":warnings}))
}

fn secret(doc: &VaultDocument, a: &EntrySecret) -> Result<Value, CliError> {
    if !a.reveal {
        return Err(CliError::new(
            "reveal_required",
            6,
            "secret output requires --reveal",
        ));
    }
    // A pipe or a file would put the secret where any program, a log or the
    // disk can keep it. `run --secret-fd` hands it to a program instead.
    if !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        return Err(CliError::new(
            "reveal_needs_terminal",
            6,
            "--reveal prints a secret only to a terminal; to hand one to a \
             program, use run --secret-fd",
        ));
    }
    if doc.snapshot().find_entry(&a.id).is_none() {
        return Err(not_found());
    }
    let field = SecretField::parse(&a.field)?;
    let value = secret_value(doc, &a.id, &field)?;
    // Copied out of the wiped buffer here on purpose: the JSON envelope below
    // owns it from this point, and wrapping the whole envelope would buy
    // nothing.
    Ok(json!({"entry_id":a.id,"field":a.field,"value":value.as_str()}))
}

/// A secret field of an entry, as `entry secret --field` and
/// `run --secret-fd` name it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum SecretField {
    Password,
    Totp,
    Custom(String),
}

impl SecretField {
    /// `custom:` with an empty `KEY` is refused like any other bad field, so
    /// `run` stops before Touch ID and not after the unlock, where no custom
    /// field has an empty key.
    fn parse(field: &str) -> Result<Self, CliError> {
        if field == "password" {
            Ok(Self::Password)
        } else if field == "totp" {
            Ok(Self::Totp)
        } else if let Some(key) = field.strip_prefix("custom:")
            && !key.is_empty()
        {
            Ok(Self::Custom(key.to_owned()))
        } else {
            Err(CliError::new(
                "invalid_field",
                6,
                "field must be password, totp, or custom:KEY",
            ))
        }
    }
}

/// The value of `field` in the entry `id`, which the caller has found. A
/// custom field counts only when it is protected.
fn secret_value(
    doc: &VaultDocument,
    id: &str,
    field: &SecretField,
) -> Result<Zeroizing<String>, CliError> {
    let value = match field {
        SecretField::Password => doc.password_for_entry(id),
        SecretField::Totp => doc
            .totp_for_entry(id)
            .map(|v| Zeroizing::new(v.code.chars().filter(|c| c.is_ascii_digit()).collect())),
        SecretField::Custom(key) => {
            let entry = doc.snapshot().find_entry(id).ok_or_else(not_found)?;
            if !entry
                .custom_fields
                .iter()
                .any(|f| f.key == *key && f.protected)
            {
                return Err(CliError::new(
                    "not_secret",
                    6,
                    "custom field is missing or is not protected",
                ));
            }
            doc.custom_field_value(id, key)
        }
    };
    value.ok_or_else(|| CliError::new("secret_not_found", 4, "secret is not present"))
}

impl EntryInput {
    fn into_draft(self) -> Result<EntryDraft, CliError> {
        if self.title.trim().is_empty() {
            return Err(CliError::new("invalid_input", 6, "title must not be empty"));
        }
        Ok(EntryDraft {
            title: self.title,
            username: self.username,
            password: Zeroizing::new(self.password),
            url: self.url,
            notes: self.notes,
            tags: self.tags,
            otp: Zeroizing::new(self.otp),
            custom_fields: convert_custom(self.custom_fields)?,
        })
    }
}
fn convert_custom(input: Vec<CustomFieldInput>) -> Result<Vec<CustomField>, CliError> {
    let mut out = Vec::new();
    for f in input {
        if f.key.trim().is_empty() {
            return Err(CliError::new(
                "invalid_input",
                6,
                "custom field key must not be empty",
            ));
        }
        if out.iter().any(|x: &CustomField| x.key == f.key) {
            return Err(CliError::new(
                "invalid_input",
                6,
                "duplicate custom field key",
            ));
        }
        out.push(CustomField {
            key: f.key,
            value: f.value,
            protected: f.protected,
        });
    }
    Ok(out)
}
fn patched_draft(
    doc: &VaultDocument,
    id: &str,
    p: EntryPatch,
) -> Result<(EntryDraft, Option<Vec<String>>), CliError> {
    let e = doc.snapshot().find_entry(id).ok_or_else(not_found)?;
    let mut d = EntryDraft {
        title: e.title.clone(),
        username: e.username.clone(),
        password: doc.password_for_entry(id).unwrap_or_default(),
        url: e.url.clone(),
        notes: e.notes.clone(),
        tags: e.tags.clone(),
        otp: doc.otp_url_for_entry(id).unwrap_or_default(),
        custom_fields: e.custom_fields.clone(),
    };
    apply_patch(&mut d.title, p.title);
    apply_patch(&mut d.username, p.username);
    apply_patch(&mut d.password, p.password);
    apply_patch(&mut d.url, p.url);
    apply_patch(&mut d.notes, p.notes);
    apply_patch(&mut d.otp, p.otp);
    let tags = match p.tags {
        Patch::Missing => None,
        Patch::Clear => Some(Vec::new()),
        Patch::Set(v) => Some(v),
    };
    match p.custom_fields {
        Patch::Missing => {}
        Patch::Clear => d.custom_fields.clear(),
        Patch::Set(v) => d.custom_fields = convert_custom(v)?,
    }
    if d.title.trim().is_empty() {
        return Err(CliError::new("invalid_input", 6, "title must not be empty"));
    }
    Ok((d, tags))
}
fn apply_patch(target: &mut String, p: Patch<String>) {
    match p {
        Patch::Missing => {}
        Patch::Clear => target.clear(),
        Patch::Set(v) => *target = v,
    }
}

fn print_success(format: OutputFormat, data: Value) {
    match format {
        OutputFormat::Json => println!("{}", json!({"schema":SCHEMA,"ok":true,"data":data})),
        OutputFormat::Human => print_human(&data),
    }
}
/// Writes the error to stderr. A stderr that cannot be written loses the
/// text, but the CLI still exits with the error's code instead of the 101
/// of a panic in `eprintln!`.
fn print_error(format: OutputFormat, e: &CliError) {
    let _ = match format {
        OutputFormat::Json => writeln!(
            std::io::stderr(),
            "{}",
            json!({"schema":SCHEMA,"ok":false,"error":{"code":e.code,"message":e.message}})
        ),
        OutputFormat::Human => writeln!(std::io::stderr(), "error [{}]: {}", e.code, e.message),
    };
}
fn print_human(data: &Value) {
    if let Some(value) = data.get("value").and_then(Value::as_str) {
        println!("{value}");
        return;
    }
    println!(
        "{}",
        serde_json::to_string_pretty(data).unwrap_or_else(|_| "{}".into())
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_sap_requires_an_explicit_entry_id() {
        let parsed = Cli::try_parse_from([
            "ferrispass-cli",
            "--vault",
            "vault.kdbx",
            "launch",
            "sap",
            "--id",
            "entry-uuid",
        ])
        .expect("launch command parses");
        assert!(matches!(
            parsed.command,
            Command::Launch {
                command: LaunchCommand::Sap(LaunchEntry { id })
            } if id == "entry-uuid"
        ));
        assert!(
            Cli::try_parse_from(["ferrispass-cli", "--vault", "vault.kdbx", "launch", "sap"])
                .is_err()
        );
    }

    #[test]
    fn launch_io_errors_do_not_expose_paths_or_payloads() {
        let sentinel = "secret-launch-payload";
        let error = launch_error(crate::launch::LaunchError::Io(std::io::Error::other(
            sentinel,
        )));
        assert_eq!(error.code, "launch_failed");
        assert_eq!(error.exit, 7);
        assert!(!error.message.contains(sentinel));
    }

    #[test]
    fn patch_distinguishes_missing_null_and_value() {
        let missing: EntryPatch = serde_json::from_str("{}").unwrap();
        assert!(matches!(missing.password, Patch::Missing));
        let clear: EntryPatch = serde_json::from_str(r#"{"password":null}"#).unwrap();
        assert!(matches!(clear.password, Patch::Clear));
        let set: EntryPatch = serde_json::from_str(r#"{"password":"secret"}"#).unwrap();
        assert!(matches!(set.password, Patch::Set(value) if value == "secret"));
    }

    #[test]
    fn input_rejects_unknown_fields() {
        let result = serde_json::from_str::<EntryInput>(r#"{"title":"x","surprise":true}"#);
        assert!(result.is_err());
    }

    #[test]
    fn protected_custom_fields_are_redacted_from_entry_json() {
        let entry = VaultEntry {
            custom_fields: vec![
                CustomField {
                    key: "public".into(),
                    value: "shown".into(),
                    protected: false,
                },
                CustomField {
                    key: "private".into(),
                    value: "hidden".into(),
                    protected: true,
                },
            ],
            ..VaultEntry::default()
        };
        let value = entry_json(&entry);
        let rendered = value.to_string();
        assert!(rendered.contains("shown"));
        assert!(!rendered.contains("hidden"));
    }

    #[test]
    fn sync_plan_token_is_stable_and_revision_bound() {
        let report = crate::keepass::merge::ConflictReport::default();
        let token = sync_plan_token(b"local revision", "etag-1", &report);

        assert_eq!(token, sync_plan_token(b"local revision", "etag-1", &report));
        assert_ne!(token, sync_plan_token(b"changed", "etag-1", &report));
        assert_ne!(token, sync_plan_token(b"local revision", "etag-2", &report));
        assert!(token.starts_with("v1:"));
        assert_eq!(token.len(), 67);

        // A group conflict has to move the token too: the commit that quotes
        // it is answering the plan, and a plan that grew a question the user
        // never saw must not be accepted.
        let with_group = crate::keepass::merge::ConflictReport {
            group_conflicts: vec![crate::keepass::merge::GroupConflict {
                id: "g1".into(),
                name: "Banking".into(),
                path: Vec::new(),
                fields: Vec::new(),
            }],
            ..Default::default()
        };
        assert_ne!(
            token,
            sync_plan_token(b"local revision", "etag-1", &with_group)
        );

        // The database settings are part of the plan for the same reason.
        let mut with_metadata = report.clone();
        with_metadata.metadata_conflict = Some(crate::keepass::merge::MetadataConflict {
            fields: vec![crate::keepass::merge::FieldDiff {
                sides_read_alike: false,
                label: "Database name".into(),
                local: "Ours".into(),
                remote: "Theirs".into(),
                differs: true,
            }],
        });
        assert_ne!(
            token,
            sync_plan_token(b"local revision", "etag-1", &with_metadata)
        );
    }

    #[test]
    fn sync_status_does_not_try_to_unlock_the_vault() {
        let vault = std::env::temp_dir().join(format!(
            "ferrispass-cli-status-{}-{}.kdbx",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::write(&vault, b"not a kdbx").unwrap();
        let cli = Cli::try_parse_from([
            "ferrispass-cli",
            "--vault",
            vault.to_str().unwrap(),
            "sync",
            "status",
        ])
        .unwrap();

        let error = execute(&cli).unwrap_err();
        std::fs::remove_file(&vault).unwrap();
        assert_eq!(error.code, "sync_not_configured");
    }

    #[test]
    fn sync_resolutions_require_one_known_unique_choice_per_conflict() {
        let entries: std::collections::HashSet<String> =
            ["entry-a".to_owned(), "entry-b".to_owned()]
                .into_iter()
                .collect();
        let groups: std::collections::HashSet<String> =
            ["group-a".to_owned()].into_iter().collect();
        let valid: SyncResolutions = serde_json::from_str(
            r#"{"resolutions":[{"entry_id":"entry-a","keep":"local"},{"entry_id":"entry-b","keep":"remote"},{"group_id":"group-a","keep":"remote"}]}"#,
        )
        .unwrap();
        let picks = validate_resolution_input(valid, &entries, &groups, false).unwrap();
        assert_eq!(picks.entries.len(), 2);
        assert_eq!(picks.entries["entry-a"], crate::keepass::merge::Side::Local);
        assert_eq!(
            picks.entries["entry-b"],
            crate::keepass::merge::Side::Remote
        );
        assert_eq!(picks.groups["group-a"], crate::keepass::merge::Side::Remote);

        for (json, code) in [
            (
                r#"{"resolutions":[{"entry_id":"entry-a","keep":"local"},{"entry_id":"entry-b","keep":"remote"}]}"#,
                "incomplete_resolution",
            ),
            (
                r#"{"resolutions":[{"entry_id":"entry-a","keep":"local"},{"entry_id":"entry-a","keep":"remote"},{"group_id":"group-a","keep":"local"}]}"#,
                "invalid_resolution",
            ),
            (
                r#"{"resolutions":[{"entry_id":"entry-a","keep":"local"},{"entry_id":"unknown","keep":"remote"},{"group_id":"group-a","keep":"local"}]}"#,
                "invalid_resolution",
            ),
            // A group id offered where an entry is expected must not resolve
            // an entry conflict, and vice versa.
            (
                r#"{"resolutions":[{"entry_id":"group-a","keep":"local"},{"entry_id":"entry-b","keep":"remote"},{"group_id":"group-a","keep":"local"}]}"#,
                "invalid_resolution",
            ),
            (
                r#"{"resolutions":[{"entry_id":"entry-a","keep":"local"},{"keep":"remote"},{"group_id":"group-a","keep":"local"}]}"#,
                "invalid_resolution",
            ),
        ] {
            let input = serde_json::from_str(json).unwrap();
            assert_eq!(
                validate_resolution_input(input, &entries, &groups, false)
                    .unwrap_err()
                    .code,
                code
            );
        }
    }

    /// The database settings are one decision for the whole file, so they
    /// travel beside the id-keyed list rather than inside it. A plan that
    /// asks for one and gets none must not commit: the merge would then
    /// apply the default and upload it, which is what the question was for.
    #[test]
    fn a_metadata_conflict_needs_its_own_answer() {
        let none: std::collections::HashSet<String> = std::collections::HashSet::new();

        let answered: SyncResolutions =
            serde_json::from_str(r#"{"resolutions":[],"metadata":"remote"}"#).unwrap();
        let picks = validate_resolution_input(answered, &none, &none, true).unwrap();
        assert_eq!(picks.metadata, Some(crate::keepass::merge::Side::Remote));

        let unanswered: SyncResolutions = serde_json::from_str(r#"{"resolutions":[]}"#).unwrap();
        assert_eq!(
            validate_resolution_input(unanswered, &none, &none, true)
                .unwrap_err()
                .code,
            "incomplete_resolution"
        );

        // And an answer to a question nobody asked is a stale plan, not a
        // decision to act on.
        let spurious: SyncResolutions =
            serde_json::from_str(r#"{"resolutions":[],"metadata":"local"}"#).unwrap();
        assert_eq!(
            validate_resolution_input(spurious, &none, &none, false)
                .unwrap_err()
                .code,
            "invalid_resolution"
        );
    }

    /// `run` up to the Touch ID prompt, with a fake store that records the
    /// reason and answers that the user cancelled.
    #[cfg(unix)]
    mod run_reason {
        use super::*;
        use crate::biometric::{
            BiometricEnrollment, BiometricError, BiometricResult, EnrollmentId,
        };
        use std::sync::Mutex;

        const ID: &str = "1b4e28ba-2fa1-11d2-883f-0016d3cca427";

        #[derive(Debug, Default)]
        struct RecordingStore {
            reasons: Mutex<Vec<String>>,
        }

        impl BiometricStore for RecordingStore {
            fn is_available(&self) -> bool {
                true
            }

            fn enroll(&self, _: &EnrollmentId, _: &str) -> BiometricResult<()> {
                Ok(())
            }

            fn retrieve(
                &self,
                _: &EnrollmentId,
                prompt: &str,
                _: RetrieveOptions,
            ) -> BiometricResult<Zeroizing<String>> {
                self.reasons.lock().unwrap().push(prompt.to_owned());
                Err(BiometricError::UserCancelled)
            }

            fn forget(&self, _: &EnrollmentId) -> BiometricResult<()> {
                Ok(())
            }
        }

        /// Runs `ferrispass-cli --vault <dir>/<vault_name> --touch-id run
        /// <run_args>` against an enrolled vault file and the fake store.
        /// The reason Touch ID was asked with, or the error that came first.
        fn reason_for(vault_name: &str, run_args: &[&str]) -> Result<String, CliError> {
            let dir = tempfile::tempdir().expect("tempdir");
            let vault = dir.path().join(vault_name);
            std::fs::write(&vault, b"never opened").expect("write the vault file");
            let vault_arg = vault.to_str().expect("a UTF-8 path").to_owned();
            let mut argv = vec!["ferrispass-cli", "--vault", &vault_arg, "--touch-id", "run"];
            argv.extend(run_args);
            let cli = Cli::try_parse_from(&argv).expect("the command line parses");
            let Command::Run(args) = &cli.command else {
                panic!("not a run command");
            };

            let store = Arc::new(RecordingStore::default());
            let mut registry = BiometricRegistry::new();
            registry.upsert(
                std::fs::canonicalize(&vault).expect("canonical vault path"),
                BiometricEnrollment {
                    id: EnrollmentId::new_random(),
                    keyfile: None,
                    enrolled_at: chrono::Local::now(),
                },
            );
            let asked = Arc::clone(&store);
            let error = run_program(
                &cli,
                args,
                move || Arc::clone(&asked) as Arc<dyn BiometricStore>,
                move || registry,
            )
            .expect_err("the fake store never unlocks");

            let reasons = store.reasons.lock().unwrap().clone();
            match reasons.as_slice() {
                [] => Err(error),
                [reason] => {
                    assert_eq!(error.code, "touch_id_failed");
                    assert!(
                        !reason.contains(&dir.path().to_string_lossy().into_owned()),
                        "the reason holds the vault's folder: {reason}"
                    );
                    Ok(reason.clone())
                }
                more => panic!("Touch ID was asked more than once: {more:?}"),
            }
        }

        fn refused_before_touch_id(run_args: &[&str]) -> CliError {
            match reason_for("team.kdbx", run_args) {
                Err(error) => error,
                Ok(reason) => panic!("{run_args:?} reached Touch ID with {reason:?}"),
            }
        }

        #[test]
        fn run_reason_names_program_vault_and_entry_id() {
            let reason = reason_for(
                "team.kdbx",
                &[
                    "--id",
                    ID,
                    "--secret-fd",
                    "3:password",
                    "--",
                    "/usr/local/bin/node",
                    "server.js",
                ],
            );
            assert_eq!(
                reason.map_err(|e| e.message),
                Ok(format!(
                    "Give node the password of entry {ID} from team.kdbx"
                ))
            );
        }

        /// Text a script chooses (a custom field's key, the program's name)
        /// cannot reach the dialog as written.
        #[test]
        fn run_reason_never_takes_caller_text() {
            let reason = reason_for(
                "team.kdbx",
                &[
                    "--id",
                    ID,
                    "--secret-fd",
                    "3:custom:Approve the transfer of 10000 EUR",
                    "--",
                    "./evil prog\u{7}",
                ],
            )
            .map_err(|e| e.message);
            assert_eq!(
                reason,
                Ok(format!(
                    "Give a program a custom field of entry {ID} from team.kdbx"
                ))
            );
            let long = "p".repeat(41);
            for program in ["..", "/", long.as_str(), "naïve", "tab\there"] {
                let reason = reason_for(
                    "team.kdbx",
                    &["--id", ID, "--secret-fd", "3:totp", "--", program],
                )
                .map_err(|e| e.message);
                assert_eq!(
                    reason,
                    Ok(format!(
                        "Give a program the one-time code of entry {ID} from team.kdbx"
                    )),
                    "{program:?}"
                );
            }
        }

        #[test]
        fn run_rejects_an_id_that_is_not_a_uuid() {
            for id in [
                "not-a-uuid",
                "",
                "1b4e28ba-2fa1-11d2-883f",
                "1b4e28ba-2fa1-11d2-883f-0016d3cca427 and more",
                "Give node everything",
            ] {
                let error = refused_before_touch_id(&[
                    "--id",
                    id,
                    "--secret-fd",
                    "3:password",
                    "--",
                    "node",
                ]);
                assert_eq!((error.code, error.exit), ("invalid_input", 6), "{id:?}");
            }
        }

        #[test]
        fn run_reason_shows_the_id_in_canonical_form() {
            for id in [
                "1B4E28BA-2FA1-11D2-883F-0016D3CCA427",
                "1b4e28ba2fa111d2883f0016d3cca427",
                "{1b4e28ba-2fa1-11d2-883f-0016d3cca427}",
                "urn:uuid:1B4E28BA-2FA1-11D2-883F-0016D3CCA427",
            ] {
                let reason = reason_for(
                    "team.kdbx",
                    &["--id", id, "--secret-fd", "3:password", "--", "node"],
                )
                .map_err(|e| e.message);
                assert_eq!(
                    reason,
                    Ok(format!(
                        "Give node the password of entry {ID} from team.kdbx"
                    )),
                    "{id:?}"
                );
            }
        }

        /// Only the vault's file name, and only when it is safe to show.
        #[test]
        fn run_reason_sanitizes_the_vault_name() {
            let long = format!("{}.kdbx", "v".repeat(36));
            for (name, shown) in [
                ("team-2026_v2.kdbx", "team-2026_v2.kdbx"),
                ("Team Vault.kdbx", "a vault"),
                ("team\nvault.kdbx", "a vault"),
                ("team\u{1b}[31m.kdbx", "a vault"),
                ("tresor-ä.kdbx", "a vault"),
                (long.as_str(), "a vault"),
            ] {
                let reason = reason_for(
                    name,
                    &["--id", ID, "--secret-fd", "3:password", "--", "node"],
                )
                .map_err(|e| e.message);
                assert_eq!(
                    reason,
                    Ok(format!("Give node the password of entry {ID} from {shown}")),
                    "{name:?}"
                );
            }
        }

        #[test]
        fn run_reason_names_every_released_field() {
            for (fields, phrase) in [
                (
                    &["3:password", "4:totp"][..],
                    "the password and the one-time code",
                ),
                (
                    &["3:totp", "4:password", "5:custom:API_KEY"][..],
                    "the one-time code, the password and a custom field",
                ),
            ] {
                let mut args = vec!["--id", ID];
                for field in fields {
                    args.extend(["--secret-fd", field]);
                }
                args.extend(["--", "node"]);
                let reason = reason_for("team.kdbx", &args).map_err(|e| e.message);
                assert_eq!(
                    reason,
                    Ok(format!("Give node {phrase} of entry {ID} from team.kdbx")),
                    "{fields:?}"
                );
            }
        }

        #[test]
        fn run_reason_counts_distinct_custom_fields() {
            for (fields, phrase) in [
                (&["3:custom:A", "4:custom:B"][..], "two custom fields"),
                (
                    &["3:custom:A", "4:custom:B", "5:custom:C"][..],
                    "three custom fields",
                ),
                (
                    &["3:custom:A", "4:password", "5:custom:a"][..],
                    "two custom fields and the password",
                ),
            ] {
                let mut args = vec!["--id", ID];
                for field in fields {
                    args.extend(["--secret-fd", field]);
                }
                args.extend(["--", "node"]);
                let reason = reason_for("team.kdbx", &args).map_err(|e| e.message);
                assert_eq!(
                    reason,
                    Ok(format!("Give node {phrase} of entry {ID} from team.kdbx")),
                    "{fields:?}"
                );
            }
        }

        #[test]
        fn the_same_field_twice_is_refused() {
            for fields in [
                ["3:password", "4:password"],
                ["3:custom:API_KEY", "4:custom:API_KEY"],
            ] {
                let error = refused_before_touch_id(&[
                    "--id",
                    ID,
                    "--secret-fd",
                    fields[0],
                    "--secret-fd",
                    fields[1],
                    "--",
                    "node",
                ]);
                assert_eq!((error.code, error.exit), ("invalid_input", 6), "{fields:?}");
                assert!(error.message.contains("only once"), "{}", error.message);
            }
        }

        /// `run --id ID <options> -- node`, each option split at the first
        /// space.
        fn with_options(options: &[&'static str]) -> Vec<&'static str> {
            let mut args = vec!["--id", ID];
            for option in options {
                let (name, value) = option.split_once(' ').expect("an option and its value");
                args.extend([name, value]);
            }
            args.extend(["--", "node"]);
            args
        }

        /// A field goes to the program one way: on a descriptor or in a
        /// variable, never both, and never in two variables.
        #[test]
        fn env_and_fd_cannot_name_the_same_field() {
            for options in [
                &["--secret-fd 3:password", "--env PW=password"][..],
                &["--env PW=password", "--secret-fd 3:password"][..],
                &["--env A=totp", "--env B=totp"][..],
                &["--secret-fd 3:custom:API_KEY", "--env KEY=custom:API_KEY"][..],
            ] {
                let error = refused_before_touch_id(&with_options(options));
                assert_eq!(
                    (error.code, error.exit),
                    ("invalid_input", 6),
                    "{options:?}"
                );
                assert!(
                    error
                        .message
                        .contains("field can go to the program only once"),
                    "{options:?}: {}",
                    error.message
                );
            }
        }

        #[test]
        fn the_same_variable_twice_is_refused() {
            let error = refused_before_touch_id(&with_options(&[
                "--env TOKEN=password",
                "--env TOKEN=totp",
            ]));
            assert_eq!((error.code, error.exit), ("invalid_input", 6));
            assert!(
                error.message.contains("variable can be set only once"),
                "{}",
                error.message
            );
        }

        #[test]
        fn an_invalid_variable_name_is_refused() {
            for spec in [
                "1X=password",
                "A-B=password",
                "=password",
                "A B=password",
                "TÖKEN=password",
                "A.B=password",
            ] {
                let error = refused_before_touch_id(&["--id", ID, "--env", spec, "--", "node"]);
                assert_eq!((error.code, error.exit), ("invalid_input", 6), "{spec}");
                assert!(
                    error.message.contains("has to start with a letter or _"),
                    "{spec}: {}",
                    error.message
                );
            }
            let error = refused_before_touch_id(&["--id", ID, "--env", "TOKEN", "--", "node"]);
            assert_eq!((error.code, error.exit), ("invalid_input", 6));
            assert!(error.message.contains("NAME=FIELD"), "{}", error.message);

            // What a shell takes is taken.
            for spec in ["X=password", "_=password", "api_token_2=password"] {
                let reason = reason_for("team.kdbx", &["--id", ID, "--env", spec, "--", "node"])
                    .map_err(|e| e.message);
                assert_eq!(
                    reason,
                    Ok(format!(
                        "Give node the password of entry {ID} from team.kdbx"
                    )),
                    "{spec}"
                );
            }
        }

        /// Fields on descriptors first, then fields in variables, each in
        /// the order given; the variable's name never shows.
        #[test]
        fn the_reason_names_fields_from_env_and_fd_in_order() {
            for (options, phrase) in [
                (
                    &["--env PW=password", "--secret-fd 3:totp"][..],
                    "the one-time code and the password",
                ),
                (
                    &[
                        "--secret-fd 3:password",
                        "--env CODE=totp",
                        "--env KEY=custom:API_KEY",
                    ][..],
                    "the password, the one-time code and a custom field",
                ),
                (
                    &[
                        "--env Approve_the_transfer=custom:A",
                        "--secret-fd 3:custom:B",
                    ][..],
                    "two custom fields",
                ),
                (&["--env PW=password"][..], "the password"),
            ] {
                let reason = reason_for("team.kdbx", &with_options(options)).map_err(|e| e.message);
                assert_eq!(
                    reason,
                    Ok(format!("Give node {phrase} of entry {ID} from team.kdbx")),
                    "{options:?}"
                );
            }

            // No field shows twice: a command line that names one twice
            // never reaches the prompt.
            for options in [
                &["--secret-fd 3:password", "--env PW=password"][..],
                &["--env A=totp", "--env B=totp"][..],
            ] {
                if let Ok(reason) = reason_for("team.kdbx", &with_options(options)) {
                    panic!("{options:?} reached Touch ID with {reason:?}");
                }
            }
        }

        #[test]
        fn run_rejects_target_fds_outside_3_to_255_and_duplicates() {
            for (spec, says) in [
                ("0:password", "below 3"),
                ("2:password", "below 3"),
                ("256:password", "above 255"),
                ("4294967295:password", "above 255"),
                ("4294967296:password", "number from 3 to 255"),
                ("three:password", "number from 3 to 255"),
            ] {
                let error =
                    refused_before_touch_id(&["--id", ID, "--secret-fd", spec, "--", "node"]);
                assert_eq!((error.code, error.exit), ("invalid_input", 6), "{spec}");
                assert!(error.message.contains(says), "{spec}: {}", error.message);
            }
            let error = refused_before_touch_id(&[
                "--id",
                ID,
                "--secret-fd",
                "3:password",
                "--secret-fd",
                "3:totp",
                "--",
                "node",
            ]);
            assert_eq!((error.code, error.exit), ("invalid_input", 6));
            assert!(error.message.contains("named twice"), "{}", error.message);
        }
    }

    /// `launch` keeps a 0600 file holding a cleartext password alive while
    /// the target app reads it. Ctrl+C in that window runs no destructor, so
    /// the interrupt path has to unlink the file itself, and it has to be
    /// able to name it from the moment it exists rather than from whenever
    /// the caller gets around to registering it.
    #[test]
    fn an_interrupt_removes_every_staged_launch_file() {
        let dir = tempfile::tempdir().expect("tempdir");

        // Nothing staged: the handler has nothing to do, which is correct
        // rather than an error.
        remove_staged_launch_files();

        let staged =
            crate::launch::tempfile::TempLaunchFile::create_in(dir.path(), "sapc", b"pass=secret")
                .expect("stage the payload");
        let path = staged.path().to_path_buf();
        assert!(path.exists());
        assert!(
            crate::launch::tempfile::live_staged_files().contains(&path),
            "registered by creation, not by the caller afterwards"
        );

        remove_staged_launch_files();
        assert!(!path.exists(), "the cleartext payload is gone");

        // Idempotent: a second signal, or the ordinary drop arriving after
        // the handler, must not fall over a file that is already gone.
        remove_staged_launch_files();
        drop(staged);
        assert!(
            !crate::launch::tempfile::live_staged_files().contains(&path),
            "and the entry does not outlive the file"
        );
    }
}

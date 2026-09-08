//! Headless, machine-readable FerrisPass command line interface.

use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    fs::File,
    io::Read as _,
    path::{Path, PathBuf},
};

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

use crate::{
    biometric::RetrieveOptions,
    domain::{CustomField, VaultEntry, VaultGroup},
    keepass::{EntryDraft, KeePassRepository, VaultDocument},
};

const SCHEMA: &str = "ferrispass-cli/v1";

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
    match execute(&cli) {
        Ok(out) => {
            print_success(cli.format, out);
            0
        }
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
    // Scoped, so it is wiped as soon as the vault is open rather than living
    // through the rest of the command: a `launch` waits ten seconds for the
    // target app, and a `sync` makes network calls. The GUI does the same,
    // and SECURITY.md says so for both.
    let mut document = {
        let password = unlock_password(cli)?;
        KeePassRepository::open(vault, &password, cli.key_file.as_deref())
            .map_err(|e| CliError::new("unlock_failed", 3, e.to_string()))?
    };
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
        } => unreachable!(),
    }
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
    // The check, the upload and the write are one operation as far as the
    // app is concerned. Holding this lock across a network call is
    // deliberate, here and only here: the alternative is a window between
    // deciding the binding is still ours and writing to it, and the app's own
    // wait for the lock is bounded, so the worst case is that it proceeds
    // exactly as it would have without the lock at all.
    //
    // The comparison is against the binding as it was when this command
    // started, not against our copy: `restore_provider` refreshes an iCloud
    // bookmark in memory, and comparing that would report a change nobody
    // else made.
    crate::sync::lock::held(crate::sync::lock::BATCH, || {
        let current = crate::sync::config::load_unlocked(&canonical).map_err(sync_error)?;
        if !current
            .is_some_and(|current| crate::sync::config::same_relationship(&current, &baseline))
        {
            return Err(CliError::new(
                "sync_binding_changed",
                5,
                "the vault's sync binding changed while this command ran; the \
                 merge is saved locally, rerun sync to send it",
            ));
        }
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
        crate::sync::config::save_unlocked(&config).map_err(sync_error)
    })
    .ok_or_else(|| {
        CliError::new(
            "sync_busy",
            5,
            "another FerrisPass process is using the sync configuration; the \
             merge is saved locally, rerun sync to send it",
        )
    })??;
    let resolved = picks.entries.len() + picks.groups.len() + usize::from(picks.metadata.is_some());
    Ok(
        json!({"status":"synced","committed":true,"uploaded":resolved_upload,"merged":merged_count,"resolved":resolved}),
    )
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

fn unlock_password(cli: &Cli) -> Result<Zeroizing<String>, CliError> {
    if let Some(fd) = cli.master_password_fd {
        return read_secret_fd(fd);
    }
    let vault = cli
        .vault
        .as_deref()
        .ok_or_else(|| CliError::new("missing_vault", 2, "--vault is required"))?;
    let canonical =
        std::fs::canonicalize(vault).map_err(|e| CliError::new("io", 7, e.to_string()))?;
    let registry = crate::biometric::registry::load_or_default();
    let enrollment = registry.get(&canonical).or_else(|| registry.get(vault));
    // Biometry needs a person in front of the machine. Preferring it whenever
    // an enrolment existed meant a scripted invocation raised an OS prompt and
    // blocked on it for up to two minutes with nobody there to answer, which
    // made agent runs nondeterministic. Ask only when explicitly requested, or
    // when stdin is a terminal and so someone is plausibly watching.
    let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let wants_biometrics = cli.touch_id
        || (!cli.no_touch_id
            && interactive
            && enrollment.is_some()
            && crate::biometric::default_store().is_available());
    if wants_biometrics {
        let enrollment = enrollment.ok_or_else(|| {
            CliError::new(
                "touch_id_not_enrolled",
                3,
                "Touch ID is not enrolled for this vault",
            )
        })?;
        return crate::biometric::default_store()
            .retrieve(
                &enrollment.id,
                "Unlock FerrisPass vault",
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
    if doc.snapshot().find_entry(&a.id).is_none() {
        return Err(not_found());
    }
    let value = if a.field == "password" {
        // Unwrapped here on purpose: the JSON envelope below owns it from
        // this point, and wrapping the whole envelope would buy nothing.
        doc.password_for_entry(&a.id)
            .map(|password| password.to_string())
    } else if a.field == "totp" {
        doc.totp_for_entry(&a.id)
            .map(|v| v.code.chars().filter(|c| c.is_ascii_digit()).collect())
    } else if let Some(key) = a.field.strip_prefix("custom:") {
        let entry = doc.snapshot().find_entry(&a.id).ok_or_else(not_found)?;
        if !entry
            .custom_fields
            .iter()
            .any(|f| f.key == key && f.protected)
        {
            return Err(CliError::new(
                "not_secret",
                6,
                "custom field is missing or is not protected",
            ));
        }
        doc.custom_field_value(&a.id, key)
            .map(|secret| secret.to_string())
    } else {
        return Err(CliError::new(
            "invalid_field",
            6,
            "field must be password, totp, or custom:KEY",
        ));
    };
    value
        .map(|v| json!({"entry_id":a.id,"field":a.field,"value":v}))
        .ok_or_else(|| CliError::new("secret_not_found", 4, "secret is not present"))
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
fn print_error(format: OutputFormat, e: &CliError) {
    match format {
        OutputFormat::Json => eprintln!(
            "{}",
            json!({"schema":SCHEMA,"ok":false,"error":{"code":e.code,"message":e.message}})
        ),
        OutputFormat::Human => eprintln!("error [{}]: {}", e.code, e.message),
    }
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

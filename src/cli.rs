//! Headless, machine-readable FerrisPass command line interface.

use std::{
    fs::File,
    io::Read as _,
    path::{Path, PathBuf},
};

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Deserialize;
use serde_json::{Value, json};
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
}

#[derive(Subcommand)]
enum SyncCommand {
    Status,
    Now,
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
    details: Option<Value>,
}
impl CliError {
    fn new(code: &'static str, exit: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            exit,
            message: message.into(),
            details: None,
        }
    }
    fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

pub fn run() -> i32 {
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
    let password = unlock_password(cli)?;
    let mut document = KeePassRepository::open(vault, &password, cli.key_file.as_deref())
        .map_err(|e| CliError::new("unlock_failed", 3, e.to_string()))?;
    match &cli.command {
        Command::Vault {
            command: VaultCommand::Info,
        } => Ok(vault_info(&document)),
        Command::Group { command } => execute_group(command, &mut document, vault),
        Command::Entry { command } => execute_entry(command, &mut document, vault),
        Command::Sync { command } => execute_sync(
            command,
            &mut document,
            vault,
            &password,
            cli.key_file.as_deref(),
        ),
    }
}

fn execute_sync(
    command: &SyncCommand,
    document: &mut VaultDocument,
    vault: &Path,
    password: &str,
    key_file: Option<&Path>,
) -> Result<Value, CliError> {
    let canonical =
        std::fs::canonicalize(vault).map_err(|e| CliError::new("io", 7, e.to_string()))?;
    let mut config = crate::sync::config::load(&canonical)
        .map_err(sync_error)?
        .ok_or_else(|| {
            CliError::new(
                "sync_not_configured",
                6,
                "vault is not connected to SharePoint",
            )
        })?;
    if matches!(command, SyncCommand::Status) {
        return Ok(json!({
            "provider":"sharepoint",
            "account":config.account_email,
            "remote_url":config.remote_url,
            "configured":true
        }));
    }

    let token =
        crate::sync::service::refresh_access_token(&config.account_email).map_err(sync_error)?;
    let local_bytes = document
        .read_current_bytes()
        .map_err(|e| CliError::new("local_revision_changed", 5, e.to_string()))?;
    match crate::sync::service::upload_after_save(&config, &token, &local_bytes)
        .map_err(sync_error)?
    {
        crate::sync::service::UploadAfterSave::Synced { new_etag, .. } => {
            config.last_etag = new_etag;
            crate::sync::config::save(&config).map_err(sync_error)?;
            Ok(json!({"status":"synced","uploaded":true,"merged":0}))
        }
        crate::sync::service::UploadAfterSave::Conflict {
            remote_bytes,
            remote_etag,
        } => {
            let remote = KeePassRepository::open_bytes(&remote_bytes, password, key_file)
                .map_err(|e| CliError::new("remote_unlock_failed", 3, e.to_string()))?;
            let report = crate::keepass::merge::diff(document.database(), remote.database());
            if !report.conflicts.is_empty() {
                let conflicts: Vec<Value> = report
                    .conflicts
                    .iter()
                    .map(|conflict| {
                        let fields: Vec<&str> = conflict
                            .fields
                            .iter()
                            .filter(|field| field.differs)
                            .map(|field| field.label)
                            .collect();
                        json!({"entry_id":conflict.id,"fields":fields})
                    })
                    .collect();
                return Err(CliError::new(
                    "sync_conflict",
                    5,
                    "ambiguous remote changes require manual resolution",
                )
                .with_details(json!({"conflicts":conflicts})));
            }
            let merged_count = report.remote_only.len() + report.auto_resolved.len();
            let needs_upload = report.has_local_contribution();
            let merged = crate::keepass::merge::apply_picks(
                document.database(),
                remote.database(),
                &std::collections::HashMap::new(),
                &report,
            )
            .map_err(|e| CliError::new("merge_failed", 5, e.to_string()))?;
            let receipt = document
                .save_payload_for_database(merged.clone())
                .save_to(vault)
                .map_err(|e| CliError::new("save_failed", 7, e.to_string()))?;
            document.replace_database(merged);
            config.last_etag = remote_etag;
            if needs_upload {
                match crate::sync::service::upload_after_save(&config, &token, &receipt.bytes())
                    .map_err(sync_error)?
                {
                    crate::sync::service::UploadAfterSave::Synced { new_etag, .. } => {
                        config.last_etag = new_etag
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
            crate::sync::config::save(&config).map_err(sync_error)?;
            Ok(json!({"status":"synced","uploaded":needs_upload,"merged":merged_count}))
        }
    }
}

fn sync_error(error: impl std::fmt::Display) -> CliError {
    CliError::new("sync_failed", 7, error.to_string())
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
    if cli.touch_id || (enrollment.is_some() && crate::biometric::default_store().is_available()) {
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
fn collect_groups<'a>(group: &'a VaultGroup, parent: Option<&str>, out: &mut Vec<Value>) {
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
        doc.password_for_entry(&a.id)
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
            password: self.password,
            url: self.url,
            notes: self.notes,
            tags: self.tags,
            otp: self.otp,
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
            json!({"schema":SCHEMA,"ok":false,"error":{"code":e.code,"message":e.message,"details":e.details}})
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
        let mut entry = VaultEntry::default();
        entry.custom_fields = vec![
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
        ];
        let value = entry_json(&entry);
        let rendered = value.to_string();
        assert!(rendered.contains("shown"));
        assert!(!rendered.contains("hidden"));
    }
}

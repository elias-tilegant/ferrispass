# FerrisPass CLI

`ferrispass-cli` is the headless interface for KeePass-compatible vaults. It
never starts the GUI. The configured provider is contacted only by `sync now`; all other
commands remain local. Use `--format json` for the versioned
`ferrispass-cli/v1` agent contract.

## Installation

The signed macOS app contains the matching CLI binary. Open **Settings →
General → Command-line interface** and choose **Register CLI**. After approval
in the macOS administrator dialog, FerrisPass creates
`/usr/local/bin/ferrispass-cli` as a symbolic link to the signed CLI inside the
app bundle. Because the link targets the bundle, FerrisPass app updates also
update the CLI automatically. FerrisPass never replaces an unrelated file or
link at that location.

Choose **Uninstall CLI** in the same settings card to remove the registration.
The bundled executable remains part of the app. Verify a registration with
`ferrispass-cli --version`.

## Command overview

```text
vault info
group list | create | rename | move | trash | restore
entry list | search | get | secret | create | update | move | favorite | trash | restore
launch sap
sync status | now
```

Use `ferrispass-cli <command> --help` for command-specific arguments. Most
commands require `--vault FILE`; `sync status` only reads the local provider
binding and does not unlock the database.

The JSON error envelope is `{"code", "message"}`. It briefly also carried a
`details` field, which was never populated on any path and has been removed.

## Unlocking safely

The CLI never accepts a master password in an argument or environment variable.
On a terminal it uses an enrolled Touch ID identity when one is available, and
otherwise prompts without echo. When stdin is not a terminal it never raises a
biometric prompt on its own: nobody would be there to answer it, and the OS
prompt blocks for two minutes before giving up. Pass `--touch-id` to require
biometrics anyway, or `--no-touch-id` to rule them out.

Automation must inherit a dedicated descriptor (3 or higher):

```sh
ferrispass-cli --vault team.kdbx --master-password-fd 3 --format json vault info 3<password.pipe
```

`--touch-id` requires an existing FerrisPass enrollment. macOS account-password
fallback is disabled unless `--allow-device-passcode` is also supplied.

When a MacBook is in clamshell mode, its Touch ID sensor is unavailable. Allow
the macOS account-password fallback explicitly:

```sh
ferrispass-cli --touch-id --allow-device-passcode \
  --vault team.kdbx vault info
```

Without `--allow-device-passcode`, cancelling or being unable to reach the
sensor fails closed. The CLI does not silently weaken a Touch ID-only request.

## Reading

```sh
ferrispass-cli --vault team.kdbx entry search github
ferrispass-cli --vault team.kdbx --format json entry get --id UUID
ferrispass-cli --vault team.kdbx --format json entry secret --id UUID --field password --reveal
```

Normal entry output never includes passwords, TOTP values, OTP seeds, or values
of protected custom fields. Secret access requires a specific UUID, field, and
`--reveal`. Treat stdout from a secret command as sensitive.

## Mutating

Create and update bodies are JSON read from stdin (or `--input-fd`). Missing
update properties are retained; `null` clears a property. Tags and custom fields
replace their complete collection when present.

```sh
printf '%s' '{"title":"Example","username":"alice"}' |
  ferrispass-cli --vault team.kdbx entry create --group-id UUID
```

Every mutation is a dry-run unless `--commit` is present. Committed writes use
FerrisPass's atomic publication, locking, and external-revision checks. Deletes
move objects to the KeePass Recycle Bin; the CLI has no permanent-delete command.

Run `ferrispass-cli --help`, `entry --help`, or `group --help` for the complete
command tree and stable option names.

## Launching external connections

Launch an SAP system from an exact entry UUID:

```sh
ferrispass-cli --vault team.kdbx launch sap --id UUID
```

The SAP launcher uses the same backend and entry-field model as the GUI. The
entry must contain non-empty `SAP_HOST` and `SAP_INSTANCE` custom fields and a
password. Optional fields are `SAP_USER`, `SAP_LANG`, `SAP_CLIENT`, and
`SAP_EXPERT`; the standard entry username is used when `SAP_USER` is absent.

FerrisPass passes only a private temporary file path to macOS Launch Services;
the password never appears in process arguments, stdout, JSON output, or error
messages. The command keeps the protected payload alive briefly so SAP GUI can
consume it, removes it before returning, and does not modify the vault.
Pressing Ctrl+C during that window unlinks the payload too, then exits 130;
without that the file would have waited for the next FerrisPass start. SAP
launch is currently supported on macOS. The typed launch command is designed
to add protocols such as SSH later without changing vault unlocking or entry
selection semantics.

## Cloud-provider sync

For a vault already connected by the FerrisPass GUI, `sync status` reads only
the local binding. It neither unlocks the vault nor contacts the provider.
Initial provider connection remains a GUI operation.

`sync now` is a two-step operation. The first invocation downloads and checks
the remote revision but does not change the local vault or cloud copy. Its JSON
result contains a `plan_token`, the proposed changes, and any ambiguous
conflicts. A second invocation with `--commit` repeats the checks and accepts
the plan only if both revisions still match:

```sh
ferrispass-cli --vault team.kdbx --format json sync status
ferrispass-cli --vault team.kdbx --format json sync now
ferrispass-cli --vault team.kdbx --format json sync now \
  --commit --plan-token 'v1:...'
```

The plan reports three kinds of conflict: `conflicts` for entries,
`group_conflicts` for groups, and `metadata_conflict` for the database's own
settings, which is one decision for the whole file and so has no UUID.

An entry or a group is reported when the two copies disagree about something
and no clock can rank them. Two clocks decide that, one per question: what the
object holds is ranked by its modification time, and where it sits by its
location time, so a name, an icon, an expiry date, a tag, plugin data or a
move can each be the reason on its own. The `fields` list names which of them
differ. Pass exactly one choice for every reported UUID on stdin (or a
dedicated `--input-fd`), naming `entry_id` or `group_id` to say which, and a
top-level `metadata` when the plan carries one. Unknown, duplicate, missing
and wrong-kind UUIDs fail closed, as does an answer to a question the plan did
not ask. Only UUIDs, group names, setting names and differing field names
appear in the plan; secret values are never emitted.

```sh
printf '%s' '{"resolutions":[
    {"entry_id":"UUID","keep":"remote"},
    {"group_id":"UUID","keep":"local"}
  ],
  "metadata":"local"}' |
  ferrispass-cli --vault team.kdbx --format json sync now \
    --commit --plan-token 'v1:...'
```

A group or settings conflict discards the losing side outright. KDBX archives
entry versions, not group or metadata versions, so there is nothing to keep
the other name in, which is why these are asked rather than decided. The
settings choice covers only the fields no change time could rank; one the
other copy demonstrably edited last is merged either way.

Uploads retain the provider revision guard (SharePoint ETag or iCloud content
revision). If the remote file changes between
planning and publication, the command stops and requires a fresh plan.

## Automation contract

`--format json` emits the versioned `ferrispass-cli/v1` envelope. Successful
results go to stdout; structured errors go to stderr. Programs should inspect
both the process exit code and the error `code` instead of matching messages.

| Exit code | Meaning |
|---:|---|
| 0 | Success |
| 2 | Command-line usage or missing vault |
| 3 | Unlock or credential input failed |
| 4 | Entry, group or secret not found |
| 5 | Revision conflict or stale sync plan |
| 6 | Invalid input, validation error or missing sync setup |
| 7 | I/O, save, network or sync service failure |

Agent workflows should follow these rules:

- Request JSON output.
- Address entries by UUID, not by title.
- Read one explicit secret field at a time and only with `--reveal`.
- Pass passwords through an inherited descriptor, never an argument or
  environment variable.
- Review mutation and sync plans before repeating them with `--commit`.

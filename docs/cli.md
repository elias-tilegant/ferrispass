# FerrisPass CLI

`ferrispass-cli` is the headless interface for KeePass-compatible vaults. It
never starts the GUI. SharePoint is contacted only by `sync now`; all other
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

## Unlocking safely

The CLI never accepts a master password in an argument or environment variable.
Without an explicit option it uses an enrolled Touch ID identity when available,
then falls back to a hidden terminal prompt. Automation must inherit a dedicated
descriptor (3 or higher):

```sh
ferrispass-cli --vault team.kdbx --master-password-fd 3 --format json vault info 3<password.pipe
```

`--touch-id` requires an existing FerrisPass enrollment. macOS account-password
fallback is disabled unless `--allow-device-passcode` is also supplied.

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

## SharePoint sync

For a vault already connected by the FerrisPass GUI, `sync status` reads only
the local binding. It neither unlocks the vault nor contacts SharePoint.
Initial SharePoint connection remains a GUI operation.

`sync now` is a two-step operation. The first invocation downloads and checks
the remote revision but does not change the local vault or SharePoint. Its JSON
result contains a `plan_token`, the proposed changes, and any ambiguous
conflicts. A second invocation with `--commit` repeats the checks and accepts
the plan only if both revisions still match:

```sh
ferrispass-cli --vault team.kdbx --format json sync status
ferrispass-cli --vault team.kdbx --format json sync now
ferrispass-cli --vault team.kdbx --format json sync now \
  --commit --plan-token 'v1:...'
```

When the plan reports conflicts, pass exactly one choice for every reported
entry UUID on stdin (or a dedicated `--input-fd`). Unknown, duplicate, and
missing UUIDs fail closed. Only UUIDs and differing field names appear in the
plan; secret values are never emitted.

```sh
printf '%s' '{"resolutions":[{"entry_id":"UUID","keep":"remote"}]}' |
  ferrispass-cli --vault team.kdbx --format json sync now \
    --commit --plan-token 'v1:...'
```

Uploads retain the SharePoint ETag guard. If the remote file changes between
planning and publication, the command stops and requires a fresh plan.

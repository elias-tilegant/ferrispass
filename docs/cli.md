# FerrisPass CLI

`ferrispass-cli` is the headless interface for local KeePass-compatible vaults.
It never starts the GUI or contacts SharePoint. Use `--format json` for the
versioned `ferrispass-cli/v1` agent contract.

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

For a vault already connected by the FerrisPass GUI, `sync status` reads the
existing binding and `sync now` refreshes the Keychain token, uploads with an
ETag guard, and automatically merges conflict-free remote changes. Ambiguous
entry conflicts fail closed with UUIDs and differing field names; no secret
values are included. Initial SharePoint connection remains a GUI operation.

```sh
ferrispass-cli --vault team.kdbx --format json sync status
ferrispass-cli --vault team.kdbx --format json sync now
```

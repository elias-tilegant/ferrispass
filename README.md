# STC KeePass

GPUI desktop app scaffold for a KeePass client.

## Structure

```text
src/
  app/       application bootstrap and app state
  domain/    UI-safe vault snapshot types
  keepass/   adapter from keepass-rs into the domain model
  ui/        GPUI views and shell layout
```

The current app starts without opening a database. The KeePass adapter already maps a `.kdbx`
database into a UI-safe snapshot without exposing passwords in the visible model.

## Commands

```sh
cargo check
cargo test
cargo run
```

## Next Steps

1. Add a file picker and unlock flow.
2. Store only unlocked vault state in `AppState`.
3. Add a vault browser view for groups and entries.
4. Add copy-to-clipboard actions with clear state boundaries.

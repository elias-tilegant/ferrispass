# Getting Started: iCloud Drive Sync

FerrisPass can bind a local working copy to any `.kdbx` file you select in
iCloud Drive. The provider only stores the encrypted KeePass file; FerrisPass
does not use CloudKit or an app-owned iCloud container.

## Connect an existing vault

1. Choose **Connect Cloud Vault** and then **iCloud Drive**.
2. Choose **Open existing iCloud vault** and select a `.kdbx` in iCloud Drive.
3. Choose a separate, non-iCloud location for the local working copy.
4. Unlock the downloaded local copy normally.

FerrisPass stores a macOS bookmark for the selected remote file, so ordinary
moves and renames can be resolved across launches. Reads and replacements are
coordinated with macOS, and every write is guarded by the last observed content
revision. A concurrent remote edit enters the normal encrypted-vault merge
flow instead of being overwritten.

## Publish the current vault

Open an unbound local vault, choose iCloud Drive in the Connect screen, then
choose **Publish current vault**. Select a new `.kdbx` path in iCloud Drive.
FerrisPass keeps the open local file as its working copy and creates the cloud
copy without replacing an existing item.

## Operational behavior

- Saving always commits the local encrypted file first, then publishes it.
- Periodic checks catch changes even when filesystem events are coalesced.
- Disconnecting removes only FerrisPass's binding metadata. It deletes neither
  the local working copy nor the iCloud file.
- `ferrispass sync status` and the two-phase `sync now` command work with the
  iCloud binding; initial file selection remains a GUI operation.

If iCloud reports that a file is unavailable, confirm that iCloud Drive is
enabled for the macOS account and download the file in Finder before retrying.

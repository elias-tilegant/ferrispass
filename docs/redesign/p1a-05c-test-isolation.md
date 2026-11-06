# P1a.05c: Test isolation

Unit tests no longer read or write the FerrisPass data of the Mac they run on, and they never clear the launch directory of a FerrisPass running there. A test now also holds the rule that copying a password or a username marks an entry as recently used. Nothing changes in the app. The mutation probes and the readings from the manual check are in [PR #29](https://github.com/elias-tilegant/ferrispass/pull/29).

## What the tests read now

**The support folder.** In the app, `app_support_dir()` in `src/sync/config.rs` is `~/Library/Application Support/ferrispass` on macOS. The settings, the recent vaults, the Touch ID registry, the update notes, the sync bindings and the sync lock file all live there, and all of them find it through this one function. In a test build it returns `$TMPDIR/ferrispass-test-<pid>/ferrispass` instead, one folder per test process, made on first use with mode 0700. If an earlier process with the same pid left a folder of that name behind, it is removed first, so a test never starts from what another run saved.

`tests_never_read_the_real_application_support_dir` holds this: the path lies in the temp directory, carries the process id and never `Library/Application Support`, the function returns the same folder every time, and both folders are 0700.

**The launch directory.** Launch payloads go into `$TMPDIR/ferrispass-launch-<uid>`. Locking and quitting remove that directory, and the main window sweeps old payloads out of it when it opens. In a test build the directory is `ferrispass-launch-<uid>-test-<pid>`, so a test that locks or quits clears its own directory and not the one a running FerrisPass is using. It goes through the same checks as the app's: created with mode 0700, a real directory, owned by the user.

`tests_use_a_launch_dir_of_their_own` holds this: the directory `launch_dir()` returns is named after this test process and is not the app's.

Both switches are `#[cfg(test)]` items. A release build has the same paths as before.

## Recently used

The Recently used list shows the entries used since the vault was unlocked. Copying the password, the username or a custom field counts, and so do launching the entry and Auto-Type. Copying the URL does not: someone passing on a link has not signed in with the entry. The new tests hold the rule for the three values the copy commands take:

| Rule | Held by |
|---|---|
| Copying the password marks the entry | `copying_the_password_marks_the_entry_as_recently_used` |
| Copying the username marks the entry | `copying_the_username_marks_the_entry_as_recently_used` |
| Copying the URL does not mark it | `copying_the_url_does_not_mark_the_entry` |

The tests are in `src/ui/legacy/shell/clipboard_tests.rs`. Each opens a vault with one login in the main window, picks the entry, copies one value through `copy_selected_value` onto a fake pasteboard and reads the entries marked as used. The URL test also checks that the URL reached the pasteboard, so it cannot pass on a copy that never happened.

## How to check by hand

In a checkout of this branch:

1. `stat -f %Sm ~/Library/Application\ Support/ferrispass/settings.json` and `ls -la "$TMPDIR/ferrispass-launch-$(id -u)"`.
2. `cargo test --locked`.
3. The same two commands again: the same time and the same listing.

A FerrisPass that is running, and tests from a checkout without this change, use that launch directory as well. Take both readings right before and right after the test run.

## Limits

- Integration tests in `tests/` and the tests of the binaries link the library without `cfg(test)`, so they get the real paths. `vault_round_trip` runs `ferrispass-cli` as a process. The CLI reads the real `settings.json` for the KDF costs the user confirmed, and when it starts it sweeps the real launch directory: it creates the directory if it is missing and removes payloads older than 60 seconds, as the app does. `sharepoint_cli_smoke` runs only on request and would do the same.
- Each test process leaves its folder in `$TMPDIR`. macOS clears that directory on its own schedule.
- The release branches of `app_support_dir` and `instance_tag` are not compiled into the unit tests. `the_instance_tag_is_the_uid` tests `user_tag`, the part the app uses as the whole tag.
- Of the other ways an entry gets marked, launching has no test: it starts SAP GUI through `/usr/bin/open`. These tests do not cover custom fields or Auto-Type either.

The agents spent about 2 hours on this, including review and fixes.

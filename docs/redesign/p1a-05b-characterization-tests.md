# P1a.05b: Characterization tests

Before `src/app/state.rs` and `src/ui/app_shell.rs` are split into smaller files, these tests write down what locking, parking, result routing and the settings file do today. The split is meant to move code without changing what it does, and these tests are how that gets checked: they have to stay green, unedited, through the move and through the first changes built on it. That is why they come first.

Started from `rebuild` at `79b3f9d` on 2026-10-28.

| Commit | What |
|---|---|
| `103f16e` | Locking, parking and result routing in `AppState` |
| `55b511b` | An automatic lock discards an open draft and says so |
| `cd12d11` | Every `settings.json` still loads into 0.9.2 |
| `55048a5` | The quarantine test checks each reason a lock waits for a save on its own |
| the last one | This page |

## What they pin

Each new test was checked by taking out the rule it names, and it failed every time. The removed lines and the failing output are in [PR #9](https://github.com/elias-tilegant/ferrispass/pull/9).

| Rule | Held by |
|---|---|
| Locking revokes the Auto-Type token before the document is dropped | `lock_rotates_auto_type_token_before_dropping_secrets` (new). `rotating_auto_type_context_revokes_only_the_previous_generation` checks the rotation itself. |
| One lock clears every decrypted session: the active one, parked ones and documents held back for a save | `lock_clears_every_parked_and_deferred_session` (new). `lock_vault_clears_parked_and_resets_sync_fields` already covered parked vaults, not held-back documents. |
| A save or sync result lands in the session that started it, and a stale one changes nothing | `results_land_in_the_session_that_dispatched_them` and `stale_session_results_change_nothing` (new), next to `apply_save_status_routes_to_parked_vault`, `stale_save_result_cannot_mark_reopened_same_path_as_saved` and `a_status_for_a_parked_vault_never_lands_on_the_active_one`. None of the older ones noticed a stale result reaching a parked vault. |
| Never two decrypted sessions for one file | `never_two_sessions_for_one_file` (new, headless test through `request_password`) |
| Locking with unsaved work quarantines the document instead of dropping it, whether a save is still being written or the last one failed | `unsaved_work_quarantines_instead_of_dropping` (new, through the public `lock_vault`, once for each reason). `deferred_lock_waits_for_dirty_session_and_writer_slot` and `deferred_lock_quarantines_all_dirty_sessions_and_drops_clean_ones` call the quarantine step directly. |
| An automatic lock discards an open draft and tells the user | `automatic_lock_discards_the_draft_and_says_so` (new, headless UI test with the real `AppShell`) |
| Every `settings.json` this build writes still loads into 0.9.2 | `new_settings_load_into_the_092_struct` in `tests/settings_compat.rs` (new), against a frozen copy of the 0.9.2 struct. The tests in `settings.rs` only load old files into the current struct. |

## Test hooks

No production code changed. Three small additions exist only in test builds:

- `AppState::open_for_test` puts a document in the active slot without a file or a KDF. `park_tests` uses it too.
- `lock_vault_now` calls a witness right before it drops the active document, so a test can see that the Auto-Type token was already revoked at that point and not only afterwards.
- `AppShell::notify_error` keeps the last message, because a notification's text can't be read back from a headless window.

## Limits

- Switching back to a parked vault writes the recent-vaults list to the real Application Support folder. `never_two_sessions_for_one_file` drops that queued write before the test harness would run it. `unsaved_work_quarantines_instead_of_dropping` does the same with the save a lock retries after a failed one.
- `automatic_lock_discards_the_draft_and_says_so` builds the real `AppShell`. It reads the real `settings.json`, and the lock clears the launch temp folder, as it does in the app.

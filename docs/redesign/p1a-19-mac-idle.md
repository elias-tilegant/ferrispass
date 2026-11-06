# P1a.19: Mac idle time as an alternative for the auto-lock

The auto-lock goes by input in FerrisPass. Four minutes without a key press or a click in FerrisPass and the vault locks, even while you are typing away in another app. Settings now has a second row under the auto-lock, "Or lock after the Mac is idle for", with Off, 5 min, 15 min and 30 min. With a value there, the auto-lock goes by how long the Mac itself has had no input, so FerrisPass stays open while you work in other apps and locks once you leave the Mac alone. Off is the default, and with Off nothing changes.

Started from `rebuild` at `7566d72` on 2026-11-06.

| Commit | What |
|---|---|
| `fb95ded` | `src/platform/system_idle.rs`, `activity::lock_is_due_with`, and the module on the list of audited platform modules |
| `a6036c3` | The setting, the auto-lock task reading both clocks, and the second row in Settings |
| `6b61d2e` | SECURITY.md and this page |
| `c658d24` | What a trip back to 0.9.2 does to the setting, and which calls of the idle clock are new |
| `4838fd2` | This table and the time below |
| `3e780db` | A shell test that a Mac in use does not hold off the lock while the Mac idle row is off |
| `ee8f3ed` | When 0.9.2 writes its settings file, this table and the time below |
| `d23d3a8` | The trip back to 0.9.2 as an accepted limit, here and in SECURITY.md, this table and the time below |
| `395eb3d` | The scan of the `unsafe` code fails on a name with retain or release in it in any letter case, a test of that name scan, the section on it here, the list of tests, this table and the time below |
| the last one | What the two settings screen tests hold and what is left to the check by hand: the tests section, the list of tests, the check by hand, the limits, this table and the time below |

## When the vault locks

`activity::lock_is_due_with` makes the decision on every tick of the auto-lock task, every five seconds:

| Mac idle setting | What macOS reports | The vault locks when |
|---|---|---|
| Off | not asked | FerrisPass has had no input for the auto-lock time, as before |
| On | an idle time | the Mac has had no input for the Mac idle time; input in FerrisPass and the auto-lock time play no part |
| On | nothing usable | FerrisPass has had no input for the Mac idle time |

The last row is the fallback. When macOS can't say how long the Mac has been idle (an error, or a reading that is NaN, negative or infinite), FerrisPass goes by its own idle clock again, with the time from the Mac idle row. It never treats a missing answer as "not idle", so with the Mac idle row on there is no reading that keeps the vault open for good. A wall clock that jumps backwards still counts as idle long enough wherever FerrisPass's own clock decides, as it did before.

The auto-lock task runs while a vault is open and either row has a value. Its tick only asks macOS for the idle time while the Mac idle row is on. Sleep, the lock screen, the screen saver and a user switch lock the vault at once, as before; that is the session lock in `src/session_lock.rs`, which this step doesn't touch.

## What counts as input on the Mac

The idle time comes from CoreGraphics, `CGEventSourceSecondsSinceLastEventType` for the combined session state and any input event type. It needs no permission. It counts every key press, click, mouse and trackpad movement and scroll in the login session, in any app.

It also counts input that a program posts to the session as if it were typed or moved. That includes FerrisPass's own Auto-Type, a presentation remote, remote-control software, and the "mouse jigglers" that keep a Mac awake. While one of those runs, the Mac never looks idle, and the vault stays open until you lock it, the Mac sleeps or the screen locks. That is the limit of this clock, and the reason it is an alternative you choose and not the default: the default only counts input you give FerrisPass. Input in another user's session after a fast user switch does not count, and the switch itself locks the vault anyway.

## The two rows in Settings

The rows are independent. Choosing a value in one keeps the other's, so the auto-lock time is still there when the Mac idle row goes back to Off, and then it applies alone again. Nothing in the app clears one setting because of the other.

So that two selected rows don't leave it open which one applies, the auto-lock row's text changes while the Mac idle row is on. It keeps its usual sentence and adds: "While the Mac idle setting below is on, this applies only when macOS cannot report idle time, and then with the time chosen there."

The setting is a new field in `settings.json`, `mac_idle_lock_secs`, in seconds, `null` for Off. A file without it reads as Off, and older builds ignore it, which `a_settings_file_with_the_mac_idle_field_loads_into_092` holds against the frozen 0.9.2 settings. The row offers 5, 15 and 30 minutes; other values can be written into the file by hand.

## The unsafe code

`src/platform/system_idle.rs` is the second module under `src/platform/`, the one place new `unsafe` code may go. No crate we depend on offers the CoreGraphics function as a safe call, so the module declares it in an `unsafe extern` block and calls it in one `unsafe` block, with two integer constants from the header (`kCGEventSourceStateCombinedSessionState` is 0, `kCGAnyInputEventType` is `~0`). A `double` comes back. There is no pointer, nothing is retained or released, there is no callback and no state, and the function has no thread requirement. The reading is checked before it becomes a `Duration`: NaN, negative and infinite values are `IdleError::Invalid`, and `Duration::from_secs_f64` is never called on them. On other platforms the answer is `IdleError::Unsupported`.

`system_idle_declares_one_foreign_function_and_one_unsafe_call` reads the file's own source with a small tokenizer, the one from `tests/architecture.rs` cut down, so comments and strings don't count. It requires exactly two `unsafe` tokens, the `extern` block with exactly the one declaration, the `unsafe` block with exactly the one call, both constants with their values, none of `as *`, `*const`, `*mut`, `static mut`, `into_raw`, `from_raw` or `transmute`, and no name with retain or release in it, in any letter case, such as `release`, `objc_retain` or `objc_release`. `the_refcount_scan_ignores_letter_case` runs that name scan over a few lines of code and expects it to report `release`, `objc_retain`, `objc_release`, `CFRetain`, `CFRelease` and `Release`, and nothing from the comment `// retain` or the string `"release"` in them. `tests/architecture.rs` lists the module in `PLATFORM_MODULES`, and SECURITY.md lists it under "Unsafe code".

Tests reach the idle time through the `App`: `idle_for(cx)` reads a fake that the test put on its own `App` as a GPUI global, and without one a test build answers `Unsupported`. Tests that run side by side, each with its own context, never see each other's value, and no unit test depends on whether someone is using the Mac that runs it.

## Tests

The rules of the lock decision, the idle reading, the `unsafe` code, the settings file and the auto-lock and session lock tasks were each checked by taking one out; the changed lines and the failing output are in [PR #27](https://github.com/elias-tilegant/ferrispass/pull/27).

The two settings screen tests are not among them. They test what the screen is built from, not the screen: the presets and labels of the Mac idle row, `auto_lock_description` for either state, and `with_auto_lock` and `with_mac_idle_lock`. Neither renders the screen, so both stay green if it stops showing the Mac idle row, hands the auto-lock row the wrong state, or a click in one row writes the other row's field. That wiring is left to the check by hand on a preview build before the merge, see below.

| Test | What it holds |
|---|---|
| `mac_idle_wins_when_the_alternative_is_set` | with the Mac idle threshold set, only the Mac's idle time counts |
| `an_unreadable_mac_idle_falls_back_to_the_app_clock_with_the_mac_threshold` | either error falls back to FerrisPass's clock with the Mac threshold, whatever the auto-lock time |
| `without_the_alternative_the_app_clock_rules` | without the Mac idle threshold, the decision is `lock_is_due` as before, for any reading |
| `a_backwards_clock_is_due_at_once` | a wall clock that went back makes the lock due wherever FerrisPass's clock decides |
| `a_nan_or_negative_reading_is_an_error` | NaN, negative, infinite and too large readings are `Invalid`, through the same path the real reading takes |
| `system_idle_is_unsupported_off_macos` | the no-op answers `Unsupported`, and so does a test build without a fake |
| `the_real_call_answers_on_macos` | CoreGraphics itself answers with less than a day, on a test thread |
| `idle_for_reads_core_graphics` (doctest) | `idle_for` in the library as the app builds it reads CoreGraphics, not the no-op |
| `the_fake_is_read_through_the_app` | two test contexts, each with its own fake, each read their own value |
| `system_idle_declares_one_foreign_function_and_one_unsafe_call`, `the_scan_skips_comments_and_strings` | the shape of the `unsafe` code, and the tokenizer that reads it |
| `the_refcount_scan_ignores_letter_case` | the scan for names with retain or release in them finds them in any letter case, and not in a comment or a string with one of the words |
| `unsafe_code_stays_on_the_exception_list` | the module is on the list, with its header and its attribute |
| `auto_lock_without_mac_idle_ignores_the_mac` | with the Mac idle row off and the Mac in use, three idle minutes in FerrisPass keep the vault open, just over four lock it |
| `auto_lock_with_mac_idle_ignores_the_app_clock` | with the Mac in use, five and twenty idle minutes in FerrisPass keep the vault open |
| `auto_lock_with_mac_idle_locks_when_the_mac_is_idle` | the Mac idle for one second short of fifteen minutes keeps it open, fifteen lock it |
| `auto_lock_with_mac_idle_falls_back_when_macos_cannot_say` | with a NaN reading, five idle minutes in FerrisPass keep it open, fifteen lock it |
| `the_auto_lock_task_runs_with_only_the_alternative_set` | the task starts with the auto-lock on Never and the Mac idle row set, and locks |
| `a_session_lock_event_still_locks_with_the_alternative` | a lock event from the session lock locks at once while the Mac idle row is on |
| `mac_idle_lock_secs_is_off_by_default_and_round_trips`, `an_older_settings_file_reads_the_field_as_off` | the field is off by default, survives a save and load, and an older file parses with it off |
| `a_settings_file_with_the_mac_idle_field_loads_into_092` | a file with the field, set or off, loads into 0.9.2 with every other choice as written |
| `the_mac_idle_row_offers_off_5_15_and_30_minutes` | the presets the Mac idle row is built from, and their labels |
| `the_auto_lock_row_says_it_is_the_fallback_while_mac_idle_is_set` | `auto_lock_description` with and without the Mac idle setting, and that `with_auto_lock` and `with_mac_idle_lock` each keep the other field's value |

The shell tests above build the real `AppShell` and set both settings themselves. `activity_is_touched_only_where_listed`, `tests/idle_clock.rs` and the characterization tests are unchanged and pass. Outside tests, this step adds no call of `activity::touch` or `activity::reset`; the new shell tests set the app's clock with `activity::reset`, as the tests before them do.

## How to check by hand

Not run yet. It comes before the merge, on a preview build with a vault open. The first two steps and the last one are also the only check of the settings screen's wiring: that it shows the Mac idle row, hands the auto-lock row the Mac idle setting, and that a click in each row writes that row's own field.

- In Settings, the row "Or lock after the Mac is idle for" is under the auto-lock row, with Off selected. Set it to 5 min and leave the auto-lock at 4 min. The Mac idle row shows 5 min, the auto-lock row still shows 4 min and now has the sentence about macOS not reporting idle time.
- Set the auto-lock to 1 min and back to 4 min. The Mac idle row still shows 5 min.
- Work in other apps for six minutes, typing and moving the mouse, without touching FerrisPass. The vault stays open.
- Take your hands off the Mac. About five minutes later the vault is locked.
- Unlock, close the lid, open it again. The vault is locked; that is the session lock, not the idle clock, and it has to stay that way.
- Set the Mac idle row back to Off. The auto-lock row has its usual text again and still shows 4 min.

To try a shorter time, quit FerrisPass and set `mac_idle_lock_secs` to 60 in `~/Library/Application Support/ferrispass/settings.json`. No row is selected in Settings then, as with any value the row doesn't offer.

## Limits

- Input that programs post counts as input on the Mac, see above. With an Auto-Type, a presentation remote or a mouse jiggler running, the Mac idle time stays near zero, and only the session lock, the lock button or the menu locks the vault.
- The settings screen is covered by unit tests over what it is built from (the presets, the labels, the auto-lock text and the two functions that build the new settings), not by rendering it or clicking its rows: in a test build a click would write the real `settings.json`. No test fails if the screen leaves out the Mac idle row, hands the auto-lock row the wrong state, or a row writes the other row's field. The check by hand covers that.
- `a_session_lock_event_still_locks_with_the_alternative` sends its event through a fake that a test can put on the `App` in place of the session lock monitor's events. `start_session_lock_task` takes its events from there when a test installed it. The macOS notifications themselves are covered by the tests in `src/session_lock.rs`.
- Test builds never read the real idle time through the `App`. `the_real_call_answers_on_macos` calls CoreGraphics directly, and the doctest `idle_for_reads_core_graphics` reads it through the library as the app builds it. Both expect someone to have used the Mac within the last day.
- A trip back to 0.9.2 can lose the setting. 0.9.2 reads a file with the new field, but whenever it writes `settings.json` it writes only the fields it knows. It writes the file when it saves a setting (any change in its Settings, the theme toggle, allowing a vault's key derivation cost; `v0.9.2:src/ui/app_shell.rs:3761-3783`), and also when its window is closed or the app quits with the window at a position or size other than the one on file, which any move or resize brings about (`v0.9.2:src/app/actions.rs:218` and `:240`, through `persist_window_bounds` in `v0.9.2:src/app/settings.rs:223-237`). Moving the window once and closing it is enough. The field is gone when you come back, and the Mac idle row reads Off. With the auto-lock on 4 minutes that only means the default clock again. With the auto-lock on Never, there is no idle lock at all until you choose the Mac idle time again; 0.9.2 itself already read that file as Never and did not auto-lock in between. Nothing here can tell a field 0.9.2 dropped from Off chosen by hand. This is an accepted limit and stays as it is: every new settings field goes the same way on the way back to 0.9.x, and sleep, the lock screen and the screen saver still lock the vault at once. SECURITY.md says it in its paragraph on the Mac idle rule.
- `#[serde(default)]` on the new field changes nothing that a test can see: serde reads a missing `Option` field as `None` without it too. It stays because it says what is meant, like on the other fields.
- CI only builds for macOS, so the no-op for other platforms is compiled there only for the test build.
- The shell tests build the real `AppShell` and read the real `settings.json`. They set both auto-lock settings themselves.

The agents spent about 4 hours on this, including review and fixes.

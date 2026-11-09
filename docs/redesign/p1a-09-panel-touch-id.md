# P1a.09: Touch ID from the quick access panel

This step builds what the locked quick access panel will run on, without the panel itself: a buffer for what you type before the vault is unlocked, one Touch ID flow that the unlock screen and the panel share, the panel's way into that flow, and the rules for when the panel takes Return. The old unlock screen already runs on the shared flow. Nothing on screen changes; the panel comes in a later step, and so do the checks that need it.

Started from `rebuild` at `7566d72` on 2026-11-06.

| Commit | What |
|---|---|
| `625e842` | The typed-ahead buffer, its tests and the scan that keeps its text out of logs and files |
| `bd2227d` | The one Touch ID flow in `src/app/unlock/touch_id.rs`; the unlock screen calls it |
| `77df130` | The panel's way in, with the confirmation token |
| `0511acc` | When the panel takes Return, and what the locked panel offers |
| `b2dc438` | The list of places that can raise the Touch ID prompt, and the automatic prompt kept inside the shell |
| `2189398` | SECURITY.md and this page |
| `e79d238` | The call-site list also holds every call of `confirm` and of the two ways into the flow |
| `038b1ba` | The ordering test for `on_answered` runs for every way the prompt can end |
| `441561b` | `wipe` held to `zeroize`, with every method called on the typed text listed |
| `63b4241` | Only the two ways in call the flow, and only the panel's way in makes a panel origin |
| `67ca17e` | This page, with what the tests cover |
| `99e744b` | `push` asserts that `String::push` finds room, with a test that types up to the edge of the allocation |
| `0bbbad1` | This page, on how the buffer grows |
| `3ae5e2f` | Tests that the Touch ID prompt takes the Mac password only when it is allowed, from the unlock screen and from the panel |
| `867a5f2` | This page, on the same |
| `578ee07` | A test that a click macOS withdrew while the automatic prompt's retry is still to come brings no other prompt, with the window in front and when it comes back from the background |
| `0f56c9b` | The test of the Mac password setting also covers the prompt the unlock screen raises on its own, with the setting off |
| `bc4460b` | The test of the automatic retry also has a click withdrawn with the window in the background, after the retry is spent |
| `02f8ee9` | This page, on the same |
| `c845fcb` | Line breaks in those tests as rustfmt sets them |
| `b7c69a0` | This page, on the two commits before it |
| `bab763f` | `on_answered` runs as soon as macOS answers, before the vault is opened, with a test that holds the order |
| `bfb8b00` | Secure input from the moment the locked panel is key: `begin` holds it for the empty buffer, and the 30 seconds start with the first key; SECURITY.md on the same |
| `9112a48` | This page, on the same |
| `fa92666` | The buffer keeps its text and its clock while the Touch ID dialog has key status: `release_secure_input` lets go of secure input, `rearm_secure_input` takes it again, and `take_query` takes the time and gives nothing for text that has expired |
| `91f76fc` | The panel's way in starts nothing while an unlock is under way, and `kind` says `Unlocking` for that time |
| `7c4d7bb` | Tests that an attempt ended by a lock or by asking for another vault opens nothing, before the open and after it |
| `04588eb` | SECURITY.md and this page, on the same |
| `2502685` | `backspace` takes no key while the buffer has let go of secure input or is wiped, as `push` does |
| the last one | This page, on the same and on what the unlock screen does while an unlock is under way |

## The typed-ahead buffer

`src/app/unlock/typed_ahead.rs`. The panel lets you start a search before Touch ID has unlocked the vault. What you type goes into a `TypedAhead`, and the panel only ever learns how many characters there are (`len`), to show as dots.

- The text sits in a `Zeroizing<String>`. `wipe` and the drop overwrite it, spare capacity included, and `take_query` hands it over in that same zeroizing string. Growing it never leaves a copy: when a character doesn't fit in the room left, `push` copies the text into a bigger zeroizing string and the old one is overwritten as it drops, instead of letting `String::push` reallocate and free the old memory as it was. In debug builds `push` then asserts that `String::push` finds room, worked out apart from the condition that decides on the copy. `a_character_that_does_not_fit_grows_the_buffer_by_copying` types up to the last free byte with a character of each UTF-8 length (`n`, `ä`, `€`, `🦀`): with exactly its bytes left it goes in place, with one byte fewer the text is copied first. A condition that misjudges a multi-byte character, such as one that copies only once every byte is in use, fails there.
- Secure input covers the locked panel from the moment it is key. The panel calls `begin` as soon as it is key while locked, before any key arrives, and `begin` takes a secure input guard (`secure_input::hold`) for the empty buffer, so other programs don't see even the first keystroke. The keys typed after that go into the same buffer under the same guard, which goes when the buffer is wiped, expires, hands its text over or is dropped. If macOS refuses secure input, or the call is off the main thread, `begin` fails and there is no buffer: the panel then sends you to the main window. Where the platform has no secure event input at all, the buffer starts without a guard and claims nothing.
- The buffer lives on while the Touch ID dialog has key status. `release_secure_input` lets go of secure input and keeps the text and its clock, and neither `push` nor `backspace` takes a key until `rearm_secure_input` has taken secure input again, once the panel is key again. A buffer that still holds secure input is left as it is, so it never takes a second guard. One that expired while the dialog was up starts over empty, under secure input, as a new one from `begin` does. If macOS refuses secure input at that point, the text is wiped and the error comes back, as from `begin`.
- It is wiped 30 seconds after the first keystroke (`EXPIRY`), also while the Touch ID dialog is up, on cancel (`wipe`), and when it looks like a password rather than a search: more than one word and not a single match (`wipe_if_password_like`). The 30 seconds start with the first `push`, not with `begin`, so a panel that has been key for a while doesn't lose what you type a moment later. With nothing typed there is nothing to expire, and the buffer keeps secure input for as long as the panel keeps it. `expire_if_due` and `wipe_if_password_like` wipe on their own and say whether they did; there is no check that only reports and leaves the wiping to the caller. The 30 seconds are wall-clock time, like the auto-lock's idle clock, so a Mac that slept through them finds the buffer expired, and a clock that went backwards counts as expired too.
- A wiped buffer stays empty, has let go of secure input and takes no more keystrokes. `take_query` hands the text over once the vault is open, as the search to run, and the buffer is gone with it. It takes the time: text that has expired by then is wiped instead, and the search starts empty.
- `Debug` prints `TypedAhead(<n> chars)`. Nothing in the file logs, prints or writes, and `typed_ahead_never_logs_or_writes` in `tests/architecture.rs` reads the file to keep it that way: no file system, log or print names, no derives, the field only in the functions listed in `TYPED_TEXT_USERS`, and never inside a macro call.
- `typed_ahead_wipes_by_overwriting` reads the same file for what is done to the text. `TYPED_TEXT_CALLS` lists every method called on it, with the function it is called in, and `wipe` has to be the one that calls `zeroize`. A method that only shortens the string (`clear`, `truncate`) leaves the typed bytes in its allocation until the buffer drops, and one that regrows or shrinks it (`reserve`, `push_str`, `shrink_to_fit`) frees the old allocation without overwriting it; either fails the test, and so does a mutable borrow of the text. No safe test can read those bytes back, so the list holds the code to its shape. The list allows `String::push` in `push`, which would regrow a string that is out of room too, so it can't tell whether `push` copies in time; the assertion in `push` does.
- The buffer is not `Send`, because the guard isn't: it lives and dies on the main thread.

A search never marks an entry as recently used; only copying a value or launching an entry does. `the_typed_query_never_reaches_last_used` takes a query out of a buffer, searches with it and then copies, and the list of recently used entries holds the entry and nothing else.

## One Touch ID flow, two ways in

`src/app/unlock/touch_id.rs` holds the Touch ID unlock that used to live in `start_biometric_unlock` in the shell. `begin_biometric_unlock` stamps a new generation, and the keychain read behind the prompt runs in the background. As soon as macOS has answered, `on_answered` runs on the foreground. With the password, the vault is then opened in a background task of its own, and the answer goes through `begin_open` and `finish_open_attempt` on the foreground; a failed prompt goes to `fail_biometric_unlock` right after `on_answered`. The open waits for `on_answered` because it can take seconds with an expensive Argon2 key derivation or a slow disk, and the panel has to have key focus back first, or keys typed in that time could go to another app. The prompt still reads "Unlock <file name>". The flow is the private `run`; nothing outside the module can call it, and no public function takes an origin or a trigger as a plain value. Inside the module, `the_raw_flow_has_no_public_entry` allows a call of `run` only in `unlock_from_lock_screen` and `unlock_from_panel`, and the `Panel` origin only in `unlock_from_panel` and in the comparison in `run`, so a new function in the module can't start the panel's flow without a token either.

| | Unlock screen | Quick access panel |
|---|---|---|
| Way in | `unlock_from_lock_screen`, from the Touch ID button, its shortcut, and on its own when the screen comes up with the window in front | `unlock_from_panel`, which takes a `Confirmed`. Only `confirm(PanelTrigger::ReturnKey)` and `confirm(PanelTrigger::UnlockButtonClick)` make one |
| macOS withdrew the prompt | the automatic prompt asks once more, as before. A withdrawn click is left for the user to click again: it brings no other prompt, neither at once with the window in front nor when the window comes back from the background | `Answer::Interrupted`, nothing else. The next attempt needs a new confirmation |
| As soon as macOS answers, whatever the answer, before the vault is opened and before the answer is applied | nothing | `on_answered` runs: the panel makes itself key and focuses the search field or the list |
| A cost still to confirm, or the header not read yet | the click counts as allowing the cost; the automatic prompt waits | no attempt starts |
| An unlock already under way, from the prompt until the vault is open or the attempt has ended | the Touch ID button, where it shows, says `Waiting…` and takes no click, whichever way in started the attempt, and the automatic prompt is scheduled only while no attempt is in flight, as before | no attempt starts, for Return and for the unlock button alike, and `kind` says `Unlocking` |

The panel waits for an unlock under way because a second attempt makes the first one stale: the first answer then ends the second attempt too, and the vault stays locked after two prompts answered with a finger. `unlock_from_panel` therefore returns `None` while a Touch ID attempt waits for its answer or for its vault to open (the attempt stays in flight until the answer is applied), or while the vault is being opened with its password. The token is used up all the same, and neither callback runs.

The token has a private field, so no code outside the module can build one. It is neither `Clone` nor `Copy`, and `unlock_from_panel` takes it by value, so each attempt uses one up. Two doctests on `Confirmed` don't compile, on purpose: one builds a token by hand, one clones it.

Rules both ways share:

- An answer from an attempt that a newer one replaced, or that ended because the vault was locked or another vault was asked for, opens nothing and ends the attempt in flight (`Answer::Stale`), as the unlock screen always did. That is checked when the answer comes, so a stale password never goes on to the open (`a_stale_password_never_goes_on_to_the_open`), and again once the open is done, for an attempt that ended while the vault was being opened (`an_attempt_ended_while_the_vault_opens_opens_nothing`).
- The task is detached inside the module. Nobody holds a handle, so dropping what the call returns cannot cancel an unlock, and neither the shell nor the panel needs a field for it. The functions return the generation of the attempt they started, or `None`.
- `on_done` gets the answer once it is applied: `Opened(path)`, `NotOpened` (Touch ID answered, but the stored password no longer fits, the file is gone, or the cost has to be confirmed; the unlock screen says which), `Failed(error)`, `Interrupted` or `Stale`.

The unlock screen's own rule, one more automatic prompt after macOS withdrew it, stays in the shell: `start_biometric_unlock` passes an `on_done` that calls `note_automatic_biometric_interruption` for an interrupted automatic attempt. `schedule_automatic_biometric_unlock` stays `pub(super)`, and `the_automatic_prompt_has_no_public_entry` in `tests/architecture.rs` holds it and the three functions around it to no more than that.

`TOUCH_ID_CALL_SITES` in `tests/architecture.rs` lists every call outside tests of `begin_biometric_unlock` and `retrieve`, of the two ways into the flow and of `confirm`, one line per call with the function it is in: the two calls in `run`, the unlock screen's `start_biometric_unlock`, and the command line's `unlock_password`. Nothing calls `confirm` or `unlock_from_panel` yet. A token only says that `confirm` was asked with Return or the unlock button, not that the key was really pressed, so the panel's calls will come in as lines that name the key and click handlers they sit in; a timer or an activation handler on the list would stand out. `touch_id_starts_only_where_listed` fails on a call that is not on the list, on a line that has no call left, and on any of these names used without being called.

## When the panel takes Return, and what it offers while locked

`src/app/unlock/locked_panel.rs`.

`return_accepted` holds Return back until the results are drawn, 300 ms have passed since (`RETURN_SETTLE`), and every vault the panel searches has finished unlocking. Return then acts on what you saw, not on a row that arrived a moment later. One exact match in a vault that was already open takes Return as soon as it is drawn, unless it is a production SAP system (`exact_hit_takes_return_at_once`, which asks `EntryKind::is_production`). Return before anything is drawn is always dropped. For now the panel searches one vault, so "every vault" is that one; a vault for which `kind` says `Unlocking` is not done.

`kind` decides what the locked panel offers:

| Case | Kind |
|---|---|
| A Touch ID attempt waits for its answer or for its vault to open, or the vault is being opened with its password | `Unlocking` |
| No vault waits on the unlock screen and there is no recent one | `NotSetUp` |
| The vault's file is not there | `FileMissing(path)` |
| A cost to confirm, or the header check not back yet | `NeedsMainWindow` |
| No Touch ID enrolment for the vault | `NeedsMainWindow` |
| Enrolled, the sensor in reach | `TouchId { device_password: false }` |
| Enrolled, no sensor (a closed lid), the Mac password allowed | `TouchId { device_password: true }` |
| Enrolled, no sensor, the Mac password not allowed | `NeedsMainWindow` |

The vault is the one waiting on the unlock screen, or else the most recent one. Only the one on the unlock screen has had its header read, so for a recent vault `kind` answers `NeedsMainWindow` until the panel puts it there with `AppState::request_password` and the shell's header check comes back.

## What the panel will do with it

- Begin a `TypedAhead` as soon as the panel is key while locked, before any key arrives, so secure input covers the locked panel from the moment it is key. Put every typed key into it, show `len()` dots, call `expire_if_due` on every tick and keystroke, the ticks while the Touch ID dialog is up included, and `wipe_if_password_like` after every search, and `wipe` on Esc. When one is wiped while the panel is still key and locked, begin the next one at once.
- When the panel loses key status while `kind` says `Unlocking`, as it does when the Touch ID dialog comes up, call `release_secure_input`: the buffer keeps its text and its clock. As soon as the panel is key again, in `on_answered`, call `rearm_secure_input`. When the panel loses key status any other way, `wipe` the buffer, so secure input goes with it, and begin a new one once it is key again and still locked.
- Ask `kind` what to show, and put the most recent vault on the unlock screen when none waits there.
- On Return or a click on the unlock button, and only then, call `confirm` and hand the token to `unlock_from_panel`. While `kind` says `Unlocking`, Return and the click start nothing, and `unlock_from_panel` refuses them as well. In `on_answered`, make the panel window key (`Window::activate_window`) and focus the search field or the list. The report on Touch ID from the panel in `docs/gpui-matrix.md` found the panel no longer key after the prompt in every run that showed one, and Return in the list did nothing until the panel was clicked.
- Check `return_accepted` for every Return in the results.
- Once the vault is open, run `take_query` with the current time as the search. Text that has expired by then gives an empty search.

## What the tests show, and what needs a Mac

PR #28 has a table of changes that each break one of these rules, with the test that failed and its output.

| Rule | Test |
|---|---|
| The buffer holds secure input while it lives, and lets go when wiped or dropped | `typed_ahead_holds_secure_input_while_it_lives` |
| Secure input is on as soon as `begin` returns, before the first key, and stays on with nothing typed, an hour on too; the keys after it take no second guard, and the guard goes once, when the buffer is wiped, expires, hands its text over or is dropped | `secure_input_is_on_before_the_first_key` |
| The 30 seconds start with the first key, not with `begin`, and a later key doesn't start them again; before the first key, not even a clock that went backwards expires the buffer | `the_thirty_seconds_start_with_the_first_key` |
| 29 seconds no, 30 yes | `typed_ahead_expires_after_thirty_seconds` |
| `expire_if_due` wipes and lets go of secure input | `expire_if_due_wipes_the_buffer_and_releases_the_guard` |
| A clock that went backwards counts as expired | `a_clock_that_went_backwards_counts_as_expired` |
| Several words without a match are wiped; one word, or matches, are not | `a_multi_word_query_without_a_hit_is_wiped_as_password_like` |
| `Debug` shows no character of the text | `typed_ahead_debug_shows_no_characters` |
| A wiped buffer is empty, has let go of secure input and gives no text back; `wipe` overwrites the text, and no method beyond the listed ones is called on it, so a `clear`, `truncate`, `reserve`, `push_str` or `shrink_to_fit` fails | `wipe_leaves_nothing_behind`, `typed_ahead_wipes_by_overwriting` |
| A character that doesn't fit in the room left, of any UTF-8 length, has the text copied into a bigger buffer first, so `String::push` never grows it (an assertion in `push`, in debug builds) | `a_character_that_does_not_fit_grows_the_buffer_by_copying`, `typing_past_the_first_allocation_keeps_every_character` |
| A refusal by macOS is an error, no buffer | `typed_ahead_refuses_when_macos_refuses_secure_input` |
| Without secure event input on the platform, the buffer still works | `typed_ahead_runs_without_a_guard_where_there_is_none` |
| The buffer can't leave the main thread | `typed_ahead_is_not_send` (doctest) |
| No log, print, file or derive in the buffer's file | `typed_ahead_never_logs_or_writes` |
| While the Touch ID dialog has key status, secure input is off and no key goes in; once the panel is key again it is on, and the text and its clock are as they were. A buffer that holds secure input takes none a second time | `secure_input_is_released_and_taken_again_around_the_dialog` |
| While secure input is let go of, a backspace changes nothing either; once it is back, a backspace takes the last character. A wiped buffer takes neither a backspace nor a key | `backspace_takes_no_key_while_released_or_wiped` |
| The 30 seconds run on while the dialog is up; a buffer wiped then starts over empty under secure input once the panel is key again | `the_thirty_seconds_run_on_while_the_dialog_is_up` |
| Text that has expired gives no search, with secure input held or let go | `an_expired_buffer_yields_no_query` |
| If macOS refuses secure input when the panel is key again, the text is wiped and no key goes in | `a_refused_rearm_wipes_the_buffer` |
| The unlock screen unlocks through the shared flow with one prompt, named after the vault; with the settings at their defaults, the prompt takes the Mac password as the default allows | `the_lock_screen_unlocks_through_the_shared_flow` |
| The prompt takes the Mac password only when it is allowed: from the unlock screen as its setting says, for the prompt the screen raises on its own with the setting off and for a click with the setting turned off and then on again while the app runs, and from the panel as `unlock_from_panel` is told, first not and then yes | `the_lock_screen_prompt_follows_the_mac_password_setting`, `the_panel_prompt_takes_the_mac_password_only_when_allowed` |
| A withdrawn automatic prompt is asked again once. A withdrawn click brings no other prompt, neither at once with the window in front nor when the window comes back from the background, whether that one retry is spent or still to come | `an_interrupted_automatic_prompt_is_retried_once_on_the_lock_screen`, `a_withdrawn_click_is_not_asked_again_on_the_lock_screen` |
| Dropping what the call returns doesn't cancel the unlock | `an_unlock_completes_without_anyone_holding_a_task` |
| A stale answer opens nothing | `a_stale_answer_opens_nothing` |
| An attempt ended while the prompt was up, by locking the vault or asking for another one, gets its answer as stale in the step that takes it, before any open | `a_stale_password_never_goes_on_to_the_open` |
| An attempt ended in the same ways while its vault was being opened leaves the vault shut | `an_attempt_ended_while_the_vault_opens_opens_nothing` |
| A failed prompt is reported | `a_failed_retrieve_reports_the_error` |
| A search never marks an entry as recently used | `the_typed_query_never_reaches_last_used` |
| Only Return and the unlock button confirm | `confirm_is_a_closed_table` |
| Opening, typing, coming back to the front, a timer, an expired buffer or a withdrawn prompt never start Touch ID | `the_panel_starts_touch_id_only_with_a_token` |
| The raw flow has no public way in, only the two ways in call it, only the panel's makes a panel origin, and the token can't be made or copied elsewhere | `the_raw_flow_has_no_public_entry`, and the two doctests on `Confirmed` |
| A withdrawn prompt in the panel is not asked again | `an_interrupted_panel_prompt_is_not_retried` |
| After the password, a cancel, a failed match, a prompt macOS withdrew, and an answer that comes after the user moved on, `on_answered` runs before the answer is applied and before `on_done`, and Return reaches the panel only after it | `the_panel_regains_key_before_it_takes_return` |
| With the password, `on_answered` runs before the vault is opened, through the panel's way in and with the unlock screen's origin: it writes the vault file the open reads, so the vault opens only if it ran first | `on_answered_runs_before_the_vault_is_opened` |
| The panel never answers a cost question | `the_panel_never_answers_a_cost_question` |
| While an attempt is under way, with the prompt up and with its vault being opened, neither Return nor the unlock button starts another one, `kind` says `Unlocking`, and the first attempt opens the vault; a cancelled prompt is not under way | `the_panel_starts_no_second_prompt_while_one_is_under_way`, `the_locked_panel_says_when_an_unlock_is_under_way` |
| Every locked case | `locked_panel_kind_for_every_case` |
| Return before drawing, within 300 ms, and before every vault is done is dropped; one exact match in an open vault goes at once unless it is production | `return_before_results_are_drawn_is_dropped`, `return_within_300ms_of_drawing_is_dropped`, `return_waits_for_every_vault_of_the_set`, `one_exact_hit_in_an_open_vault_takes_return_at_once_unless_production` |
| Touch ID starts only where listed; the automatic prompt stays inside the shell | `touch_id_starts_only_where_listed`, `the_automatic_prompt_has_no_public_entry` |

The characterization tests and `src/app/state/biometric_tests.rs` are unchanged and pass.

On a Mac, with a preview build, the unlock screen gets the same checks as before, since it now runs on the shared flow: Touch ID with the button and with Return unlocks; with the window sent to the background and brought back, the automatic prompt comes again, once; cancelling the prompt brings no second one without a click; and with a loop running `ioreg -l -w 0 | grep -o '"kCGSSessionSecureInputPID"=[0-9]*'` in a Terminal window behind FerrisPass, the PID shows while the master password field has focus and is gone after the unlock.

The panel's own checks need the panel: Return working after Touch ID without a click into the panel, focus going back to TextEdit, SAP GUI and Safari afterwards, a panel tick that leaves out `expire_if_due`, the ticks while the Touch ID dialog is up included, making a test fail, and a test that the locked panel holds secure input from the moment it is key, before a key is typed, lets go of it while the Touch ID dialog has key status and takes it again afterwards, and wipes the buffer when it loses key status any other way. They come with the panel, which wires the buffer and the flow in and tests them there.

## Limits

- The 30 seconds count from the first keystroke, not from the last.
- With nothing typed, the buffer does not expire and keeps secure input on for as long as the panel keeps it. That the panel lets go of secure input when the Touch ID dialog takes key status, takes it again afterwards, and wipes the buffer when it loses key status any other way is the panel's to do, and so is its test.
- From the moment the Touch ID dialog takes key status until the panel calls `rearm_secure_input`, secure input is off and the buffer takes no keys, neither `push` nor `backspace`. Keys typed then go to the dialog.
- After the unlock, the typed text becomes the panel's search, and from there it is held like any search typed into the open vault: as a plain string in the vault's state, not wiped on a timer.
- `backspace` leaves the removed character's bytes in the spare capacity until the buffer is wiped or dropped, which overwrites the whole allocation.
- That `wipe` overwrites the text is held by the list of methods called on it, not read back from memory.
- That `String::push` never has to grow the text is held by an assertion in `push` that only debug builds carry: the tests run it, a release build does not. The tests drive it at the edge of the first allocation.
- The panel's way in starts no second attempt while one is under way, and tests hold that. The shared flow does not refuse one for the unlock screen's origin; there the screen keeps it from starting, as before. Its Touch ID button, where it shows, says `Waiting…` and has no click handler while an attempt is in flight, whichever way in started it, and its automatic prompt is scheduled only while no attempt is in flight (`automatic_biometric_unlock_path`). That rests on this code; no test clicks the button while an attempt is under way.
- `kind` looks at the file system once, to see whether the vault's file is there. A file that can't be checked counts as needing the main window, which shows the real error.
- Opening a vault writes the list of recent vaults under `~/Library/Application Support/ferrispass`, and the shell reads its settings from there. Tests that open a vault or build the shell therefore run again in a child process of the test binary whose `HOME` and `TMPDIR` point at a temporary folder; the parent only checks that exactly that test passed there.
- A token only shows that `confirm` was asked with Return or the unlock button. That the panel asks it only from its handlers for that key and that click rests on the call-site list, not on the type.
- The scans in `tests/architecture.rs` read names, not values. A helper in another file that is handed the text, a call through a trait object that renames `retrieve`, a method called on the typed text through a dereference in parentheses, or a panel origin reached without its name is out of their sight. The compiler carries the first line: the token's private field, no `Clone`, the buffer not `Send`, `run` private to its module. Inside the module, who calls `run` and where a panel origin is made rests on the scan.
- That the Touch ID dialog leaves the panel without key focus was measured on one Mac with macOS 27.0.1. The tests only show the order, for every way the prompt can end: `on_answered` before the answer is applied and before Return counts, and with the password before the vault is opened.
- The tests run on GPUI's test executor, where a background task starts only once the foreground step that spawned it is over. `on_answered_runs_before_the_vault_is_opened` therefore fails for an open in the background task of the keychain read and for `on_answered` moved behind the open, but not for an open spawned inside the foreground step just before `on_answered` is called. That the open is spawned only after the step has handed the password out rests on the shape of `run`; no test sees it.
- `a_stale_password_never_goes_on_to_the_open` takes `on_done` having its answer in the same step as `on_answered` as the sign that no open started; it does not watch the open itself. The tests that hold the open run the executor one task at a time and rely on the same shape of `run`: the step that takes the answer spawns the open without running it.

The agents spent about 11 hours on this, including review and fixes.

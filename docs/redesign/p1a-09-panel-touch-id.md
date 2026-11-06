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
| the last one | The call-site list also holds every call of `confirm` and of the two ways into the flow |

## The typed-ahead buffer

`src/app/unlock/typed_ahead.rs`. The panel lets you start a search before Touch ID has unlocked the vault. What you type goes into a `TypedAhead`, and the panel only ever learns how many characters there are (`len`), to show as dots.

- The text sits in a `Zeroizing<String>`. `wipe` and the drop overwrite it, spare capacity included, and `take_query` hands it over in that same zeroizing string. Growing it never leaves a copy: when it is full, `push` copies it into a bigger zeroizing string and the old one is overwritten as it drops, instead of letting `String::push` reallocate and free the old memory as it was.
- `begin` takes a secure input guard (`secure_input::hold`), so other programs don't see the keystrokes, and the buffer holds it until it is wiped, taken or dropped. If macOS refuses secure input, or the call is off the main thread, `begin` fails and there is no buffer: the panel then sends you to the main window. Where the platform has no secure event input at all, the buffer starts without a guard and claims nothing.
- It is wiped 30 seconds after `begin` (`EXPIRY`), on cancel (`wipe`), and when it looks like a password rather than a search: more than one word and not a single match (`wipe_if_password_like`). `expire_if_due` and `wipe_if_password_like` wipe on their own and say whether they did; there is no check that only reports and leaves the wiping to the caller. The 30 seconds are wall-clock time, like the auto-lock's idle clock, so a Mac that slept through them finds the buffer expired, and a clock that went backwards counts as expired too.
- A wiped buffer stays empty, has let go of secure input and takes no more keystrokes. `take_query` hands the text over once, as the search to run after the unlock, and the buffer is gone with it.
- `Debug` prints `TypedAhead(<n> chars)`. Nothing in the file logs, prints or writes, and `typed_ahead_never_logs_or_writes` in `tests/architecture.rs` reads the file to keep it that way: no file system, log or print names, no derives, the field only in the functions listed in `TYPED_TEXT_USERS`, and never inside a macro call.
- The buffer is not `Send`, because the guard isn't: it lives and dies on the main thread.

A search never marks an entry as recently used; only copying a value or launching an entry does. `the_typed_query_never_reaches_last_used` takes a query out of a buffer, searches with it and then copies, and the list of recently used entries holds the entry and nothing else.

## One Touch ID flow, two ways in

`src/app/unlock/touch_id.rs` holds the Touch ID unlock that used to live in `start_biometric_unlock` in the shell. The steps are the same as before: `begin_biometric_unlock` stamps a new generation, the keychain read behind the prompt and the open run in the background, and on the foreground the answer goes through `begin_open` and `finish_open_attempt`, or `fail_biometric_unlock`. The prompt still reads "Unlock <file name>". The flow is the private `run`; nothing outside the module can call it, and no public function takes an origin or a trigger as a plain value.

| | Unlock screen | Quick access panel |
|---|---|---|
| Way in | `unlock_from_lock_screen`, from the Touch ID button, its shortcut, and on its own when the screen comes up with the window in front | `unlock_from_panel`, which takes a `Confirmed`. Only `confirm(PanelTrigger::ReturnKey)` and `confirm(PanelTrigger::UnlockButtonClick)` make one |
| macOS withdrew the prompt | the automatic prompt asks once more, as before; a click is left for the user to click again | `Answer::Interrupted`, nothing else. The next attempt needs a new confirmation |
| As soon as macOS answers, before the answer is applied | nothing | `on_answered` runs: the panel makes itself key and focuses the search field or the list |
| A cost still to confirm, or the header not read yet | the click counts as allowing the cost; the automatic prompt waits | no attempt starts |

The token has a private field, so no code outside the module can build one. It is neither `Clone` nor `Copy`, and `unlock_from_panel` takes it by value, so each attempt uses one up. Two doctests on `Confirmed` don't compile, on purpose: one builds a token by hand, one clones it.

Rules both ways share:

- An answer from an attempt that a newer one replaced opens nothing and ends the attempt in flight (`Answer::Stale`), as the unlock screen always did.
- The task is detached inside the module. Nobody holds a handle, so dropping what the call returns cannot cancel an unlock, and neither the shell nor the panel needs a field for it. The functions return the generation of the attempt they started, or `None`.
- `on_done` gets the answer once it is applied: `Opened(path)`, `NotOpened` (Touch ID answered, but the stored password no longer fits, the file is gone, or the cost has to be confirmed; the unlock screen says which), `Failed(error)`, `Interrupted` or `Stale`.

The unlock screen's own rule, one more automatic prompt after macOS withdrew it, stays in the shell: `start_biometric_unlock` passes an `on_done` that calls `note_automatic_biometric_interruption` for an interrupted automatic attempt. `schedule_automatic_biometric_unlock` stays `pub(super)`, and `the_automatic_prompt_has_no_public_entry` in `tests/architecture.rs` holds it and the three functions around it to no more than that.

`TOUCH_ID_CALL_SITES` in `tests/architecture.rs` lists every call outside tests of `begin_biometric_unlock` and `retrieve`, of the two ways into the flow and of `confirm`, one line per call with the function it is in: the two calls in `run`, the unlock screen's `start_biometric_unlock`, and the command line's `unlock_password`. Nothing calls `confirm` or `unlock_from_panel` yet. A token only says that `confirm` was asked with Return or the unlock button, not that the key was really pressed, so the panel's calls will come in as lines that name the key and click handlers they sit in; a timer or an activation handler on the list would stand out. `touch_id_starts_only_where_listed` fails on a call that is not on the list, on a line that has no call left, and on any of these names used without being called.

## When the panel takes Return, and what it offers while locked

`src/app/unlock/locked_panel.rs`.

`return_accepted` holds Return back until the results are drawn, 300 ms have passed since (`RETURN_SETTLE`), and every vault the panel searches has finished unlocking. Return then acts on what you saw, not on a row that arrived a moment later. One exact match in a vault that was already open takes Return as soon as it is drawn, unless it is a production SAP system (`exact_hit_takes_return_at_once`, which asks `EntryKind::is_production`). Return before anything is drawn is always dropped. For now the panel searches one vault, so "every vault" is that one.

`kind` decides what the locked panel offers:

| Case | Kind |
|---|---|
| No vault waits on the unlock screen and there is no recent one | `NotSetUp` |
| The vault's file is not there | `FileMissing(path)` |
| A cost to confirm, or the header check not back yet | `NeedsMainWindow` |
| No Touch ID enrolment for the vault | `NeedsMainWindow` |
| Enrolled, the sensor in reach | `TouchId { device_password: false }` |
| Enrolled, no sensor (a closed lid), the Mac password allowed | `TouchId { device_password: true }` |
| Enrolled, no sensor, the Mac password not allowed | `NeedsMainWindow` |

The vault is the one waiting on the unlock screen, or else the most recent one. Only the one on the unlock screen has had its header read, so for a recent vault `kind` answers `NeedsMainWindow` until the panel puts it there with `AppState::request_password` and the shell's header check comes back.

## What the panel will do with it

- Start a `TypedAhead` with the first keystroke while locked, show `len()` dots, call `expire_if_due` on every tick and keystroke and `wipe_if_password_like` after every search, and `wipe` on Esc.
- Ask `kind` what to show, and put the most recent vault on the unlock screen when none waits there.
- On Return or a click on the unlock button, and only then, call `confirm` and hand the token to `unlock_from_panel`. In `on_answered`, make the panel window key (`Window::activate_window`) and focus the search field or the list. The report on Touch ID from the panel in `docs/gpui-matrix.md` found the panel no longer key after the prompt in every run that showed one, and Return in the list did nothing until the panel was clicked.
- Check `return_accepted` for every Return in the results.
- After the unlock, run `take_query` as the search.

## What the tests show, and what needs a Mac

PR #28 has a table of changes that each break one of these rules, with the test that failed and its output.

| Rule | Test |
|---|---|
| The buffer holds secure input while it lives, and lets go when wiped or dropped | `typed_ahead_holds_secure_input_while_it_lives` |
| 29 seconds no, 30 yes | `typed_ahead_expires_after_thirty_seconds` |
| `expire_if_due` wipes and lets go of secure input | `expire_if_due_wipes_the_buffer_and_releases_the_guard` |
| A clock that went backwards counts as expired | `a_clock_that_went_backwards_counts_as_expired` |
| Several words without a match are wiped; one word, or matches, are not | `a_multi_word_query_without_a_hit_is_wiped_as_password_like` |
| `Debug` shows no character of the text | `typed_ahead_debug_shows_no_characters` |
| A refusal by macOS is an error, no buffer | `typed_ahead_refuses_when_macos_refuses_secure_input` |
| Without secure event input on the platform, the buffer still works | `typed_ahead_runs_without_a_guard_where_there_is_none` |
| The buffer can't leave the main thread | `typed_ahead_is_not_send` (doctest) |
| No log, print, file or derive in the buffer's file | `typed_ahead_never_logs_or_writes` |
| The unlock screen unlocks through the shared flow, one prompt, the setting for the Mac password passed on | `the_lock_screen_unlocks_through_the_shared_flow` |
| A withdrawn automatic prompt is asked again once, a withdrawn click is not | `an_interrupted_automatic_prompt_is_retried_once_on_the_lock_screen` |
| Dropping what the call returns doesn't cancel the unlock | `an_unlock_completes_without_anyone_holding_a_task` |
| A stale answer opens nothing | `a_stale_answer_opens_nothing` |
| A failed prompt is reported | `a_failed_retrieve_reports_the_error` |
| A search never marks an entry as recently used | `the_typed_query_never_reaches_last_used` |
| Only Return and the unlock button confirm | `confirm_is_a_closed_table` |
| Opening, typing, coming back to the front, a timer, an expired buffer or a withdrawn prompt never start Touch ID | `the_panel_starts_touch_id_only_with_a_token` |
| The raw flow has no public way in, and the token can't be made or copied elsewhere | `the_raw_flow_has_no_public_entry`, and the two doctests on `Confirmed` |
| A withdrawn prompt in the panel is not asked again | `an_interrupted_panel_prompt_is_not_retried` |
| `on_answered` runs before the answer is applied, and Return reaches the panel only after it | `the_panel_regains_key_before_it_takes_return` |
| The panel never answers a cost question | `the_panel_never_answers_a_cost_question` |
| Every locked case | `locked_panel_kind_for_every_case` |
| Return before drawing, within 300 ms, and before every vault is done is dropped; one exact match in an open vault goes at once unless it is production | `return_before_results_are_drawn_is_dropped`, `return_within_300ms_of_drawing_is_dropped`, `return_waits_for_every_vault_of_the_set`, `one_exact_hit_in_an_open_vault_takes_return_at_once_unless_production` |
| Touch ID starts only where listed; the automatic prompt stays inside the shell | `touch_id_starts_only_where_listed`, `the_automatic_prompt_has_no_public_entry` |

The characterization tests and `src/app/state/biometric_tests.rs` are unchanged and pass.

On a Mac, with a preview build, the unlock screen gets the same checks as before, since it now runs on the shared flow: Touch ID with the button and with Return unlocks; with the window sent to the background and brought back, the automatic prompt comes again, once; cancelling the prompt brings no second one without a click; and with a loop running `ioreg -l -w 0 | grep -o '"kCGSSessionSecureInputPID"=[0-9]*'` in a Terminal window behind FerrisPass, the PID shows while the master password field has focus and is gone after the unlock.

The panel's own checks need the panel: Return working after Touch ID without a click into the panel, focus going back to TextEdit, SAP GUI and Safari afterwards, and a panel tick that leaves out `expire_if_due` making a test fail. They come with the panel.

## Limits

- The 30 seconds count from the first keystroke, not from the last.
- After the unlock, the typed text becomes the panel's search, and from there it is held like any search typed into the open vault: as a plain string in the vault's state, not wiped on a timer.
- `backspace` leaves the removed character's bytes in the spare capacity until the buffer is wiped or dropped, which overwrites the whole allocation.
- A stale answer ends the attempt it finds in flight, as the unlock screen always did. Two attempts on top of each other therefore both end, and the user tries again. In practice the second prompt makes macOS withdraw the first, which then comes back as `Interrupted`, not as a stale success.
- `kind` looks at the file system once, to see whether the vault's file is there. A file that can't be checked counts as needing the main window, which shows the real error.
- Opening a vault writes the list of recent vaults under `~/Library/Application Support/ferrispass`, and the shell reads its settings from there. Tests that open a vault or build the shell therefore run again in a child process of the test binary whose `HOME` and `TMPDIR` point at a temporary folder; the parent only checks that exactly that test passed there.
- A token only shows that `confirm` was asked with Return or the unlock button. That the panel asks it only from its handlers for that key and that click is held by the call-site list and by review, not by the type.
- The scans in `tests/architecture.rs` read names, not values. A helper in another file that is handed the text, or a call through a trait object that renames `retrieve`, is out of their sight. The compiler carries the first line: the token's private field, no `Clone`, the buffer not `Send`, `run` private.
- That the Touch ID dialog leaves the panel without key focus was measured on one Mac with macOS 27.0.1. The test only shows the order: `on_answered` before the answer is applied and before Return counts.

The agents spent about 2 hours on this, including review and fixes.

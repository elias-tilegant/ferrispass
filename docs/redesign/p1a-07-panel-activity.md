# P1a.07: One idle clock for the auto-lock

The auto-lock now reads one clock for the whole app instead of a timestamp on the main window's view, and only input the user gives FerrisPass moves it. Today that is any key press, click or scroll in the window, or the pointer moving over it while it is in front. The quick access panel, the global shortcuts and the menu bar item get their calls with the steps that build them; the clock, the names for them and the list they go on are here.

Started from `p1a-08-secure-input` at `5031d12` on 2026-11-02.

| Commit | What |
|---|---|
| `d85c0c7` | `src/app/services/activity.rs`, the auto-lock and the window reading it, the list of call sites and the tests |
| `8a7b1cc` | SECURITY.md and this page |
| `506310e` | Shortcuts, clicks, scrolling and the pointer over a dialog count as input in the window |
| `4e21c3f` | This list and the time in the progress log |
| `d92f331` | A test of the clock as the app builds it, with a touch and a reset from a second thread |
| `a303725` | The unit test comments say what a new source needs |
| the last one | This page and the progress log on the two before it |

## What counts as activity

`ActivitySource` has one name for each kind of input that holds off the auto-lock:

| Source | Input | Called from |
|---|---|---|
| `WindowInput` | a key press, a click or a scroll in a FerrisPass window, or the pointer moving over one while it is in front | `src/ui/legacy/shell/render.rs`: a keystroke interceptor for the window and three pointer listeners on it |
| `Panel` | a key press or a click in the quick access panel | not yet; the panel doesn't exist |
| `Hotkey` | a global shortcut, at the moment it is pressed | not yet; the Auto-Type shortcut leaves the clock alone, as it did before |
| `StatusItem` | a click on the menu bar item | not yet; the menu bar item doesn't exist |

Nothing else moves the clock. Background work doesn't: sync, the TOTP tick, favicon downloads, the update check, launch cleanup, and the auto-lock check itself, which only reads it. Nothing from outside the process reaches it either: there is no function that turns an event in another app into a `touch`, and a global shortcut counts when it is pressed, not while it is registered.

The window's input used to be counted by two listeners on its root element, `on_key_down` and `on_mouse_move`, and three kinds of input got past them:

- GPUI hands a key to an element's key listener only if no key binding took it, so Cmd+C, Cmd+F, the arrow keys, Return and Escape never counted. A keystroke interceptor (`count_key_presses`) now sees every key in the window before bindings and text input do.
- A click without moving the pointer, and scrolling, had no listener at all.
- A dialog that blocks the pointer for the view below (`occlude`, as the About box and the entry editor do) hid the root element's listener, so pointer input in it didn't count. The pointer listeners now go on the window itself, registered in the paint of an empty canvas, and see every pointer event in the window.

GPUI also tracks the pointer over a window while another app is in front (`NSTrackingActiveAlways` in its macOS backend), so moving the mouse in another app across a visible FerrisPass window used to count as activity. A move now counts only when `window.is_window_active()`. Clicks and scrolling count either way: macOS sends a scroll to the window under the pointer, so scrolling there is scrolling FerrisPass, and a click brings the window to the front. Key presses only reach the window in front.

## Where the clock is read

Only the auto-lock task in `src/ui/legacy/shell/tasks.rs` reads it. When it starts, which is when a vault is in memory and the auto-lock is on, it calls `activity::reset(now)`, so a vault opened after a long pause is not locked on the first check. Every five seconds it asks `activity::idle_for(now)` and `activity::lock_is_due(idle, threshold)`, with the threshold read from the settings on each tick.

The clock is wall-clock time (`SystemTime`), as the field was. `Instant` stops while the Mac sleeps, and hours asleep would count as no idle time. A clock that jumps backwards makes `idle_for` return `Duration::MAX`, so the lock is due at once instead of later. Before the first `reset` the clock reads as the epoch, idle for decades, so a task that skipped it would lock rather than never.

The clock is a `static` behind a `Mutex`, written from the main thread in practice. `touch` needs no GPUI context, so a callback from AppKit can call it too. In unit tests each test thread has its own clock: libtest runs tests side by side, and the GPUI test executor runs a test's tasks on its thread, so one test typing into its window can't reset the clock another test is watching. That leaves the `static` out of every unit test, so `tests/idle_clock.rs` builds against the library the way the app does and checks it: a touch and a reset on one thread are read on another, as they have to be once the global shortcuts move the clock from the thread that listens for them.

## How the list of call sites is kept

`activity_is_touched_only_where_listed` in `tests/architecture.rs` reads every `.rs` file in the repository, with the tokenizer the `unsafe` check uses, so comments and strings don't count. For each `activity::touch(..)` outside items switched off by `#[cfg(test)]` it reads the source the call names and checks the pair of file and source against `ACTIVITY_CALL_SITES`. It fails on:

- a call from a file that is not on the list, or with a source the list doesn't give that file,
- a line on the list that no call matches any more,
- a call that doesn't name its source as `ActivitySource::<name>`, for example one that passes a variable along,
- `touch` imported or used as a value, a glob import from the module, or the module renamed with `as`, since calls through another name would go unseen.

Today the list has `src/ui/legacy/shell/render.rs` with `WindowInput`, and `tests/idle_clock.rs` with `Hotkey`: the scan reads tests as well, and that test touches the clock from a second thread. A step that adds a source adds its call and one line to the list, and the line is what review looks at.

`only_listed_sources_exist` in the module has a `match` over `ActivitySource` without a `_` arm, so a fifth source does not compile until it has an arm there. The arm does not put it in `ALL_SOURCES`, the list `touch_resets_idle` goes through. That is a second edit, which the comments in the test module ask for and nothing checks: Rust can't go through an enum's variants without naming them.

## Tests

Each rule was checked by taking it out; the changed lines and the failing output are in [PR #20](https://github.com/elias-tilegant/ferrispass/pull/20).

| Test | What it holds |
|---|---|
| `lock_is_due_at_threshold`, `lock_never_due_without_threshold` | the lock is due from the threshold on, and never with the auto-lock off |
| `lock_is_due_when_clock_goes_backwards` | a `now` before the last input reads as `Duration::MAX`, and the lock is due |
| `touch_resets_idle`, `only_listed_sources_exist` | every source moves the clock, and there are exactly four |
| `reading_the_clock_does_not_reset_it`, `reset_moves_the_baseline_both_ways`, `a_clock_never_set_reads_as_idle` | reads don't move it, `reset` sets it, an unset clock makes the lock due |
| `the_app_has_one_idle_clock_for_every_thread` (`tests/idle_clock.rs`) | the clock as the app builds it: an unset clock makes the lock due, `idle_for` reads what `reset` set, and a touch or a reset on another thread is read on this one |
| `key_press_in_the_window_resets_the_clock`, `shortcut_in_the_window_resets_the_clock` | a typed key and a key an action takes (Cmd+F) both move the clock (headless window) |
| `click_in_the_window_resets_the_clock`, `scrolling_in_the_window_resets_the_clock` | a click without a move, and a scroll, move it |
| `pointer_over_a_dialog_resets_the_clock` | the pointer over the About box, which blocks the pointer for the view below, moves it |
| `background_tick_does_not_reset_the_clock` | 30 seconds of TOTP ticks and auto-lock checks leave it where it was |
| `pointer_counts_only_while_the_window_is_in_front` | the pointer over the window moves it only while the window is active |
| `auto_lock_locks_once_the_clock_passes_the_threshold` | the task reads this clock: three idle minutes keep the vault open, four lock it |
| `auto_lock_starts_counting_when_the_vault_opens` | the task sets the baseline when it starts |
| `activity_is_touched_only_where_listed`, `activity_touches_are_found_with_their_source` | the list of call sites, and the scan itself on a sample with each finding |

`automatic_lock_discards_the_draft_and_says_so` and the other characterization tests are unchanged and pass.

## How to check by hand

On a preview build, with the auto-lock at its default of 4 minutes and a vault open:

- Leave the FerrisPass window in front and move the mouse over it now and then, without typing, for five minutes. The vault stays open.
- Work only with the keyboard for five minutes: Cmd+F, the arrow keys through the list, Cmd+C on an entry. The vault stays open.
- Put the FerrisPass window next to another app, click into the other app and work there for five minutes, moving the mouse across the FerrisPass window a few times. The vault locks within five seconds of the four minutes being up.
- Leave the Mac alone with FerrisPass in front for five minutes. The vault locks.

## Limits

- The quick access panel, the menu bar item and the global shortcuts don't call `touch` yet, and SECURITY.md already names them: the row in the security contract applies from the redesign release, which ships with them. Until then the Auto-Type shortcut does not hold off the auto-lock, as before.
- The scan sees the calls themselves, not who calls the function around them. A function in a listed file that wraps `touch` can be called from anywhere, and code that another crate's macro generates is out of its reach; review of the listed files covers both.
- The shell tests build the real `AppShell` and read the real `settings.json`. They set the auto-lock themselves and use a fake pasteboard.

# P1a.08: Secure event input

While the master password field on the unlock screen has focus, FerrisPass turns on macOS secure event input. Programs that watch the keyboard, through an event tap or the older Carbon hooks, stop seeing what is typed until the field loses focus. Nothing changes on screen. If macOS refuses, a notice says the field is not protected.

Started from `p1a-06-clipboard-path` at `5da7f72` on 2026-11-01.

| Commit | What |
|---|---|
| `38a0aff` | The guard in `src/platform/secure_input.rs`, its tests, and the first entry in the list of modules allowed to use `unsafe` |
| `d58da6b` | The master password field, the window, lock, unlock and quit use the guard |
| `e016fc5` | A Disable that fails is tried again later, and no second Enable is stacked on top of it |
| `0a989da` | The test with the real Carbon calls runs on a real main thread, and the calls check for it |
| the last one | SECURITY.md and this page |

## How the guard works

Secure event input is one count for the whole login session, shared by every app. Each `EnableSecureEventInput` adds one, each `DisableSecureEventInput` takes one away, and while the count is above zero no other program gets keyboard events. Apple's header for these functions asks apps to turn it off when they become inactive. An Enable that never gets its Disable keeps it on with another app in front, and that app's shortcuts and text expansion stop working until FerrisPass quits.

`secure_input::hold(cx)` returns a `SecureInputGuard`. As long as one guard is alive, secure input is on. The module keeps its own count of live guards and calls Carbon only on the edges: Enable when the count goes from 0 to 1, Disable when it goes from 1 to 0. `release_all` turns it off for every guard at once, sets the count to zero and starts a new generation; a guard from an earlier generation does nothing when it is dropped later, so it can't switch off a hold taken after the release. If a Disable fails, its Enable stays outstanding: the next hold uses it instead of adding another, and the next release tries the Disable again. All of this sits under one lock.

Carbon is only called on the main thread. The header marks the three functions "Not thread safe". `hold` returns an error on any other thread, `release_all` does nothing there, a guard is not `Send`, so it is dropped on the thread that took it, and in debug builds each Carbon call asserts that it runs on the main thread. A refused `hold` holds nothing; the caller gets the error and the OSStatus goes to the log. On platforms other than macOS `hold` returns `Unsupported`, and the app writes one line to the log at launch instead of claiming protection.

This is the first module under `src/platform/`, the one place new `unsafe` code may go. Its `unsafe` is the `extern` block for the three Carbon functions and the three calls; everything else in the file is safe Rust. `tests/architecture.rs` lists it, and SECURITY.md has a new section "Unsafe code" that lists it with the seven older places.

## When it is released

Each test below was checked by taking out the rule it names; the changed lines and the failing output are in [PR #19](https://github.com/elias-tilegant/ferrispass/pull/19).

| When | What happens | Held by |
|---|---|---|
| The field loses focus (Tab, a click elsewhere) | the field's guard is dropped | `password_focus_holds_and_blur_releases` |
| Another app comes to the front, or Cmd+W hides the window | the window's activation handler drops the guard at once, without waiting for a redraw; coming back with the field still focused takes a new one | `window_deactivation_releases_and_activation_holds_again` |
| Lock: the button, Cmd+L, the idle timer, the screen lock | every guard is released, not only the field's | `lock_releases_secure_input` |
| A successful unlock | the unlock screen goes away, and the field with it | `unlock_leaves_nothing_held` |
| Quit with Cmd+Q or the menu, or restart into an update | released before macOS is asked to quit | `quitting_releases_secure_input` |
| Any other quit | released in the app's quit handler | `quit_hook_releases` |
| A panic on the main thread | released in the panic hook, before the previous hook prints | `panic_hook_releases_on_the_main_thread` |
| A panic on another thread | nothing changes: the guards live on the main thread, which still releases them | `panic_hook_off_the_main_thread_touches_nothing` |
| The process is killed | macOS drops the hold with the process | checked by hand, see below |

The counting rules have tests of their own in `src/platform/secure_input.rs`: three guards make one Enable, the first of two drops leaves it on, a second `release_all` does nothing, a guard dropped after `release_all` doesn't touch the new hold, a drop on another thread during `release_all` waits for it, a second release of the same guard can't push the count below zero, a failed Disable is retried and never gets a second Enable on top, and a hold off the main thread is refused.

## What was measured

The spike on 2026-10-17 ran the same guard shape in a GPUI window on a Mac: an event tap saw none of the keystrokes typed into the password field, a global hotkey still arrived while secure input was on, 999 focus changes made 999 Enables and 999 Disables, and lock, quit and a panic each released it.

`carbon_calls_balance` makes the real Carbon calls in every test run: after `hold` the system reports secure input on, after the drop it is back where it was. libtest runs unit tests on worker threads, so this one is a doctest that rustdoc builds as a program of its own; it runs from `main`, on a real main thread.

## Checked by hand

One case no test can show: the process killed while the field has focus. Apple's header says that if the app crashes, secure event input is turned off unless another app has it on, and Technical Note TN2150 says the same for a process that ends. On a preview build, with the field focused, `ioreg -l -w 0 | grep -o '"kCGSSessionSecureInputPID"=[0-9]*'` shows FerrisPass's PID; after `kill -9` it shows nothing. The same command shows nothing after a click into another app, after unlocking and after Cmd+Q, and the Auto-Type shortcut still reaches FerrisPass while the field has focus.

## Limits

- GPUI also reports the focus leaving the field when the window stops being active, and entering it again when the window comes back, but only with the next frame. The activation handler doesn't wait for that, because a hidden window may not draw one. Coming back to the window, the field is therefore held twice in a row, by the handler and then by the focus report; the second guard replaces the first, so it is still one Enable. If macOS refuses secure input at that moment, the notice appears twice.
- The refusal notice is checked by count, not by text: a headless window can't give the text back. The text is the constant `SECURE_INPUT_REFUSED` in `src/ui/legacy/shell/unlock.rs`.
- Only the master password field on the unlock screen is covered. The password fields in the entry editor are masked but don't hold secure input.
- On platforms other than macOS there is no secure event input. CI only builds for macOS, so the no-op is compiled there only for its test.
- `carbon_calls_balance` compares with the state before the test. If another app holds secure input during the run (Terminal's "Secure Keyboard Entry", for example), it shows less.
- If a Disable fails, secure input stays on until the next release, which may only come at lock or quit. That is the price of never stacking a second Enable.
- The shell tests build the real `AppShell` and read the real `settings.json`, and `quitting_releases_secure_input` empties the per-user launch folder, as quitting does. No test touches the general pasteboard.

# P1a.10: The quick access panel

Control-Option-Command-P opens a panel over whatever app is in front, without making FerrisPass the active app. It searches the open vault, says in its footer what Return does, runs that action, copies through the clipboard service, and gives the keyboard back to the app behind it when it closes. While the vaults are locked, what you type waits under secure input for Touch ID. It opens over every state of the app, the hidden one included, shows the selected entry next to the list, and lists its commands in a menu.

The open and the locked panel came with [PR #31](https://github.com/elias-tilegant/ferrispass/pull/31), merged as `19d13f4`; the rest with [PR #44](https://github.com/elias-tilegant/ferrispass/pull/44):

| Commit | What |
|---|---|
| `478abda` | The panel over a hidden app, `src/platform/activation.rs` |
| `b964014` | One rule for a short SAP client, for the launch, the rows, the search and the footer |
| `ccc8a07` | The preview, a search without a match, the error line, a sign-in SAP GUI refuses |
| `d884e28` | The footer's menu |
| `4722b0c` | The main window back with the app after the panel closed over a hidden FerrisPass |

## Opening and closing

The shortcut, Control-Option-Command-P unless switched off in the Auto-Type settings, and Quick Access in the Window menu open the panel. One listener carries both global shortcuts, quick access and Auto-Type, on one `GlobalHotKeyManager` (`src/app/services/hotkeys.rs`). The panel is a `PopUp` window, which macOS makes a non-activating panel: it takes the keyboard while the app the user works in stays in front. It is 680 points wide and at most 460 high, in the upper part of the main display, not movable.

It closes on Escape (the first Escape clears a search that holds text), on Command-W, on the shortcut, on a click into another app, after an action that hands over to another app, and when the vault is locked. Closing removes the window and activates nothing, so the app behind gets its window and its text field back. Command-W and Escape close the panel and nothing else, and Command-M does nothing while the panel is in front: the vault stays open and the main window stays as it was. The handlers that act on a window, Close Window, Minimize, Quit's window geometry and the lifecycle notices, act on the main window and never on the key window. The search and its selection stay for two minutes after the panel closes, in the model (`src/ui/vm/panel.rs`); the window is built anew each time.

## Over every state of the app

Visible, minimized or without a main window, the panel opens and closes as above. Over a hidden FerrisPass, after Command-H or after Command-W in the main window hid it, macOS would bring the main window back with the panel. So, only when the app is hidden as the panel opens, the panel unhides it without activating it (`unhideWithoutActivation`) and then orders out right away the windows that brings back, which is the main window. Once the panel's window is gone, it puts those windows back and hides the app again right after: an unhide brings back only the windows that were on the screen when the app was hidden, so this way a click in the Dock or Command-Tab brings FerrisPass back with its main window, and so does Command-O in the next panel over the hidden app. When it closes because you asked for the main window, with Command-O, Unlock, New or the menu's Settings, the app stays unhidden and the windows come back in front. The AppKit calls are in `src/platform/activation.rs`; all of them are safe calls in objc2-app-kit, each made only on the main thread, and the file forbids `unsafe`.

## Search and the rows

Typing ranks the open vault the way the main window's search does, at most nine rows, one for each of Command-1 to Command-9. An empty search lists favorites, then recently used entries, then the rest by title. A new query selects the first row; Up, Down and Command-1 to Command-9 pick one. Each row is named after its entry: the kind and the title, the role in words ("Production system", never the letters on the chip), the client, the user and the vault. A row that stands for the same system as others (the same SID and client) has the count in the footer. The vault's name comes from its file and is cleaned of control characters and direction marks. The panel shows an entry without its notes and its protected custom fields.

An SAP client in `SAP_CLIENT` shorter than three digits reads as the launch writes it: 1 is 001, in the row, in the exact identifier "NHP 001", for the twins count and in the footer.

## What Return does

Return runs the selected entry's main action: an SAP system signs in through SAP GUI, a web login opens its website (http and https only), a remote access entry copies its password. Signing in and opening a website close the panel; a copy keeps it open. From an empty search Return never acts on a production system the user has not picked with Down or Command-1 to Command-9; the footer says so in ochre instead. Return counts only for results that have been on screen for 300 ms and while no vault is still being unlocked, except for one exact match in a vault that was open before the search, which takes Return at once unless it is a production system. Command-O closes the panel and shows the entry in the main window.

When SAP GUI cannot be started for the entry, the panel stays open and says so in the error line; a password SAP GUI's launch file cannot carry, with `&`, `=` or a line break, gets a sentence of its own that does not name the character. The entry then does not count as used.

## Copies

Command-C copies the password, Shift-Command-C the user name, Option-Command-C the one-time code, also from the search field with text marked in it. Every copy, from a key, the preview or the menu, goes through `ClipboardService::write_secret` with the clear time from the settings; the panel has no other way to the pasteboard, and the search field's own Copy and Cut write nothing. A copy keeps the panel open, the footer says what was copied, and a copied password or user name counts as using the entry. Every action acts on the entry the panel selected, by its id, never on the main window's selection.

## The preview

Once something is typed, or a row is picked, the selected entry shows next to the list: its title and vault, the user name, shortened in the middle when it is long, the password as twelve dots in a secret field, and the one-time code as "••• •••" in its ring, each with a copy button. The panel never reveals the password and never unmasks the code; it has no button for that. The ring is drawn again ten times a second while it shows, and not at all otherwise. The password and the code are read by the selected entry's id while the panel draws and kept nowhere.

Whether VoiceOver runs is read from macOS when the panel opens and each time it becomes key, and passed to the secret field and the ring as it is. Both put their secret into the accessibility tree only while it is shown and VoiceOver runs, which in the panel is never, so no app that reads the tree through accessibility access gets the password or the code.

## No match, the error line and the menu

A search without a match says "No match in" and the vault's name, with a New button and Command-N, which close the panel and start a new entry in the main window. Unlock, which opens the main window, shows while another vault is unlocked besides the open one.

What failed, a website that did not open, a password that could not be copied or SAP GUI that did not start, stands in one carmine line above the search, an alert, until the next key. Its text is a fixed sentence and names no entry, user or host. The footer keeps the confirmations of copies.

The menu button at the end of the footer opens a popover with the commands the command table offers in the panel and that apply now, each with its keys and its word from the table: the entry's main action, its copies, the address, Show in FerrisPass, Lock everything, Settings and Close. While Return waits for a production system, the main action is not in it; locked, only Settings and Close are. A click runs the command, Escape closes the menu first, and the search field keeps the keyboard.

## Locked

While no vault is open, the panel is the locked panel. With Touch ID set up, what you type goes into a buffer and shows as dots, and the footer says "Search saved". Secure input is on from the moment the locked panel is the key window, before the first key, and never while another app is still in front; it is off only while the Touch ID dialog has the keyboard. Touch ID starts only from Return or a click on "Unlock with Touch ID", never on its own. After the answer the panel makes itself key again before it takes the next Return. Once the vault is open, what was typed becomes the search, unless 30 seconds have passed since its first key or it is several words without a match, which looks like a password typed into the wrong window. The buffer is wiped after 30 seconds, on Escape, on a click elsewhere and when the panel closes, and never lands in the recently used entries. The locked panel shows nothing from a vault. Without a usable Touch ID it says what is missing: "Needs the main window" with Command-O, "Unlock with Mac password" without a sensor in reach, "FerrisPass isn't set up yet." with a button, the vault file's name when the file is gone, or that the vault is still being unlocked. The unlock screen's own automatic prompt waits while the panel's attempt runs.

## Deviations

- An empty search shows favorites, recently used entries and the rest as one list, without headers between them.
- The footer's menu has no filter and no way into the actions of a single entry; it lists the commands.
- Command-C in the search field copies the password even with text marked in the field. There is nothing in the search worth more than the password.
- The rule that drops several words without a match runs when the typed text is handed to the search, after Touch ID, since the locked panel searches nothing before.
- The preview sets no `sharingType`; whether that keeps a window out of screen recordings is still to be tried.
- The main window is kept off the screen over a hidden app by ordering out the windows `NSApplication` lists, not through GPUI's window handle, which FerrisPass cannot reach without a new dependency; so `activation.rs` needs no `unsafe`.
- What failed moved from the footer's second line to the error line above the search.
- Unlock, in a search without a match, opens the main window, where other vaults are switched to; it unlocks nothing itself.

## What the tests show

The panel's tests open the main window over a test vault the way the app does and then the panel as a second window, and drive it through its keys, actions and clicks (`src/ui/windows/panel/tests.rs`, `tests/apps.rs` next to it, `locked/tests.rs`). They read the accessibility node of each row, each part of the secret field, the ring, the preview's copy buttons, the footer and the search group, and the copies a fake pasteboard got. They show the rankings and the production rule, Return's gate, every copy going through the clipboard service with its clear time and the main window's selection staying untouched, the window handlers acting on the main window only, the two minutes, the ids `fp.panel.row.0` to `fp.panel.row.8` with no title, user or host in them, the model's `Debug` output naming no entry, and the notes and protected fields staying out.

For the hidden app, a fake of `activation` notes the calls in their order and keeps AppKit's rule that an unhide brings back only the windows that were on the screen at the hide. Over a visible app there is no call. Over a hidden one, opening unhides the app and then orders the main window out; closing, for each way the panel closes, puts the main window back and then hides the app, once GPUI lists the panel's window no more, with no window on the screen while the app is hidden; an unhide after that, as from the Dock, shows the main window again, and so does Command-O in the next panel over the hidden app; closing for the main window brings the windows back and hides nothing. For the preview, the secret "sp-pw-7Qx" and the current one-time code, as they are, in threes and in pairs, are in neither the title nor the user line, and in no node of the secret field, the ring or the two copy buttons, with and without VoiceOver. The copy buttons are read as the design system hands them to GPUI: each is a button named for what it copies, and the texts of its node and the whole node are checked. In the source, the panel builds two copy buttons, both in `preview`, each with a literal id and a name from `tr!`, and no function or `impl` block in the panel's files whose signature names the button or a type there that holds it has a description, an accessibility id, a tooltip or an `aria_` property in its body; the flags the panel hands the secret field and the ring follow the VoiceOver answer at opening and on becoming key, never reveal and never unmask; and a click on the password copies the panel's entry, not the main window's.

For the locked panel, a fake of secure input and an in-memory Touch ID store show secure input from the first key moment and never while the panel is open but not key, its release while the dialog is key and its return after, the buffer wiped on every path and replaced while the panel stays key and locked, Touch ID only from Return and the button, and the typed text reaching the search only after the vault opened.

Source scans hold what a runtime test cannot reach: no `cx.activate` or `show_main_window` outside Command-O's handler, no pasteboard call but `write_secret`, no log line, one alert and no other live region, no handler reading the key window, `unhideWithoutActivation` and `orderOut` only in `activation.rs`, no `.revealed(` and no `.masked(false)` in the panel, the preview's copy buttons as just described, and every menu row the command table's pair. `tests/architecture.rs` lists every call of `activity::touch`, `confirm` and `unlock_from_panel` with its function.

The characterization tests and `src/i18n/tests.rs` are unchanged and pass.

## What was checked on a Mac

The open and the locked panel were tried with a preview build before they were merged; the list is in PR #31. Without VoiceOver, Control-Option-Command-P opened the panel; with VoiceOver running it did not (Limits). The checks for the hidden app, the preview, a search without a match, the error line and the menu, and the 50 alternating presses of both shortcuts, are listed in PR #44 for a run by hand with the preview build.

## Limits

- The test platform knows no window kinds, never activates an app, cannot hide one and makes a window key only when a test asks it to. That the panel is a non-activating panel over another app, that the app behind keeps the front and gets the keyboard back, that macOS takes both shortcuts on one manager, and that the main window stays off the screen over a hidden app need a run on a Mac.
- The tests hold the hidden app against a fake that keeps AppKit's rule for hiding: an unhide brings back the windows that were on the screen when the app was hidden, and no other. That AppKit keeps it, that a click in the Dock and Command-Tab bring FerrisPass back with its main window after the panel closed over it, and that the main window does not show for a moment while it is put back right before the hide, need a run on a Mac.
- `activation.rs` uses objc2-app-kit's `NSApplication` and `NSWindow`, whose features GPUI and the GPUI Kit turn on for their own use of the crate; `Cargo.toml` lists only the features FerrisPass used before.
- VoiceOver is not part of this step. The tests keep secrets and codes out of the accessibility tree, which apps with accessibility access read too (the preview's copy buttons as well, see below), but nothing here is tuned for VoiceOver. With VoiceOver running, Control-Option-Command-P and Control-Option-Command-V very likely never reach FerrisPass, since Control-Option is the VoiceOver modifier; whoever needs both can set the VoiceOver modifier to Caps Lock in the VoiceOver settings.
- `global-hotkey` keeps its event sender private, so the tests put events on a per-thread stand-in; the drain of the real channel is not exercised by a test.
- The panel opens on the main display, not on the display with the pointer, and is 460 points high whatever it shows.
- Vaults carry no kind yet, so every row shows the badge of the user's own vault; rows name no customer yet. A click on a row does not select it.
- While the panel is open, the Auto-Type shortcut does nothing, and nothing says why.
- A failed copy of the user name or the one-time code says nothing; a failed password copy does.
- A website fails in the panel only when its address is refused: GPUI's `open_url` reports nothing back. Every launch error but a refused password gives the same sentence.
- The menu shows each command's keys from the command table. In the panel, Shift-Command-L, Option-Return, Command-L and Command-comma do nothing yet; a click runs those four.
- The ring gets the time left in whole seconds, so it moves once a second. The one-time code comes from the state as a plain `String`, which is not overwritten when dropped; the panel moves it into a `Zeroizing` string at once. The user name is shortened by characters, not by width.
- The flags test sees what the panel hands the secret field and the ring, not what they keep; a fixed value at the call is caught by the same test's source part.
- The preview's two copy buttons are read at run time as `preview` builds them, through a test-only way the design system opens to its buttons (`IconButton::into_base`). What happens to them after `preview`, on their way to the window, only the source scan holds: in `src/ui/windows` and `src/ui/vm`, where they are built, every function and `impl` block whose signature names the button, a name it is brought in under or a type there that holds it, and `render_open`, which hands the parts on unchanged. A macro, or a function, trait implementation or alias outside those two folders, that changes the buttons after `preview` gets past both.
- The test platform never shows a Touch ID dialog and never moves the keyboard on its own; the tests let another window take key status in the dialog's place. That macOS gives the panel key status back after the dialog and that `ioreg` shows the secure input PID while the locked panel is key need a run on a Mac.
- The panel keeps its own copies of the search's rule for an exact match and of the launcher's rule for the user it signs in as; `names_exactly_as_the_search_does` compares the first over a set of entries, nothing compares the second.
- The locked panel's field takes printable characters and Backspace, nothing else. When macOS refuses secure input, the panel says it needs the main window until it is opened again.
- Opening the panel on the welcome screen puts the most recent vault on the unlock screen, which stays there when the panel closes without unlocking; no test holds that.
- The source scans read text: a call made through a macro, a function in another file or a name brought in under another name gets past them.

## Mutation probes

Each rule was checked by taking it out on a clean tree and running the tests named; the changed lines and the red output are in PR #31 for the open and locked panel and in PR #44 for the rest. Every probe turned its tests red, except where the table says otherwise.

| # | Test | What the probe changes |
|---|---|---|
| 1 | `drain_pressed_hands_both_shortcuts_on` | `drain_pressed` hands on only the first press (`.take(1)`) |
| 2 | `cmd_w_and_escape_close_only_the_panel`, `no_handler_reads_the_active_window` | `close_main_window` takes `cx.active_window()` instead of the main window |
| 3 | `cmd_w_and_escape_close_only_the_panel` | `ClosePanel` calls `request_window_close` instead of closing the panel |
| 4 | `copies_go_through_the_clipboard_service_and_keep_the_panel_open`, `the_panel_never_touches_the_pasteboard` | the copy writes with `cx.write_to_clipboard` instead of `write_secret` |
| 5 | `a_row_is_named_after_the_pattern_and_never_by_the_letters_prd` | the role word taken from the chip's letters |
| 6 | `an_empty_search_never_lets_return_act_on_a_production_system` | the footer names a command for Return while it waits for a production system to be picked |
| 7 | `the_quick_access_hotkey_counts_as_activity`, `activity_is_touched_only_where_listed` | `activity::touch(Hotkey)` removed from the loop |
| 7 | `keys_and_clicks_in_the_panel_count_as_activity` | `activity::touch(Panel)` removed from the panel's key interceptor |
| 8 | `an_accessibility_id_takes_literal_names_only`, `panel_ids_carry_no_title_user_or_host` | the row's id built from the row's name, leaked to a `&'static str` |
| 8 | `raw_accessibility_ids_are_applied_in_one_file` | GPUI's `accessibility_id` with a text, on the locked panel's sentence |
| 9 | `search_and_selection_are_forgotten_after_two_minutes` | `MEMORY` ten minutes |
| 10 | `the_panel_never_activates_the_app` | `cx.activate(true)` when the panel opens |
| 11 | `only_http_and_https_open` | `web_url` lets any other address through |
| 12 | `one_manager_carries_both_registrations` | `HotkeyRegistry::register` makes a new manager for each shortcut |
| 13 | `arrow_keys_and_cmd_c_reach_the_panel_while_the_search_field_is_focused` | Up, Down, Cmd+C and Cmd+1 to Cmd+9 bound in `Panel` instead of `Panel > Input` (stays green: a single-line Kit field has no handler for Up and Down, and its own Copy is caught at the panel's root) |
| 13b | same | 13, and the root no longer catches the field's Copy |
| 14 | `panel_actions_use_the_panel_selection_not_the_main_windows` | Cmd+C copies through `copy_selected_value(Password)`, the main window's selection |
| 15 | `an_empty_search_lists_favourites_then_recent_then_the_rest` | the empty search ranked through `rank` |
| 15 | same | a favorite used recently listed among the recent as well |
| 16 | `quit_and_notifications_from_the_panel_go_to_the_main_window`, `no_handler_reads_the_active_window` | Quit stores the geometry of `cx.active_window()` |
| 17 | `the_search_field_never_writes_its_text_to_the_pasteboard` | the root no longer catches the field's Cut |
| 18 | `the_panel_model_debug_names_no_entry` | the model's `Debug` prints the query |
| 19 | `copies_go_through_the_clipboard_service_and_keep_the_panel_open`, `a_copy_is_cleared_after_the_time_the_settings_give` | the panel copies with no clear time (`None`) |
| 20 | `what_the_panel_says_stays_in_its_footer`, `the_panel_builds_no_notification_and_no_live_region` | a refused website also puts up a notification in the panel |
| 21 | `the_panel_builds_no_notification_and_no_live_region` | the locked panel's sentence gets an id and `Role::Status` |
| 22 | `the_auto_type_hotkey_types_nothing_while_the_panel_is_open` | the loop no longer holds Auto-Type back while the panel is open |
| 23 | `rows_for_the_same_system_are_counted_in_the_footer` | the footer counts twins only above nine, which never happens |
| 24 | `reconcile_registers_only_what_the_settings_ask_for`, `one_manager_carries_both_registrations` | `HotkeyLoop::unregister` keeps the id |
| 25 | `an_empty_search_never_lets_return_act_on_a_production_system` | Return runs `registry::primary_command` directly and skips the footer's rule |
| 26 | `keys_and_clicks_in_the_panel_count_as_activity` | `activity::touch` removed from the mouse-down listener |
| 27 | `the_selected_row_carries_the_footer_text_and_the_empty_group_carries_it_too` | every row carries the footer sentence as its description |
| 28 | `locking_closes_the_panel_and_empties_the_model` | locking no longer forgets the search |
| 29 | `clicking_elsewhere_closes_the_panel` | the panel stays open when it loses the keyboard |
| 30 | `the_panel_model_debug_names_no_entry` | `Hit`'s `Debug` prints the kind |
| 31 | `the_panel_opens_as_a_popup_that_takes_focus` | the window is `WindowKind::Normal` |
| 32 | `quit_and_notifications_from_the_panel_go_to_the_main_window`, `no_handler_reads_the_active_window` | the lifecycle notice goes to `cx.active_window()` |
| 33 | `a_copy_is_cleared_after_the_time_the_settings_give` | the clear time taken from `AppSettings::default()` instead of the saved settings |
| 34 | `what_the_panel_says_stays_in_its_footer` | a helper in `src/app/actions.rs`, outside the folders the scan reads, puts up a notification in the main window after a copy |
| 35 | `return_and_the_button_are_the_only_ways_to_touch_id`, `touch_id_starts_only_where_listed` | the activation observer asks `confirm(PanelTrigger::ReturnKey)` and starts the unlock |
| 36 | same | the activation observer asks `confirm(PanelTrigger::Activated)` and starts the unlock with what it gets |
| 37 | `the_tick_calls_expire_if_due` | the tick no longer calls `expire_if_due` |
| 38 | `activation_takes_secure_input_once_before_the_first_key` | the buffer begins with the first key instead of when the panel becomes key |
| 39 | `an_open_but_inactive_locked_panel_holds_no_secure_input` | `PanelView::new` begins the buffer, and `ensure_buffer` no longer asks `is_window_active` |
| 40 | `a_wiped_buffer_is_replaced_while_the_panel_is_key_and_locked` | the tick wipes an expired buffer and begins no new one |
| 41 | `secure_input_is_released_while_the_dialog_is_key_and_taken_again_after` | losing key status to the dialog no longer lets go of secure input |
| 42 | `the_typed_text_becomes_the_search_only_after_opened` | `on_done` hands the text over on `Failed` too |
| 43 | `a_password_like_buffer_never_becomes_the_search` | the `is_password_like` check after `take_query` skipped |
| 44 | `the_panel_regains_key_and_focus_before_it_takes_return` | `take_key` notes the call but no longer calls `activate_window` |
| 45 | `return_waits_while_an_unlock_is_under_way` | Return and the button go for Touch ID while `kind` says `Unlocking` |
| 46 | `return_in_the_open_panel_goes_through_the_gate` | `on_primary` skips `return_accepted` |
| 47 | `the_automatic_prompt_yields_to_a_panel_attempt` | `start_automatic_biometric_unlock` no longer asks `unlock_under_way` |
| 48 | `typed_keys_show_as_dots_and_never_as_a_value`, `only_build_names_what_the_locked_panel_draws` | the field gets `aria_value` with its dots in `locked_parts`, on its way to the window |
| 49 | `closing_the_locked_panel_wipes_the_buffer` | neither way of closing wipes the buffer |
| 50 | `the_locked_panel_shows_nothing_from_a_vault` | locking keeps the model's rows (`forget` keeps `results`), and `refresh` keeps them while no vault is open |
| 51 | `the_countdown_types_nothing_when_the_panel_opened_meanwhile` | `perform_auto_type` no longer looks at the panel |
| 52 | `opening_the_panel_cancels_a_running_auto_type` | `panel::open` no longer rotates the Auto-Type context |
| 53 | `the_panel_shows_no_notes_and_no_protected_fields`, then named `the_panel_reads_no_notes_and_no_protected_fields` | `shown_fields` keeps the protected custom fields |
| 54 | `a_switched_off_shortcut_does_nothing_with_its_old_id` | the loop takes the quick access id from `QUICK_ACCESS_HOTKEY` instead of from what is registered |
| 55 | `close_window_and_quit_reach_the_main_window_from_inside_it` | Close Window and Quit run at once again, not deferred |
| 56 | `minimize_reaches_the_main_window_from_inside_it` | Minimize runs at once again |
| 57 | `a_click_outside_while_the_vault_opens_wipes_and_closes` | losing key status keeps the panel whenever `kind` says `Unlocking`, not only while its own prompt is up |
| 58 | `the_panel_key_again_during_the_prompt_takes_secure_input_back` | becoming key again no longer takes secure input back |
| 59 | `a_wiped_buffer_is_replaced_while_the_panel_is_key_and_locked` | no buffer begins while `kind` says `Unlocking` |
| 60 | `names_exactly_as_the_search_does` | the panel's copy of the rule takes any word after the client as a role |
| 61 | `a_key_calls_expire_if_due` | the key handler no longer calls `expire_if_due` |
| 62 | `a_vault_opened_another_way_gets_nothing_of_what_was_typed` | `locked_state_changed` no longer wipes the buffer when the vault opens |
| 63 | `only_build_names_what_the_locked_panel_draws` | `assemble` gives the body an id, `Role::Group` and a name |
| 64 | `typed_keys_show_as_dots_and_never_as_a_value`, `only_the_dots_show_how_much_was_typed` | `build` draws the dots as a text with an id (`Text::new`), so they get a node of their own |
| 65 | `only_the_dots_show_how_much_was_typed` | `build` gives the unlock button the dots as its description |
| 66 | `an_interrupted_prompt_is_not_asked_again_from_the_panel`, `return_and_the_button_are_the_only_ways_to_touch_id`, `the_typed_text_becomes_the_search_only_after_opened` | `on_done` calls `on_return` after `Answer::Interrupted` |
| 67 | none: `cargo check --locked --all-targets` | `on_activation` calls `start_unlock` with a token it makes itself, `Confirmed(())` (does not compile: the token's constructor is private) |
| 68 | `a_production_role_in_a_protected_field_counts_in_the_panel`, `a_production_system_the_panel_shows_as_a_web_login_waits_too` | `Hit::new` works out whether the entry is a production system from the shown fields only |
| 69 | `the_footer_names_client_and_user_only_as_signing_in_uses_them`, `the_panel_shows_no_notes_and_no_protected_fields` | the footer names the client and the user without comparing them with what signing in uses |
| 70 | `a_combo_auto_type_lets_go_of_goes_to_quick_access_in_the_same_save` | `reconcile` registers without letting go of changed shortcuts first |
| 71 | `the_header_names_the_vault_and_the_footer_mentions_parked_vaults` | the search field labelled `tr!("a11y.search")` instead of with the vault (`view.rs:504`) |
| 72 | `only_build_names_what_the_locked_panel_draws` | `locked_parts` gives the unlock button the dots as its shortcut, which the design system hands to `aria_keyshortcuts` |
| 73 | `a_protected_role_never_takes_production_away_in_the_panel` | `Hit::new` works out whether the entry is a production system from the whole entry only |
| 74 | `the_open_panel_cleans_the_vault_name_as_the_locked_panel_does`, `the_file_name_in_file_missing_is_a_basename_through_display_untrusted` | `turns_direction` without U+061C |
| 75 | `only_build_names_what_the_locked_panel_draws` | `render_locked` builds a `Switch` named after the number of characters typed, `format!("{typed} typed")` |
| 76 | `the_open_panel_cleans_the_vault_name_as_the_locked_panel_does` | `turns_direction` without U+2067 |
| 77 | `the_panel_remembers_whether_the_app_was_hidden_and_hides_it_again` | closing hides the app whether it was hidden or not |
| 78 | same | opening unhides the app but orders no window out |
| 79 | `the_preview_passes_the_flags_the_field_and_the_ring_need` | the secret field gets `.assistive(true)` (red through the test's source part; the sentinel test stays green, since a field that is not revealed puts nothing into the tree) |
| 80 | `the_preview_shows_the_user_and_hides_the_secret_and_the_code`, `the_preview_passes_the_flags_the_field_and_the_ring_need` | the secret field gets `.revealed(true)` |
| 81 | same | the ring gets `.masked(false)` |
| 82 | `the_preview_reads_the_entry_of_the_hit_not_the_main_selection` | the preview's password copy reads the entry the main window selected |
| 83 | `preview_copies_go_through_the_service_and_keep_the_panel_open`, `the_panel_never_touches_the_pasteboard` | the copy writes with `cx.write_to_clipboard` instead of `write_secret` |
| 84 | `a_failed_website_shows_a_carmine_line_without_the_title` | the sentence for a website that did not open ends in the entry's title and host |
| 85 | `a_short_client_shows_and_ranks_as_the_launch_writes_it`, `the_footer_names_the_client_the_launch_writes`, `one_and_001_are_twins` | `SapSystem::read` takes `SAP_CLIENT` as it is |
| 86 | `a_refused_sign_in_shows_a_carmine_line_and_keeps_the_panel_open` | `sign_in` drops every launch error without a word |
| 87 | `the_panel_remembers_whether_the_app_was_hidden_and_hides_it_again` | opening orders the windows out without unhiding the app first |
| 88 | `the_main_window_comes_back_when_the_panel_closes_for_it` | closing for the main window hides the app again instead of bringing the windows back |
| 89 | `the_ring_ticks_only_while_a_code_is_shown` | the ring's clock runs for any preview, with or without a code |
| 90 | `the_preview_passes_the_flags_the_field_and_the_ring_need` | VoiceOver is asked only when the panel opens, not when it becomes key |
| 91 | `the_carmine_line_clears_on_the_next_key` | the key interceptor no longer takes the line away |
| 92 | `every_menu_row_is_a_shortcut_pair_from_the_command_table`, `no_literals_in_render_code` | a menu row written by hand, its pair from `Shortcut::from_keystroke` and its word as a literal |
| 93 | `the_more_menu_lists_the_panel_commands_enabled_for_the_selection` | the menu lists Return's main action while Return waits for a production system |
| 94 | `menu_open_is_set_while_the_popover_shows` | `render` no longer tells the footer that the menu is open |
| 95 | same | Escape clears the search before it closes the menu |
| 96 | same | the popover takes the keyboard from the search field (no `track_focus`) |
| 97 | `the_preview_shows_the_user_and_hides_the_secret_and_the_code` | the copy button of the one-time code gets the current code as its description |
| 98 | same | the copy button of the one-time code is named with the current code instead of `tr!` |
| 99 | same | `render_open` gives the copy button of the one-time code the current code as its description after `preview` built it |
| 100 | same | a helper in `view.rs`, called from `preview`, gives the copy button of the one-time code the current code as its description |
| 101 | same | the same through a method in an `impl IconButton` block in `view.rs` |
| 102 | same | the same through a helper whose signature names the button only as `CopyButton`, brought in with `IconButton as CopyButton` |
| 103 | same | the same through a helper that takes and returns a struct holding the button |
| 104 | same | the same through a helper in the panel's `mod.rs` |
| 105 | same | the copy button of the one-time code gets the current code as its description (red where the button's node is read, before the source scan) |
| 106 | same | a helper in `src/app/actions.rs`, outside the folders the source scan reads, called from `preview`, gives the copy button of the one-time code the current code as its description |
| 107 | `the_panel_remembers_whether_the_app_was_hidden_and_hides_it_again`, `the_main_window_comes_back_when_the_panel_closes_for_it`, `the_main_window_comes_back_with_the_app_after_the_panel_hid_it_again` | `open` orders the windows out first and unhides the app after |
| 108 | `the_panel_remembers_whether_the_app_was_hidden_and_hides_it_again`, `the_main_window_comes_back_with_the_app_after_the_panel_hid_it_again` | closing hides the app again without putting the main window back first |
| 109 | `the_panel_remembers_whether_the_app_was_hidden_and_hides_it_again` | closing from inside the panel hides the app at once, while GPUI still lists the panel's window |
| 110 | same | closing from outside the panel hides the app before it removes the panel's window |
| 111 | same | closing hides the app first and puts the main window back after |

The agents spent about 16 hours on this, including review and fixes.

# P1a.03: Command table

Quick Access, the menu bar item's menu, the main window's menus and the command palette will all offer commands once they are built. This step puts those commands in one table, `src/app/registry/`, so a command has one name, one word and one set of keys wherever it appears, and whether it is available is decided in one place. Nothing is bound or shown yet: the old main window keeps its bindings and menus in `src/app/actions.rs` until the new surfaces take the table over.

Started from `p1a-02-i18n` at `2a437c6` on 2026-11-01, because the words are text keys.

| Commit | What |
|---|---|
| `888c35f` | The table, the words in `locales/action.yml`, the keys |
| `7d51c3f` | Availability, the main action per entry kind, Return and production systems |
| `ec1ccf6` | This page |
| `c01da34` | A test that names every command touching a password or one-time code |
| `28782da` | Return takes the word of the selected entry's main action |
| `d80a26f` | Return's word and what the types leave out, on this page |
| `46ecd86` | Return's word shows once Quick Access is built, not before |
| `6e39271` | Open address is tested with smb, ssh and vnc addresses, which have a host |
| `2e61298` | This page and the module comment say what will read the table, not what does |
| `d185179` | Open address is tested in every situation the table can be asked about |
| `3e2b008` | A test holds every command to its keys |
| `c03e491` | `every_label_key_exists` checks the English label; a missing German one fails `every_key_has_both_languages` |
| the last one | This page: the word Copy user name, which rules were broken on purpose, and a limit of Sign in |

## What the table holds

One `CommandSpec` per command:

| Field | What |
|---|---|
| `id` | A stable name, `<area>.<verb>`: `entry.copy_password`, `app.quit`, `panel.close`. The names are constants on `CommandId` |
| `label_key`, `label` | The command's word, see below |
| `keys` | GPUI key strings such as `cmd-shift-c`, written the way GPUI writes them back. Drawing them as keycaps is up to the components that show them |
| `surfaces` | Where the command appears: `Panel` (Quick Access, its list of all commands and its `[...]` menu) or `StatusMenu` (the menu bar item's right-click menu). The main window, its menus, the Dock and the command palette join when they are built |
| `target` | What it acts on: the selected entry, the app, or the window it is given in |
| `enabled` | Whether it can be used in a given situation, see below |
| `touches_secret` | Whether it reads or hands on a password or one-time code: Return, Sign in, Copy password and Copy code. Every such command acts on the selected entry |

The commands so far:

| Command | English | German | Keys | Where |
|---|---|---|---|---|
| `entry.primary` | Main action | Hauptaktion | `enter` | Panel |
| `entry.sap_login` | Sign in | Anmelden | | through `entry.primary` |
| `entry.open_website` | Open website | Website öffnen | | through `entry.primary` |
| `entry.copy_password` | Copy password | Passwort kopieren | `cmd-c` | Panel |
| `entry.copy_username` | Copy user name | Benutzer kopieren | `cmd-shift-c` | Panel |
| `entry.copy_code` | Copy code | Code kopieren | `alt-cmd-c` | Panel |
| `entry.copy_url` | Copy address | Adresse kopieren | `cmd-shift-l` | Panel |
| `entry.open_url` | Open address | Adresse öffnen | `alt-enter` | Panel |
| `entry.show_in_app` | Show in FerrisPass | In FerrisPass zeigen | `cmd-o` | Panel |
| `app.quick_access` | Quick Access | Schnellzugriff | | menu bar item |
| `app.open` | Open FerrisPass | FerrisPass öffnen | | menu bar item |
| `app.lock_all` | Lock everything | Alles sperren | `cmd-l` | Panel, menu bar item |
| `app.settings` | Settings | Einstellungen | `cmd-,` | Panel, menu bar item |
| `app.quit` | Quit | Beenden | `cmd-q` | menu bar item |
| `panel.close` | Close | Schließen | `cmd-w`, `escape` | Panel |

`every_command_has_its_keys` writes these keys out once more, so a key cannot go missing, change or move to another command without a test failing.

## How the words are keyed

`locales/action.yml` holds one word or short phrase per command, in English and German. The command `<area>.<verb>` reads `action.<verb>.label`, and no screen names a command with words of its own. `locales/menubar.yml` held five words for the menu bar item's menu (`menubar.menu.*`), reserved until the menu is built. They are gone: once it is built, the menu will show the command words.

The English words follow the other catalogues, which write user name as two words: Quick Access confirms a copy with "User name copied" and labels the field "User name" (`locales/quick.yml`). So the command is Copy user name, not Copy username.

Each row reads its word with `tr!("action.<verb>.label")` in a small function, so the key scans in `src/i18n/tests.rs` treat it like any other text: `used_keys_exist` checks the key exists, and `no_unused_keys` counts it as read. The row also holds the key as text, `label_key`, and `label_key_matches_label` checks that the two name the same key. To read the catalogues from that test, `src/i18n/mod.rs` has a test-only `catalogue_text(language, key)`.

`entry.primary` has a word of its own, Main action, for lists of shortcuts, where no entry is in view. Wherever an entry is in view, Return goes by what it does there: `primary_label(kind)` gives the word of the entry's main action, Sign in, Open website or Copy password. Once Quick Access is built, its footer and its list of all commands show that word.

## When a command is available

`enabled` gets a `CommandCtx`: whether the app is locked, whether the search is empty, and the selected entry if there is one (its kind, whether the user picked it, what its address field holds, whether it has a one-time code and a user name). It answers `Enabled`, `Disabled` with the text key of the reason a surface shows, or `Hidden`.

- Commands on an entry are hidden while the app is locked and while nothing is selected.
- Copy code needs a one-time code, Copy user name a user name, Copy address any address.
- Open address opens only http and https. An address without a scheme, such as `github.com/login`, counts as https, because that is how the main window opens it. `ftp:`, `javascript:`, `file:` and other schemes can be copied but not opened.
- Lock everything is hidden while everything is locked.
- Quick Access, Open FerrisPass, Settings, Quit and Close are always there.

`for_surface(surface, ctx)` lists what a surface shows, in table order, without the hidden commands. For the menu bar item that is Quick Access, Open FerrisPass, Lock everything, Settings and Quit.

## Return and production systems

Return runs the main action of the selected entry, which depends on its kind: `primary_command` gives `entry.sap_login` for an SAP system, `entry.open_website` for a web login and `entry.copy_password` for remote access. Sign in and Open website have no keys and no surface of their own, so Return reaches them only through `entry.primary`. `only_entry_primary_takes_return` makes sure no other command sits on Return.

Mixing up a test system and a production system is the most expensive mistake a user can make here. So from an empty search, Return never acts on a production system the user has not picked: `entry.primary` is disabled, with the reason `quick.footer.prd_select_first` ("Production system: select it first (↓ or ⌘1), then ⏎"). Typing in the search lifts the rule, and so does picking the row with the arrow keys or Cmd+1 to Cmd+9. A production system that is selected because it came first, or because it is the only match, has not been picked.

The rule is the availability of `entry.primary` and is written down nowhere else. `prd_rule_holds_for_every_input` checks it for every combination of locked, empty search, production and picked, and that nothing else in the context changes the answer. Which row the user picked is up to the search; the context takes it as a plain flag.

## What the types leave out

Quick Access and the menu bar item will build on these types. Three things the types do not have are left out on purpose:

- `CommandCtx` has no surface. No command's availability depends on where it is shown: `for_surface` takes the surface and picks the rows, and `enabled` sees only the situation. A command that is available on one surface and not on another brings the field with it.
- `TargetKind` has no session target, because no command acts on a single vault's session yet. The first one that does adds it.
- `Selection` says what the address field holds, as a `UrlKind` (empty, an http or https address, anything else), rather than only whether there is one. Copy address takes any address and Open address only http and https, so a yes or no would not be enough.

## What later steps add

- Quick Access binds its keys from the table, runs the commands, shows the main action (`primary_label`) and the reason in its footer, and lists all commands for the selected entry. It works out the context from the entry it shows.
- The menu bar item builds its menu from `for_surface(Surface::StatusMenu, ..)`.
- The main window's menus, the Dock menu and the command palette get their surfaces and rows, and the old main window's shortcuts move into the table.
- Letters are bound by character and punctuation by the key of the active keyboard layout, so `cmd-,` follows the layout, and keycaps show that layout.
- A help text per command (`action.<verb>.help`) and search words for the command palette.

## Limits

- Lock everything is hidden rather than disabled while everything is locked. A disabled command shows a reason, and there is nothing to explain.
- URL rules read a host and port without a scheme, such as `localhost:8080`, as the scheme `localhost`. Such an address can be copied but not opened.
- The English words are in sentence case ("Copy password"). Menus on macOS use title case; the menus decide that when they are built.
- Sign in on its own, `entry.sap_login`, is enabled for every SAP system, including a production system nobody picked from an empty search. The rule against signing in to production by accident holds only because Return reaches Sign in through `entry.primary`, which carries the rule, and Sign in has no keys and no surface of its own. A later surface that lists Sign in directly has to apply the same rule.

## Checks

All five gates pass on every commit: `cargo fmt --all --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo test --locked`, `cargo audit`, and the dash scans over tracked files and commit messages. `src/app/actions.rs`, `tests/keymap.rs` and `tests/snapshots/keymap.txt` are unchanged, and the keymap snapshot test passes. The tests from the translation step pass with the new catalogue.

| Test | Fails when |
|---|---|
| `ids_are_unique` | two rows share a name or a label key |
| `ids_name_an_area_and_a_verb` | a name is not `<area>.<verb>` in snake_case, or its label key is not `action.<verb>.label` |
| `every_label_key_exists` | a label key has no English text. A label without German reads the English text, as `tr!` does, so this test cannot see a missing German label; `every_key_has_both_languages` in `src/i18n/tests.rs` fails for it |
| `label_key_matches_label` | `label` reads another key than `label_key` names |
| `every_keystroke_parses` | a key does not parse, is not written the way GPUI writes it back, or names no key |
| `no_shortcut_takes_an_option_character` | a key uses Option with a character and without Cmd or Ctrl. On a Mac that types a character (Option-L is @ on a German keyboard), and the shortcut would take it away from every text field |
| `no_keystroke_collision_per_surface` | two commands on one surface share a key |
| `every_command_has_its_keys` | a command loses a key, gains one or has another one than the test lists, or a command is missing from the test's list |
| `touches_secret_needs_an_entry_target` | a command that touches a secret acts on something other than the selected entry |
| `exactly_these_commands_touch_a_secret` | a command gains or loses the mark for touching a password or one-time code, or Return lacks it while a main action it runs has it |
| `primary_action_by_kind` | an entry kind gets the wrong main action |
| `primary_label_names_the_main_action` | Return's word for an entry is not the word of that entry's main action |
| `prd_rule_holds_for_every_input` | Return and production systems, see above |
| `only_entry_primary_takes_return` | another command sits on Return |
| `every_disabled_reason_exists` | a reason key is missing or has a placeholder |
| `locked_hides_every_entry_command`, `no_selection_hides_every_entry_command` | a command on an entry is offered while the app is locked or nothing is selected |
| `copy_code_needs_otp`, `copy_username_needs_a_user_name` | Copy code or Copy user name is offered for an entry without one |
| `open_url_needs_http_or_https` | an address other than http or https counts as one, Open address is offered in any situation other than an unlocked app with an http or https address selected, or an empty address can be copied |
| `status_menu_has_exactly_the_five_items` | the menu bar item's menu gains, loses or reorders an item |
| `panel_more_lists_every_panel_command` | the panel's list of all commands misses one, or one is disabled that should not be |

These rules were broken on purpose, one at a time, and each time the named test failed:

- Return and production systems, `prd_rule_holds_for_every_input`: the condition without the part for a picked row, for the empty search, or for production.
- Commands on an entry while the app is locked, `locked_hides_every_entry_command`: the lock not looked at.
- `label_key_matches_label`: a row whose `label_key` names another command's word.
- Open address, `open_url_needs_http_or_https`: `ftp` let through, any address that is not empty opened, a few named schemes kept out instead of only http and https let through, and any address opened on a row the user picked.
- `status_menu_has_exactly_the_five_items`: Copy password added to the menu bar item's menu.
- `only_entry_primary_takes_return`: Open address on Return.
- `exactly_these_commands_touch_a_secret`: Copy password without the mark, and Open website with it.
- `primary_label_names_the_main_action`: Return's word read from its own row.
- `every_command_has_its_keys`: Return without a key, and Copy user name on `cmd-shift-u`.
- `every_key_has_both_languages`: Copy user name without German. `every_label_key_exists` passes with it.

The other tests were not probed. [PR #15](https://github.com/elias-tilegant/ferrispass/pull/15) lists the probes and their failing output.

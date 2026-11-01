# P1a.03: Command table

Quick Access, the menu bar item's menu and later the main window's menus and the command palette all offer commands. This step puts them in one table, `src/app/registry/`, so a command has one name, one word and one set of keys wherever it appears, and whether it is available is decided in one place. Nothing is bound or shown yet: the old main window keeps its bindings and menus in `src/app/actions.rs` until the new surfaces take the table over.

Started from `p1a-02-i18n` at `2a437c6` on 2026-11-01, because the words are text keys.

| Commit | What |
|---|---|
| `888c35f` | The table, the words in `locales/action.yml`, the keys |
| `7d51c3f` | Availability, the main action per entry kind, Return and production systems |
| the last one | This page |

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
| `touches_secret` | Whether it reads or hands on a password or one-time code. Every such command acts on the selected entry |

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

## How the words are keyed

`locales/action.yml` holds one word or short phrase per command, in English and German. The command `<area>.<verb>` reads `action.<verb>.label`, and no screen names a command with words of its own. The menu bar item's menu had five words of its own (`menubar.menu.*`); they are gone, and its menu shows the command words.

Each row reads its word with `tr!("action.<verb>.label")` in a small function, so the key scans in `src/i18n/tests.rs` treat it like any other text: `used_keys_exist` checks the key exists, and `no_unused_keys` counts it as read. The row also holds the key as text, `label_key`, and `label_key_matches_label` checks that the two name the same key. To read the catalogues from that test, `src/i18n/mod.rs` has a test-only `catalogue_text(language, key)`.

`entry.primary` has a word of its own, Main action, for lists of shortcuts. A list of what the selected entry offers shows the word of the main action itself, Sign in or Open website.

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

## What later steps add

- Quick Access binds its keys from the table, runs the commands, shows the main action and the reason in its footer, and lists all commands for the selected entry. It works out the context from the entry it shows.
- The menu bar item builds its menu from `for_surface(Surface::StatusMenu, ..)`.
- The main window's menus, the Dock menu and the command palette get their surfaces and rows, and the old main window's shortcuts move into the table.
- Letters are bound by character and punctuation by the key of the active keyboard layout, so `cmd-,` follows the layout, and keycaps show that layout.
- A help text per command (`action.<verb>.help`) and search words for the command palette.

## Limits

- Lock everything is hidden rather than disabled while everything is locked. A disabled command shows a reason, and there is nothing to explain.
- URL rules read a host and port without a scheme, such as `localhost:8080`, as the scheme `localhost`. Such an address can be copied but not opened.
- The English words are in sentence case ("Copy password"). Menus on macOS use title case; the menus decide that when they are built.

## Checks

All five gates pass on every commit: `cargo fmt --all --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo test --locked`, `cargo audit`, and the dash scans over tracked files and commit messages. `src/app/actions.rs`, `tests/keymap.rs` and `tests/snapshots/keymap.txt` are unchanged, and the keymap snapshot test passes. The tests from the translation step pass with the new catalogue.

| Test | Fails when |
|---|---|
| `ids_are_unique` | two rows share a name or a label key |
| `ids_name_an_area_and_a_verb` | a name is not `<area>.<verb>` in snake_case, or its label key is not `action.<verb>.label` |
| `every_label_key_exists` | a label key has no text in one of the languages |
| `label_key_matches_label` | `label` reads another key than `label_key` names |
| `every_keystroke_parses` | a key does not parse, is not written the way GPUI writes it back, or names no key |
| `no_shortcut_takes_an_option_character` | a key uses Option with a character and without Cmd or Ctrl. On a Mac that types a character (Option-L is @ on a German keyboard), and the shortcut would take it away from every text field |
| `no_keystroke_collision_per_surface` | two commands on one surface share a key |
| `touches_secret_needs_an_entry_target` | a command that touches a secret acts on something other than the selected entry |
| `primary_action_by_kind` | an entry kind gets the wrong main action |
| `prd_rule_holds_for_every_input` | Return and production systems, see above |
| `only_entry_primary_takes_return` | another command sits on Return |
| `every_disabled_reason_exists` | a reason key is missing or has a placeholder |
| `locked_hides_every_entry_command`, `no_selection_hides_every_entry_command` | a command on an entry is offered while the app is locked or nothing is selected |
| `copy_code_needs_otp`, `copy_username_needs_a_user_name` | Copy code or Copy user name is offered for an entry without one |
| `open_url_needs_http_or_https` | an address other than http or https could be opened, or an empty one copied |
| `status_menu_has_exactly_the_five_items` | the menu bar item's menu gains, loses or reorders an item |
| `panel_more_lists_every_panel_command` | the panel's list of all commands misses one, or one is disabled that should not be |

Each rule was checked by breaking it on purpose; [PR #15](https://github.com/elias-tilegant/ferrispass/pull/15) lists the probes and their failing output.

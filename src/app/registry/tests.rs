//! The command table checked against itself, the catalogues and GPUI's key
//! parser.

use std::collections::{BTreeSet, HashMap};

use gpui::Keystroke;

use super::Enablement::{Enabled, Hidden};
use super::*;
use crate::i18n::{Language, catalogue_text};

fn id(spec: &CommandSpec) -> &'static str {
    spec.id.as_str()
}

fn parsed(spec: &CommandSpec, key: &str) -> Keystroke {
    Keystroke::parse(key)
        .unwrap_or_else(|err| panic!("{}: `{key}` does not parse: {err:?}", id(spec)))
}

// ----------------------------------------------------------- names, words --

#[test]
fn ids_are_unique() {
    let mut ids = BTreeSet::new();
    let mut label_keys = BTreeSet::new();
    for spec in commands() {
        assert!(ids.insert(id(spec)), "{} is in the table twice", id(spec));
        assert!(
            label_keys.insert(spec.label_key),
            "{} is the label of two commands",
            spec.label_key
        );
        assert!(
            command(spec.id).is_some_and(|found| std::ptr::eq(found, spec)),
            "command({}) finds another row",
            id(spec)
        );
    }
}

#[test]
fn ids_name_an_area_and_a_verb() {
    let snake_case = |part: &str| {
        !part.is_empty()
            && part.starts_with(|c: char| c.is_ascii_lowercase())
            && part.chars().all(|c| c.is_ascii_lowercase() || c == '_')
    };
    for spec in commands() {
        let (area, verb) = id(spec)
            .split_once('.')
            .unwrap_or_else(|| panic!("{}: no area", id(spec)));
        assert!(
            snake_case(area) && snake_case(verb),
            "{}: ids are <area>.<verb> in snake_case",
            id(spec)
        );
        assert_eq!(
            spec.label_key,
            format!("action.{verb}.label"),
            "{}: the label key follows from the id",
            id(spec)
        );
    }
}

#[test]
fn every_label_key_exists() {
    // English only. A label without German reads the English text, as tr!
    // does, so a German check here could never fail.
    // every_key_has_both_languages in src/i18n/tests.rs fails for it.
    for spec in commands() {
        let text = catalogue_text(Language::English, spec.label_key);
        assert!(
            text.is_some_and(|text| !text.trim().is_empty()),
            "{}: `{}` has no text in locales/action.yml",
            id(spec),
            spec.label_key
        );
    }
}

#[test]
fn label_key_matches_label() {
    // `label` reads its key through tr!, which the key scans see; `label_key`
    // is the same key as text. A row whose two disagree would show one word
    // and be checked for another.
    for spec in commands() {
        let shown = (spec.label)().to_string();
        let keyed = catalogue_text(Language::English, spec.label_key);
        assert_eq!(
            Some(shown),
            keyed,
            "{}: `label` does not read `{}`",
            id(spec),
            spec.label_key
        );
    }
}

// ------------------------------------------------------------------ keys --

/// Keys with a name rather than a character, as GPUI writes them.
const NAMED_KEYS: &[&str] = &[
    "enter",
    "escape",
    "tab",
    "space",
    "backspace",
    "delete",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
    "pageup",
    "pagedown",
];

#[test]
fn every_keystroke_parses() {
    for spec in commands() {
        for key in spec.keys {
            let keystroke = parsed(spec, key);
            // GPUI takes "shift-cmd-c" and "cmd-shift-c" alike. Writing each
            // key the one way GPUI writes it back keeps the table greppable
            // and the same as the keymap snapshot.
            assert_eq!(
                keystroke.unparse(),
                *key,
                "{}: write `{key}` the way GPUI does",
                id(spec)
            );
            // The parser takes any word as a key name, so a misspelt one
            // would never fire.
            assert!(
                keystroke.key.chars().count() == 1 || NAMED_KEYS.contains(&keystroke.key.as_str()),
                "{}: `{key}` is no key GPUI knows",
                id(spec)
            );
        }
    }
}

#[test]
fn no_shortcut_takes_an_option_character() {
    // Option with a character key and without Cmd or Ctrl types a character
    // on Mac layouts: Option-L is @ on a German keyboard. A shortcut on it
    // would take that character away from every text field.
    for spec in commands() {
        for key in spec.keys {
            let keystroke = parsed(spec, key);
            let modifiers = keystroke.modifiers;
            let types_a_character = modifiers.alt
                && !modifiers.platform
                && !modifiers.control
                && keystroke.key.chars().count() == 1;
            assert!(
                !types_a_character,
                "{}: `{key}` takes an Option character",
                id(spec)
            );
        }
    }
}

#[test]
fn no_keystroke_collision_per_surface() {
    let mut taken: HashMap<(Surface, Keystroke), &str> = HashMap::new();
    for spec in commands() {
        for surface in spec.surfaces {
            for key in spec.keys {
                let keystroke = parsed(spec, key);
                if let Some(other) = taken.insert((*surface, keystroke), id(spec)) {
                    panic!(
                        "`{key}` on {surface:?} is taken twice: {other} and {}",
                        id(spec)
                    );
                }
            }
        }
    }
}

#[test]
fn every_command_has_its_keys() {
    // Each command's keys, written out once more. The tests above check
    // that keys parse and do not collide, which a key that went missing or
    // changed would still pass. The keys are written the way GPUI writes
    // them back, as in the table.
    let agreed: &[(&str, &[&str])] = &[
        ("entry.primary", &["enter"]),
        // Return reaches these two through entry.primary.
        ("entry.sap_login", &[]),
        ("entry.open_website", &[]),
        ("entry.copy_password", &["cmd-c"]),
        ("entry.copy_username", &["cmd-shift-c"]),
        ("entry.copy_code", &["alt-cmd-c"]),
        ("entry.copy_url", &["cmd-shift-l"]),
        ("entry.open_url", &["alt-enter"]),
        ("entry.show_in_app", &["cmd-o"]),
        ("app.quick_access", &[]),
        ("app.open", &[]),
        ("app.lock_all", &["cmd-l"]),
        ("app.settings", &["cmd-,"]),
        ("app.quit", &["cmd-q"]),
        ("panel.close", &["cmd-w", "escape"]),
    ];
    for spec in commands() {
        let keys = agreed
            .iter()
            .find(|(name, _)| *name == id(spec))
            .map(|(_, keys)| *keys)
            .unwrap_or_else(|| panic!("{}: not in the list of keys", id(spec)));
        assert_eq!(spec.keys, keys, "{}", id(spec));
    }
    for (name, _) in agreed {
        assert!(
            commands().iter().any(|spec| id(spec) == *name),
            "{name} is not in the table"
        );
    }
}

// --------------------------------------------------------------- secrets --

#[test]
fn touches_secret_needs_an_entry_target() {
    let secret: Vec<&CommandSpec> = commands()
        .iter()
        .filter(|spec| spec.touches_secret)
        .collect();
    assert!(!secret.is_empty(), "no command touches a secret");
    for spec in secret {
        assert_eq!(
            spec.target,
            TargetKind::Entry,
            "{}: a command that touches a secret acts on the selected entry",
            id(spec)
        );
    }
}

#[test]
fn exactly_these_commands_touch_a_secret() {
    // Whoever runs a command decides by this flag how carefully to handle
    // it. A row that lost the flag would still pass the check above, so the
    // list names every one.
    let secret: Vec<&str> = commands()
        .iter()
        .filter(|spec| spec.touches_secret)
        .map(id)
        .collect();
    assert_eq!(
        secret,
        [
            "entry.primary",
            "entry.sap_login",
            "entry.copy_password",
            "entry.copy_code",
        ]
    );
    // Return runs whichever main action the selected entry has, so it
    // touches a secret as soon as one of them does.
    let primary = command(CommandId::ENTRY_PRIMARY).expect("entry.primary is in the table");
    for kind in KINDS {
        let main = primary_command(&kind);
        if command(main).is_some_and(|spec| spec.touches_secret) {
            assert!(
                primary.touches_secret,
                "{} touches a secret and Return runs it",
                main.as_str()
            );
        }
    }
}

// ---------------------------------------------------------- availability --

const PRD_SELECT_FIRST: Enablement = Enablement::Disabled("quick.footer.prd_select_first");

const KINDS: [SelectedKind; 4] = [
    SelectedKind::Sap { production: false },
    SelectedKind::Sap { production: true },
    SelectedKind::Remote,
    SelectedKind::Web,
];
const URLS: [UrlKind; 3] = [UrlKind::Empty, UrlKind::Web, UrlKind::Other];
const BOTH: [bool; 2] = [false, true];

/// An entry of this kind with everything a command could ask for, selected
/// because it came first.
fn full(kind: SelectedKind) -> Selection {
    Selection {
        kind,
        explicit: false,
        url: UrlKind::Web,
        has_otp: true,
        has_username: true,
    }
}

fn ctx(locked: bool, query_empty: bool, selection: Option<Selection>) -> CommandCtx {
    CommandCtx {
        locked,
        query_empty,
        selection,
    }
}

/// Every selection the context can describe, and none.
fn every_selection() -> Vec<Option<Selection>> {
    let mut all = vec![None];
    for kind in KINDS {
        for explicit in BOTH {
            for url in URLS {
                for has_otp in BOTH {
                    for has_username in BOTH {
                        all.push(Some(Selection {
                            kind,
                            explicit,
                            url,
                            has_otp,
                            has_username,
                        }));
                    }
                }
            }
        }
    }
    all
}

/// Every context: each selection, locked or not, with and without a search.
fn every_ctx() -> Vec<CommandCtx> {
    let mut all = Vec::new();
    for locked in BOTH {
        for query_empty in BOTH {
            for selection in every_selection() {
                all.push(ctx(locked, query_empty, selection));
            }
        }
    }
    all
}

fn enablement(command_id: CommandId, ctx: &CommandCtx) -> Enablement {
    let spec = command(command_id)
        .unwrap_or_else(|| panic!("{} is not in the table", command_id.as_str()));
    (spec.enabled)(ctx)
}

fn shown(surface: Surface, ctx: &CommandCtx) -> Vec<(&'static str, Enablement)> {
    for_surface(surface, ctx)
        .map(|(spec, enablement)| (id(spec), enablement))
        .collect()
}

fn entry_commands() -> Vec<&'static CommandSpec> {
    commands()
        .iter()
        .filter(|spec| spec.target == TargetKind::Entry)
        .collect()
}

#[test]
fn primary_action_by_kind() {
    for (kind, main) in [
        (
            SelectedKind::Sap { production: false },
            CommandId::ENTRY_SAP_LOGIN,
        ),
        (
            SelectedKind::Sap { production: true },
            CommandId::ENTRY_SAP_LOGIN,
        ),
        (SelectedKind::Web, CommandId::ENTRY_OPEN_WEBSITE),
        (SelectedKind::Remote, CommandId::ENTRY_COPY_PASSWORD),
    ] {
        assert_eq!(primary_command(&kind), main, "{kind:?}");
        assert_eq!(
            command(main).map(|spec| spec.target),
            Some(TargetKind::Entry)
        );
        // Picked from a search, the main action is there, and the other
        // kinds' main actions are not.
        let picked = ctx(false, false, Some(full(kind)));
        assert_eq!(enablement(main, &picked), Enabled, "{kind:?}");
        let sap = matches!(kind, SelectedKind::Sap { .. });
        let web = kind == SelectedKind::Web;
        assert_eq!(
            enablement(CommandId::ENTRY_SAP_LOGIN, &picked),
            if sap { Enabled } else { Hidden },
            "{kind:?}"
        );
        assert_eq!(
            enablement(CommandId::ENTRY_OPEN_WEBSITE, &picked),
            if web { Enabled } else { Hidden },
            "{kind:?}"
        );
    }
}

#[test]
fn primary_label_names_the_main_action() {
    // With an entry in view, Return goes by the word of what it does there.
    for (kind, word) in [
        (
            SelectedKind::Sap { production: false },
            "action.sap_login.label",
        ),
        (
            SelectedKind::Sap { production: true },
            "action.sap_login.label",
        ),
        (SelectedKind::Web, "action.open_website.label"),
        (SelectedKind::Remote, "action.copy_password.label"),
    ] {
        assert_eq!(
            Some(primary_label(&kind).to_string()),
            catalogue_text(Language::English, word),
            "{kind:?}"
        );
    }
}

#[test]
fn prd_rule_holds_for_every_input() {
    // The rule over the four inputs it reads, for an SAP system.
    for locked in BOTH {
        for query_empty in BOTH {
            for production in BOTH {
                for explicit in BOTH {
                    let expected = match (locked, query_empty, production, explicit) {
                        (true, _, _, _) => Hidden,
                        (false, true, true, false) => PRD_SELECT_FIRST,
                        _ => Enabled,
                    };
                    let selection = Selection {
                        explicit,
                        ..full(SelectedKind::Sap { production })
                    };
                    assert_eq!(
                        enablement(
                            CommandId::ENTRY_PRIMARY,
                            &ctx(locked, query_empty, Some(selection))
                        ),
                        expected,
                        "locked {locked}, empty search {query_empty}, \
                         production {production}, picked {explicit}"
                    );
                }
            }
        }
    }
    // Nothing else changes the outcome: not the kind, the address, the code
    // or the user name. Without a selection there is nothing to act on.
    for ctx in every_ctx() {
        let expected = match ctx.selection {
            _ if ctx.locked => Hidden,
            None => Hidden,
            Some(selection)
                if ctx.query_empty
                    && matches!(selection.kind, SelectedKind::Sap { production: true })
                    && !selection.explicit =>
            {
                PRD_SELECT_FIRST
            }
            Some(_) => Enabled,
        };
        assert_eq!(
            enablement(CommandId::ENTRY_PRIMARY, &ctx),
            expected,
            "{ctx:?}"
        );
    }
}

#[test]
fn only_entry_primary_takes_return() {
    // The rule for production systems sits on entry.primary. Any other
    // command on plain Return would get past it.
    let enter = Keystroke::parse("enter").expect("enter parses");
    for spec in commands() {
        for key in spec.keys {
            if parsed(spec, key) == enter {
                assert_eq!(
                    spec.id,
                    CommandId::ENTRY_PRIMARY,
                    "{} takes Return",
                    id(spec)
                );
            }
        }
    }
}

#[test]
fn every_disabled_reason_exists() {
    let mut reasons = BTreeSet::new();
    for ctx in every_ctx() {
        for spec in commands() {
            if let Enablement::Disabled(reason) = (spec.enabled)(&ctx) {
                reasons.insert(reason);
            }
        }
    }
    assert!(!reasons.is_empty(), "no command is ever disabled");
    for reason in reasons {
        for language in Language::ALL {
            let text = catalogue_text(language, reason)
                .unwrap_or_else(|| panic!("`{reason}` is not in locales/"));
            // A surface shows the reason as it is, with nothing to fill in.
            assert!(!text.contains("%{"), "`{reason}` has a placeholder");
        }
    }
}

#[test]
fn locked_hides_every_entry_command() {
    let entry_commands = entry_commands();
    assert!(!entry_commands.is_empty());
    for ctx in every_ctx().into_iter().filter(|ctx| ctx.locked) {
        for spec in &entry_commands {
            assert_eq!((spec.enabled)(&ctx), Hidden, "{} while locked", id(spec));
        }
        // What a locked panel still offers.
        assert_eq!(
            shown(Surface::Panel, &ctx),
            [("app.settings", Enabled), ("panel.close", Enabled)],
            "{ctx:?}"
        );
    }
}

#[test]
fn no_selection_hides_every_entry_command() {
    for query_empty in BOTH {
        let nothing = ctx(false, query_empty, None);
        for spec in entry_commands() {
            assert_eq!((spec.enabled)(&nothing), Hidden, "{}", id(spec));
        }
        assert_eq!(
            shown(Surface::Panel, &nothing),
            [
                ("app.lock_all", Enabled),
                ("app.settings", Enabled),
                ("panel.close", Enabled)
            ]
        );
    }
}

#[test]
fn copy_code_needs_otp() {
    for kind in KINDS {
        for has_otp in BOTH {
            let selection = Selection {
                has_otp,
                ..full(kind)
            };
            assert_eq!(
                enablement(
                    CommandId::ENTRY_COPY_CODE,
                    &ctx(false, false, Some(selection))
                ),
                if has_otp { Enabled } else { Hidden },
                "{kind:?}, one-time code {has_otp}"
            );
        }
    }
    // In every situation, Copy code is there exactly when the app is
    // unlocked and the selected entry has a one-time code.
    for ctx in every_ctx() {
        let offered = !ctx.locked && ctx.selection.is_some_and(|selection| selection.has_otp);
        assert_eq!(
            enablement(CommandId::ENTRY_COPY_CODE, &ctx),
            if offered { Enabled } else { Hidden },
            "copy code, {ctx:?}"
        );
    }
}

#[test]
fn copy_username_needs_a_user_name() {
    for kind in KINDS {
        for has_username in BOTH {
            let selection = Selection {
                has_username,
                ..full(kind)
            };
            assert_eq!(
                enablement(
                    CommandId::ENTRY_COPY_USERNAME,
                    &ctx(false, false, Some(selection))
                ),
                if has_username { Enabled } else { Hidden },
                "{kind:?}, user name {has_username}"
            );
        }
    }
    // In every situation, Copy user name is there exactly when the app is
    // unlocked and the selected entry has a user name.
    for ctx in every_ctx() {
        let offered = !ctx.locked
            && ctx
                .selection
                .is_some_and(|selection| selection.has_username);
        assert_eq!(
            enablement(CommandId::ENTRY_COPY_USERNAME, &ctx),
            if offered { Enabled } else { Hidden },
            "copy user name, {ctx:?}"
        );
    }
}

#[test]
fn open_url_needs_http_or_https() {
    for (url, kind) in [
        ("https://example.com", UrlKind::Web),
        ("http://example.com/login", UrlKind::Web),
        ("HTTPS://EXAMPLE.COM", UrlKind::Web),
        ("  https://example.com  ", UrlKind::Web),
        // Without a scheme it opens as https, as in the main window.
        ("github.com/login", UrlKind::Web),
        ("ftp://example.com", UrlKind::Other),
        // Other schemes with a host open another app: a file share, a
        // terminal, screen sharing. Only http and https are let through,
        // so a scheme nobody thought to list stays out as well.
        ("smb://fileserver/share", UrlKind::Other),
        ("ssh://host", UrlKind::Other),
        ("vnc://host", UrlKind::Other),
        ("javascript:alert(1)", UrlKind::Other),
        ("file:///etc/hosts", UrlKind::Other),
        ("mailto:it@example.com", UrlKind::Other),
        ("http://", UrlKind::Other),
        ("not an address", UrlKind::Other),
        ("", UrlKind::Empty),
        ("   ", UrlKind::Empty),
    ] {
        assert_eq!(UrlKind::of(url), kind, "{url:?}");
        let selection = Selection {
            url: UrlKind::of(url),
            ..full(SelectedKind::Web)
        };
        let picked = ctx(false, false, Some(selection));
        assert_eq!(
            enablement(CommandId::ENTRY_OPEN_URL, &picked),
            if kind == UrlKind::Web {
                Enabled
            } else {
                Hidden
            },
            "open {url:?}"
        );
        // Any address can be copied.
        assert_eq!(
            enablement(CommandId::ENTRY_COPY_URL, &picked),
            if kind == UrlKind::Empty {
                Hidden
            } else {
                Enabled
            },
            "copy {url:?}"
        );
    }
    // In every situation, Open address is there exactly when the app is
    // unlocked and the selected entry has an http or https address, and Copy
    // address exactly when it has any address. Picking the row, the search,
    // the kind of entry and what else it has change nothing about that.
    for ctx in every_ctx() {
        let web_address = ctx
            .selection
            .is_some_and(|selection| selection.url == UrlKind::Web);
        let offered = !ctx.locked && web_address;
        assert_eq!(
            enablement(CommandId::ENTRY_OPEN_URL, &ctx),
            if offered { Enabled } else { Hidden },
            "open, {ctx:?}"
        );
        let any_address = ctx
            .selection
            .is_some_and(|selection| selection.url != UrlKind::Empty);
        let copyable = !ctx.locked && any_address;
        assert_eq!(
            enablement(CommandId::ENTRY_COPY_URL, &ctx),
            if copyable { Enabled } else { Hidden },
            "copy address, {ctx:?}"
        );
    }
}

#[test]
fn status_menu_has_exactly_the_five_items() {
    let five = [
        ("app.quick_access", Enabled),
        ("app.open", Enabled),
        ("app.lock_all", Enabled),
        ("app.settings", Enabled),
        ("app.quit", Enabled),
    ];
    for query_empty in BOTH {
        for selection in every_selection() {
            assert_eq!(
                shown(Surface::StatusMenu, &ctx(false, query_empty, selection)),
                five
            );
        }
    }
    // Locked, there is nothing to lock.
    let locked: Vec<(&str, Enablement)> = five
        .into_iter()
        .filter(|(command_id, _)| *command_id != "app.lock_all")
        .collect();
    for selection in every_selection() {
        assert_eq!(
            shown(Surface::StatusMenu, &ctx(true, true, selection)),
            locked
        );
    }
}

#[test]
fn panel_more_lists_every_panel_command() {
    let panel: Vec<&str> = commands()
        .iter()
        .filter(|spec| spec.surfaces.contains(&Surface::Panel))
        .map(id)
        .collect();
    assert_eq!(
        panel,
        [
            "entry.primary",
            "entry.copy_password",
            "entry.copy_username",
            "entry.copy_code",
            "entry.copy_url",
            "entry.open_url",
            "entry.show_in_app",
            "app.lock_all",
            "app.settings",
            "panel.close",
        ]
    );
    // For an entry that has everything, the panel's list of all commands
    // offers every one of them, picked or not, with or without a search.
    // Only the main action can be disabled, by the rule for production
    // systems.
    for kind in KINDS {
        for explicit in BOTH {
            for query_empty in BOTH {
                let selection = Selection {
                    explicit,
                    ..full(kind)
                };
                let listed = shown(Surface::Panel, &ctx(false, query_empty, Some(selection)));
                let ids: Vec<&str> = listed.iter().map(|(command_id, _)| *command_id).collect();
                assert_eq!(ids, panel, "{selection:?}, empty search {query_empty}");
                for (command_id, enablement) in listed {
                    if command_id != "entry.primary" {
                        assert_eq!(enablement, Enabled, "{command_id}, {selection:?}");
                    }
                }
            }
        }
    }
}

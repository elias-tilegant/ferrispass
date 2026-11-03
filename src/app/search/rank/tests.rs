use chrono::Duration;

use super::*;
use crate::app::search::ranked_entries;
use crate::domain::{CustomField, VaultGroup};

use Tier::{ExactIdentifier, Favorite, RecentlyUsed, Text};

fn entry(id: &str, title: &str) -> VaultEntry {
    VaultEntry {
        id: id.to_string(),
        title: title.to_string(),
        ..VaultEntry::default()
    }
}

fn starred(entry: VaultEntry) -> VaultEntry {
    VaultEntry {
        starred: true,
        ..entry
    }
}

fn with_fields(entry: VaultEntry, fields: &[(&str, &str, bool)]) -> VaultEntry {
    VaultEntry {
        custom_fields: fields
            .iter()
            .map(|(key, value, protected)| CustomField {
                key: key.to_string(),
                value: value.to_string(),
                protected: *protected,
            })
            .collect(),
        ..entry
    }
}

fn group(name: &str, entries: Vec<VaultEntry>) -> VaultGroup {
    VaultGroup {
        id: name.to_lowercase(),
        name: name.to_string(),
        entries: entries
            .into_iter()
            .map(|entry| VaultEntry {
                group_path: vec!["Root".to_string(), name.to_string()],
                ..entry
            })
            .collect(),
        ..VaultGroup::default()
    }
}

fn snapshot(groups: Vec<VaultGroup>, entries: Vec<VaultEntry>) -> VaultSnapshot {
    VaultSnapshot::new(VaultGroup {
        id: "root".to_string(),
        name: "Root".to_string(),
        groups,
        entries: entries
            .into_iter()
            .map(|entry| VaultEntry {
                group_path: vec!["Root".to_string()],
                ..entry
            })
            .collect(),
        ..VaultGroup::default()
    })
}

/// Two customers with the same SID and client, and a few entries of the
/// user's own:
///
/// - Nordhafen: NHP 100 PRD, NHQ 100 QAS, NHD 100, and an old entry for
///   NHP 100 the user starred. Its title starts with the SID and client but
///   goes on, so it matches the words "nhp 100" without naming the system.
/// - Brenner: its own NHP 100, and Brenner VPN.
/// - The user's own: Salesforce, starred, and weekly notes, also starred,
///   whose notes and custom field hold the words the tests search for.
fn sample_vault() -> VaultSnapshot {
    sample_vault_with(Vec::new())
}

/// The sample vault with more entries of the user's own.
fn sample_vault_with(more: Vec<VaultEntry>) -> VaultSnapshot {
    snapshot(
        vec![
            group(
                "Nordhafen",
                vec![
                    entry("nhp-prd", "NHP 100 PRD"),
                    entry("nhq-qas", "NHQ 100 QAS"),
                    entry("nhd-dev", "NHD 100"),
                    starred(entry("nhp-old", "NHP 100 Nordhafen alt")),
                ],
            ),
            group(
                "Brenner",
                vec![
                    entry("brenner-nhp", "NHP 100"),
                    entry("brenner-vpn", "Brenner VPN"),
                ],
            ),
        ],
        vec![
            starred(entry("salesforce", "Salesforce")),
            starred(with_fields(
                VaultEntry {
                    notes: "NHP 100 down again, nhq fine, Salesforce and Brenner VPN".to_string(),
                    ..entry("notes", "Weekly notes")
                },
                &[("Remark", "NHP 100 nhq salesforce brenner", false)],
            )),
        ]
        .into_iter()
        .chain(more)
        .collect(),
    )
}

fn set_starred(vault: &mut VaultSnapshot, id: &str, value: bool) {
    fn walk(group: &mut VaultGroup, id: &str, value: bool) {
        for entry in &mut group.entries {
            if entry.id == id {
                entry.starred = value;
            }
        }
        for child in &mut group.groups {
            walk(child, id, value);
        }
    }
    walk(&mut vault.root, id, value);
}

/// Entries used so many minutes ago, by id.
fn used(entries: &[(&str, i64)]) -> HashMap<String, DateTime<Local>> {
    let now = Local::now();
    entries
        .iter()
        .map(|(id, minutes_ago)| (id.to_string(), now - Duration::minutes(*minutes_ago)))
        .collect()
}

fn rows(
    vault: &VaultSnapshot,
    query: &str,
    last_used: &HashMap<String, DateTime<Local>>,
) -> Vec<(String, Tier)> {
    rank(RankInput::new(vault, query, last_used))
        .into_iter()
        .map(|ranked| (ranked.entry.id.clone(), ranked.tier))
        .collect()
}

fn expect(rows: &[(&str, Tier)]) -> Vec<(String, Tier)> {
    rows.iter()
        .map(|(id, tier)| (id.to_string(), *tier))
        .collect()
}

fn exact_ids(vault: &VaultSnapshot, query: &str) -> Vec<String> {
    rows(vault, query, &HashMap::new())
        .into_iter()
        .filter(|(_, tier)| *tier == ExactIdentifier)
        .map(|(id, _)| id)
        .collect()
}

/// One query against the sample vault: the entries used shortly before it,
/// most recent first, and the list the user sees.
struct Case {
    query: &'static str,
    recent: &'static [&'static str],
    listed: &'static [(&'static str, Tier)],
}

/// Every tier and the tie, in the sample vault.
#[test]
fn the_sample_vault_ranks_by_tier() {
    let cases = [
        Case {
            query: "nhp 100",
            recent: &[],
            listed: &[
                ("brenner-nhp", ExactIdentifier),
                ("nhp-prd", ExactIdentifier),
                ("nhp-old", Favorite),
            ],
        },
        Case {
            query: "nhp 100",
            recent: &["nhp-prd"],
            listed: &[
                ("nhp-prd", ExactIdentifier),
                ("brenner-nhp", ExactIdentifier),
                ("nhp-old", Favorite),
            ],
        },
        Case {
            query: "nhp",
            recent: &["brenner-nhp"],
            listed: &[
                ("nhp-old", Favorite),
                ("brenner-nhp", RecentlyUsed),
                ("nhp-prd", Text),
            ],
        },
        Case {
            query: "nhq",
            recent: &["brenner-vpn"],
            listed: &[("nhq-qas", Text)],
        },
        Case {
            query: "nhq 100",
            recent: &[],
            listed: &[("nhq-qas", ExactIdentifier)],
        },
        Case {
            query: "brenner",
            recent: &["brenner-nhp", "brenner-vpn"],
            listed: &[("brenner-nhp", RecentlyUsed), ("brenner-vpn", RecentlyUsed)],
        },
        Case {
            query: "salesforce",
            recent: &[],
            listed: &[("salesforce", ExactIdentifier)],
        },
        Case {
            query: "sales",
            recent: &[],
            listed: &[("salesforce", Favorite)],
        },
        Case {
            query: "",
            recent: &["brenner-vpn"],
            listed: &[],
        },
    ];
    let vault = sample_vault();
    for case in cases {
        let last_used: HashMap<String, DateTime<Local>> = case
            .recent
            .iter()
            .enumerate()
            .map(|(age, id)| (id.to_string(), Local::now() - Duration::minutes(age as i64)))
            .collect();
        assert_eq!(
            rows(&vault, case.query, &last_used),
            expect(case.listed),
            "query {:?}, recently used {:?}",
            case.query,
            case.recent
        );
    }
}

#[test]
fn exact_identifier_beats_everything() {
    let vault = sample_vault();
    // The old entry is a favorite, was used a minute ago and its title even
    // starts with the words; both systems the query names still come first.
    let last_used = used(&[("nhp-old", 1)]);
    assert_eq!(
        rows(&vault, "nhp 100", &last_used),
        expect(&[
            ("brenner-nhp", ExactIdentifier),
            ("nhp-prd", ExactIdentifier),
            ("nhp-old", Favorite),
        ])
    );
}

#[test]
fn nhq_finds_nhq_100_first() {
    let vault = sample_vault();
    // A favorite and a recently used entry that do not match the word stay
    // out, and so do the NHP and NHD systems next to it.
    let last_used = used(&[("brenner-vpn", 1), ("nhp-prd", 2)]);
    assert_eq!(
        rows(&vault, "nhq", &last_used),
        expect(&[("nhq-qas", Text)])
    );
    assert_eq!(
        rows(&vault, "nhq 100", &last_used).first(),
        Some(&("nhq-qas".to_string(), ExactIdentifier))
    );
}

#[test]
fn favorites_beat_recent_beat_text() {
    let vault = sample_vault();
    let last_used = used(&[("brenner-nhp", 1)]);
    assert_eq!(
        rows(&vault, "nhp", &last_used),
        expect(&[
            ("nhp-old", Favorite),
            ("brenner-nhp", RecentlyUsed),
            ("nhp-prd", Text),
        ])
    );
    // By text alone all three titles start with the word, so they go by
    // title, and the favorite is not first.
    let by_text: Vec<&str> = ranked_entries(&vault, "nhp")
        .into_iter()
        .map(|entry| entry.id.as_str())
        .collect();
    assert_eq!(by_text, ["brenner-nhp", "nhp-old", "nhp-prd"]);
}

#[test]
fn recent_newest_first() {
    let vault = sample_vault();
    // Brenner VPN matches better, its title starts with the word, while the
    // other entry only sits in the Brenner group. The more recent use wins
    // either way.
    let last_used = used(&[("brenner-vpn", 120), ("brenner-nhp", 60)]);
    assert_eq!(
        rows(&vault, "brenner", &last_used),
        expect(&[("brenner-nhp", RecentlyUsed), ("brenner-vpn", RecentlyUsed)])
    );
    let last_used = used(&[("brenner-vpn", 60), ("brenner-nhp", 120)]);
    assert_eq!(
        rows(&vault, "brenner", &last_used),
        expect(&[("brenner-vpn", RecentlyUsed), ("brenner-nhp", RecentlyUsed)])
    );
}

#[test]
fn tie_same_sid_two_customers() {
    let nobody = HashMap::new();
    let first_two = |vault: &VaultSnapshot, last_used: &HashMap<String, DateTime<Local>>| {
        let ranked = rank(RankInput::new(vault, "nhp 100", last_used));
        assert!(
            ranked.iter().any(|ranked| ranked.entry.id == "nhp-old"),
            "the old entry matches the words and is listed"
        );
        assert_eq!(
            tie_count(&ranked),
            2,
            "NHP 100 at Brenner and at Nordhafen are the same system; the old entry is not"
        );
        ranked
            .iter()
            .take(2)
            .map(|ranked| ranked.entry.id.clone())
            .collect::<Vec<_>>()
    };

    let mut vault = sample_vault();
    // Neither starred nor used: by title.
    assert_eq!(first_two(&vault, &nobody), ["brenner-nhp", "nhp-prd"]);
    // Recently used comes before the title.
    let prd_used = used(&[("nhp-prd", 1)]);
    assert_eq!(first_two(&vault, &prd_used), ["nhp-prd", "brenner-nhp"]);
    // A favorite comes before recently used.
    set_starred(&mut vault, "brenner-nhp", true);
    assert_eq!(first_two(&vault, &prd_used), ["brenner-nhp", "nhp-prd"]);

    // A unique first row is no tie, and no results are none.
    let vault = sample_vault();
    assert_eq!(
        tie_count(&rank(RankInput::new(&vault, "nhq 100", &nobody))),
        1
    );
    assert_eq!(
        tie_count(&rank(RankInput::new(&vault, "nothing", &nobody))),
        0
    );

    // The same title at two customers ties as well.
    let vault = vault_of_two_salesforce_logins();
    assert_eq!(
        tie_count(&rank(RankInput::new(&vault, "salesforce", &nobody))),
        2
    );
}

#[test]
fn protected_fields_do_not_make_a_tie() {
    let nobody = HashMap::new();
    // A third NHP 100 whose title does not say so; only its fields do.
    let with_hidden_system = |protected: bool| {
        sample_vault_with(vec![with_fields(
            entry("hidden", "Nordhafen 100 NHP"),
            &[
                ("SAP_SID", "NHP", protected),
                ("SAP_CLIENT", "100", protected),
            ],
        )])
    };

    let vault = with_hidden_system(true);
    assert_eq!(
        rows(&vault, "nhp 100", &nobody),
        expect(&[
            ("brenner-nhp", ExactIdentifier),
            ("nhp-prd", ExactIdentifier),
            ("nhp-old", Favorite),
            ("hidden", Text),
        ])
    );
    assert_eq!(
        tie_count(&rank(RankInput::new(&vault, "nhp 100", &nobody))),
        2,
        "protected SAP_SID and SAP_CLIENT fields made the entry a twin"
    );

    // The same fields, not protected, make it the third NHP 100.
    let vault = with_hidden_system(false);
    assert_eq!(
        tie_count(&rank(RankInput::new(&vault, "nhp 100", &nobody))),
        3
    );
}

fn vault_of_two_salesforce_logins() -> VaultSnapshot {
    snapshot(
        vec![
            group("Nordhafen", vec![entry("sf-nordhafen", "Salesforce")]),
            group("Brenner", vec![entry("sf-brenner", "salesforce")]),
        ],
        Vec::new(),
    )
}

#[test]
fn notes_never_match() {
    let vault = sample_vault();
    // The weekly notes are a favorite and were just used, and their notes
    // and a custom field hold every word searched for here. Neither is
    // search text, and notes never name a system.
    let last_used = used(&[("notes", 1)]);
    for query in [
        "nhp 100",
        "nhq",
        "salesforce",
        "brenner vpn",
        "down again",
        "remark",
    ] {
        let listed = rows(&vault, query, &last_used);
        assert!(
            listed.iter().all(|(id, _)| id != "notes"),
            "{query:?} found the notes entry: {listed:?}"
        );
    }
}

#[test]
fn whitespace_and_case_do_not_matter_for_identifiers() {
    let vault = sample_vault();
    for query in ["nhp 100", "NHP 100", "nhp  100", " Nhp\t100 ", "nHp 100"] {
        assert_eq!(
            exact_ids(&vault, query),
            ["brenner-nhp", "nhp-prd"],
            "{query:?}"
        );
    }
    for query in ["salesforce", "SALESFORCE", "  SalesForce "] {
        assert_eq!(exact_ids(&vault, query), ["salesforce"], "{query:?}");
    }
    for query in ["brenner vpn", "BRENNER   VPN"] {
        assert_eq!(exact_ids(&vault, query), ["brenner-vpn"], "{query:?}");
    }
}

#[test]
fn empty_query_returns_nothing() {
    let vault = sample_vault();
    // Favorites and recently used entries need a query to match too.
    let last_used = used(&[("brenner-vpn", 1)]);
    for query in ["", "   ", "\t"] {
        assert!(rows(&vault, query, &last_used).is_empty(), "{query:?}");
    }
}

#[test]
fn identifier_needs_sid_and_client() {
    let vault = sample_vault();
    for query in [
        "nhp",
        "100",
        "nhp 10",
        "nhp 1000",
        "100 nhp",
        "nhp 100 nordhafen",
    ] {
        assert!(exact_ids(&vault, query).is_empty(), "{query:?}");
    }

    // A SID from a field without a client is not one either.
    let vault = snapshot(
        Vec::new(),
        vec![with_fields(
            entry("sid-only", "Nordhafen ERP"),
            &[("SAP_SID", "NHP", false)],
        )],
    );
    for query in ["nhp", "nhp 100"] {
        assert!(rows(&vault, query, &HashMap::new()).is_empty(), "{query:?}");
    }
}

fn alone(entry: VaultEntry) -> VaultSnapshot {
    snapshot(Vec::new(), vec![entry])
}

#[test]
fn a_title_with_a_role_word_names_the_system() {
    // Each of these is one word longer than the query, so none is found by
    // its whole title. The role words are the ones the entry kinds know, in
    // any case.
    for title in [
        "NHP 100 PRD",
        "NHP 100 prd",
        "NHP  100\tQAS",
        "NHP 100 DEV",
        "NHP 100 SBX",
    ] {
        assert_eq!(
            rows(&alone(entry("system", title)), "nhp 100", &HashMap::new()),
            expect(&[("system", ExactIdentifier)]),
            "{title:?}"
        );
    }
}

#[test]
fn a_title_with_other_text_after_the_client_does_not_name_the_system() {
    for title in [
        "NHP 100 Nordhafen alt",
        "NHP 100 PRD alt",
        "NHP 100 PROD",
        "NHP 100 old",
        "NHP 100-PRD",
        "NHP 100 (PRD)",
    ] {
        // A favorite that matches the words, and no more than that.
        assert_eq!(
            rows(
                &alone(starred(entry("old", title))),
                "nhp 100",
                &HashMap::new()
            ),
            expect(&[("old", Favorite)]),
            "{title:?}"
        );
    }

    // The entry kinds still read such a title as the system; only the
    // ranking does not take it for one.
    let EntryKind::Sap(system) = kinds::classify(&entry("old", "NHP 100 Nordhafen alt"), None)
    else {
        panic!("the kinds no longer read the title as an SAP system");
    };
    assert_eq!(
        (
            system.sid.as_ref().map(Sid::as_str),
            system.client.as_deref()
        ),
        (Some("NHP"), Some("100"))
    );
}

#[test]
fn fields_name_the_system_whatever_the_title() {
    let fields = [("SAP_SID", "NHP", false), ("SAP_CLIENT", "100", false)];
    for title in [
        "Nordhafen production",
        "NHP 100 Nordhafen alt",
        "NHP 100-PRD",
        "NHQ 200 QAS",
    ] {
        assert_eq!(
            exact_ids(
                &alone(with_fields(entry("system", title), &fields)),
                "nhp 100"
            ),
            ["system"],
            "{title:?}"
        );
    }

    // Both have to come from the fields. A SID or a client from a title that
    // goes on after the client does not count, and neither does a host and
    // an instance, which make an entry an SAP system without naming it.
    for fields in [
        &[("SAP_SID", "NHP", false)][..],
        &[("SAP_CLIENT", "100", false)],
        &[
            ("SAP_HOST", "nhp.example", false),
            ("SAP_INSTANCE", "00", false),
        ],
    ] {
        let vault = alone(with_fields(entry("old", "NHP 100 Nordhafen alt"), fields));
        assert_eq!(
            rows(&vault, "nhp 100", &HashMap::new()),
            expect(&[("old", Text)]),
            "{fields:?}"
        );
    }
}

/// "NHP 100 PRD" with one open SAP field that contradicts the title. The
/// entry kinds take the other half from the title, so to them the entry is
/// `mixed`. Neither the fields alone nor the title alone say so, and the
/// title no longer says what the entry is, so no query is an exact
/// identifier for it: not `mixed`, and not "nhp 100".
fn assert_contradicted_title_names_no_system(field: (&str, &str, bool), mixed: &str) {
    let vault = alone(with_fields(entry("system", "NHP 100 PRD"), &[field]));
    assert!(
        exact_ids(&vault, mixed).is_empty(),
        "a field {field:?} and the title named {mixed:?}"
    );
    assert_eq!(
        rows(&vault, "nhp 100", &HashMap::new()),
        expect(&[("system", Text)]),
        "the title named NHP 100 against a field {field:?}"
    );
}

#[test]
fn a_sid_field_against_the_title_names_no_system() {
    // BRQ 100 to the entry kinds: the SID from the field, the client from
    // the title.
    assert_contradicted_title_names_no_system(("SAP_SID", "BRQ", false), "brq 100");
}

#[test]
fn a_client_field_against_the_title_names_no_system() {
    // NHP 200 to the entry kinds: the SID from the title, the client from
    // the field.
    assert_contradicted_title_names_no_system(("SAP_CLIENT", "200", false), "nhp 200");
}

#[test]
fn a_field_that_agrees_with_the_title_keeps_it_naming_the_system() {
    for field in [("SAP_SID", "NHP", false), ("SAP_CLIENT", "100", false)] {
        let vault = alone(with_fields(entry("system", "NHP 100 PRD"), &[field]));
        assert_eq!(exact_ids(&vault, "nhp 100"), ["system"], "{field:?}");
    }
}

#[test]
fn protected_sap_fields_do_not_make_an_identifier() {
    let nobody = HashMap::new();
    let system = |title: &str, sid_protected: bool, client_protected: bool| {
        snapshot(
            Vec::new(),
            vec![with_fields(
                entry("nordhafen", title),
                &[
                    ("SAP_SID", "NHP", sid_protected),
                    ("SAP_CLIENT", "100", client_protected),
                ],
            )],
        )
    };

    // Open fields name the system although the title does not.
    assert_eq!(
        rows(&system("Nordhafen", false, false), "nhp 100", &nobody),
        expect(&[("nordhafen", ExactIdentifier)])
    );
    for (sid_protected, client_protected) in [(true, true), (true, false), (false, true)] {
        assert!(
            rows(
                &system("Nordhafen", sid_protected, client_protected),
                "nhp 100",
                &nobody
            )
            .is_empty(),
            "SAP_SID protected: {sid_protected}, SAP_CLIENT protected: {client_protected}"
        );
    }

    // Under a title that starts with the SID and client and goes on, only
    // the fields could name the system, and protected ones do not. The
    // entry still matches as text.
    let title = "NHP 100 Nordhafen alt";
    assert_eq!(
        rows(&system(title, false, false), "nhp 100", &nobody),
        expect(&[("nordhafen", ExactIdentifier)])
    );
    for (sid_protected, client_protected) in [(true, true), (true, false), (false, true)] {
        assert_eq!(
            rows(
                &system(title, sid_protected, client_protected),
                "nhp 100",
                &nobody
            ),
            expect(&[("nordhafen", Text)]),
            "{title:?}, SAP_SID protected: {sid_protected}, SAP_CLIENT protected: {client_protected}"
        );
    }

    // A protected client does not override the one in the title either:
    // this entry is NHP 100, and "nhp 200" must not find it.
    let vault = snapshot(
        Vec::new(),
        vec![with_fields(
            entry("titled", "NHP 100"),
            &[("SAP_CLIENT", "200", true)],
        )],
    );
    assert!(rows(&vault, "nhp 200", &nobody).is_empty());
    assert_eq!(
        rows(&vault, "nhp 100", &nobody),
        expect(&[("titled", ExactIdentifier)])
    );
}

#[test]
fn a_deleted_entry_is_never_an_identifier() {
    let mut bin = group(
        "Recycle Bin",
        vec![with_fields(
            entry("deleted", "NHP 100"),
            &[("SAP_SID", "NHP", false), ("SAP_CLIENT", "100", false)],
        )],
    );
    bin.in_recycle_bin = true;
    for entry in &mut bin.entries {
        entry.in_recycle_bin = true;
    }
    let vault = VaultSnapshot {
        recycle_bin_id: Some(bin.id.clone()),
        ..snapshot(vec![bin], vec![entry("live", "NHP 100 PRD")])
    };
    assert_eq!(
        rows(&vault, "nhp 100", &HashMap::new()),
        expect(&[("live", ExactIdentifier)])
    );
}

#[test]
fn the_text_tier_keeps_the_text_order() {
    let mut vault = sample_vault();
    for id in ["nhp-old", "salesforce", "notes"] {
        set_starred(&mut vault, id, false);
    }
    let nobody = HashMap::new();
    for query in ["nh", "100", "nordhafen", "vpn", "qas"] {
        let ranked = rank(RankInput::new(&vault, query, &nobody));
        assert!(ranked.iter().all(|ranked| ranked.tier == Text), "{query:?}");
        let ranked: Vec<&str> = ranked
            .iter()
            .map(|ranked| ranked.entry.id.as_str())
            .collect();
        let by_text: Vec<&str> = ranked_entries(&vault, query)
            .into_iter()
            .map(|entry| entry.id.as_str())
            .collect();
        assert_eq!(ranked, by_text, "{query:?}");
        assert!(!ranked.is_empty(), "{query:?} should match something");
    }
}

/// The copy a kind is read from when an entry has protected fields. It must
/// hold neither a protected value nor the notes, not even for a moment, and
/// keep everything else the kinds can read.
#[test]
fn the_identifier_copy_leaves_out_notes_and_protected_values() {
    let full = with_fields(
        VaultEntry {
            notes: "the door code is 4711".to_string(),
            username: "nhp-user".to_string(),
            url: "https://nhp.example".to_string(),
            tags: vec!["vpn".to_string()],
            starred: true,
            group_path: vec!["Root".to_string(), "Nordhafen".to_string()],
            ..entry("nordhafen", "Nordhafen")
        },
        &[
            ("SAP_SID", "NHP", false),
            ("SAP_CLIENT", "100", true),
            ("Recovery code", "hunter2", true),
        ],
    );

    let view = identifier_view(&full);

    assert!(view.notes.is_empty(), "the copy carries the notes");
    assert_eq!(
        view.custom_fields,
        [CustomField {
            key: "SAP_SID".to_string(),
            value: "NHP".to_string(),
            protected: false,
        }],
        "the copy carries a protected field"
    );
    assert_eq!(
        VaultEntry {
            notes: full.notes.clone(),
            custom_fields: full.custom_fields.clone(),
            ..view
        },
        full,
        "the copy lost something other than the notes and the protected fields"
    );
}

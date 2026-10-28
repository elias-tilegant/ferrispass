use std::fs;
use std::path::Path;

use keepass::Database;
use keepass::db::fields;

use super::*;
use crate::domain::CustomField;
use crate::keepass::repository::snapshot_from_database;

/// One made-up entry and the kind it should get, written the way
/// [`summary`] prints a kind so a table reads like the rules.
struct Row {
    title: &'static str,
    fields: &'static [(&'static str, &'static str)],
    tags: &'static [&'static str],
    url: &'static str,
    expect: &'static str,
}

impl Row {
    fn entry(&self) -> VaultEntry {
        VaultEntry {
            url: self.url.to_string(),
            tags: self.tags.iter().map(|tag| tag.to_string()).collect(),
            ..entry(self.title, self.fields)
        }
    }
}

fn entry(title: &str, fields: &[(&str, &str)]) -> VaultEntry {
    VaultEntry {
        id: title.to_string(),
        title: title.to_string(),
        custom_fields: fields
            .iter()
            .map(|(key, value)| CustomField {
                key: key.to_string(),
                value: value.to_string(),
                protected: false,
            })
            .collect(),
        ..VaultEntry::default()
    }
}

fn source_name(source: Source) -> &'static str {
    match source {
        Source::Marker => "marker",
        Source::Fields => "fields",
        Source::Title => "title",
        Source::Heuristic => "heuristic",
        Source::Fallback => "fallback",
    }
}

fn or_dash(value: Option<&str>) -> &str {
    value.unwrap_or("-")
}

fn summary(kind: &EntryKind) -> String {
    match kind {
        EntryKind::Sap(sap) => {
            let role = sap.role.map(|role| match role {
                SapRole::Dev => "dev",
                SapRole::Qas => "qas",
                SapRole::Prd => "prd",
                SapRole::Sbx => "sbx",
            });
            format!(
                "sap {} sid={} client={} role={} line={}",
                source_name(sap.source),
                or_dash(sap.sid.as_ref().map(Sid::as_str)),
                or_dash(sap.client.as_deref()),
                or_dash(role),
                or_dash(sap.line.as_deref()),
            )
        }
        EntryKind::Remote(remote) => {
            let program = match &remote.program {
                None => "-".to_string(),
                Some(RemoteProgram::GlobalProtect) => "GlobalProtect".to_string(),
                Some(RemoteProgram::Cisco) => "Cisco".to_string(),
                Some(RemoteProgram::Forti) => "Forti".to_string(),
                Some(RemoteProgram::Citrix) => "Citrix".to_string(),
                Some(RemoteProgram::Workspot) => "Workspot".to_string(),
                Some(RemoteProgram::Other(name)) => format!("other:{name}"),
            };
            format!("remote {} program={program}", source_name(remote.source))
        }
        EntryKind::Web(web) => format!(
            "web {} url={}",
            source_name(web.source),
            if web.has_url { "yes" } else { "no" }
        ),
    }
}

/// Classifies every row without a marker and reports all mismatches at once.
fn check(rows: &[Row]) {
    let failures: Vec<String> = rows
        .iter()
        .filter_map(|row| {
            let got = summary(&classify(&row.entry(), None));
            (got != row.expect).then(|| {
                format!(
                    "{:?} {:?}: expected `{}`, got `{got}`",
                    row.title, row.fields, row.expect
                )
            })
        })
        .collect();
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

const HOST: (&str, &str) = ("SAP_HOST", "nhp-ci");
const INSTANCE: (&str, &str) = ("SAP_INSTANCE", "00");

#[test]
fn sap_table() {
    check(&[
        Row {
            title: "NHP 100",
            fields: &[HOST, INSTANCE],
            tags: &[],
            url: "",
            expect: "sap fields sid=NHP client=100 role=prd line=-",
        },
        Row {
            title: "Nordhafen production",
            fields: &[("SAP_SID", "NHP"), ("SAP_CLIENT", "100")],
            tags: &[],
            url: "",
            expect: "sap fields sid=NHP client=100 role=prd line=-",
        },
        // SAP_ROLE wins, here agreeing with the letter.
        Row {
            title: "NHQ 100",
            fields: &[HOST, INSTANCE, ("SAP_ROLE", "qas")],
            tags: &[],
            url: "",
            expect: "sap fields sid=NHQ client=100 role=qas line=-",
        },
        // SAP_ROLE wins against the letter D.
        Row {
            title: "NHD 100",
            fields: &[HOST, INSTANCE, ("SAP_ROLE", "PRD")],
            tags: &[],
            url: "",
            expect: "sap fields sid=NHD client=100 role=prd line=-",
        },
        // The client comes from the title when SAP_CLIENT is missing.
        Row {
            title: "BRS 200",
            fields: &[("SAP_SID", "BRS")],
            tags: &[],
            url: "",
            expect: "sap fields sid=BRS client=200 role=sbx line=-",
        },
        Row {
            title: "NHX 100",
            fields: &[("SAP_SID", "NHX")],
            tags: &[],
            url: "",
            expect: "sap fields sid=NHX client=100 role=- line=-",
        },
        Row {
            title: "NHP 100",
            fields: &[],
            tags: &[],
            url: "",
            expect: "sap title sid=NHP client=100 role=prd line=-",
        },
        // Fields incomplete, the title carries it.
        Row {
            title: "NHP 100",
            fields: &[HOST],
            tags: &[],
            url: "",
            expect: "sap title sid=NHP client=100 role=prd line=-",
        },
        Row {
            title: "EWM line",
            fields: &[("SAP_SID", "NHP"), ("SAP_LINE", "EWM")],
            tags: &[],
            url: "",
            expect: "sap fields sid=NHP client=- role=prd line=EWM",
        },
        // A field beats the title, for the SID and for the client.
        Row {
            title: "NHP 100",
            fields: &[("SAP_SID", "BRQ"), ("SAP_CLIENT", "200")],
            tags: &[],
            url: "",
            expect: "sap fields sid=BRQ client=200 role=qas line=-",
        },
        // A role that is none of the four names leaves the letter to decide.
        Row {
            title: "NHD 100",
            fields: &[("SAP_SID", "NHD"), ("SAP_ROLE", "production")],
            tags: &[],
            url: "",
            expect: "sap fields sid=NHD client=100 role=dev line=-",
        },
        // SAP_ROLE also counts when only the title made it SAP.
        Row {
            title: "NHD 100",
            fields: &[("SAP_ROLE", "PRD")],
            tags: &[],
            url: "",
            expect: "sap title sid=NHD client=100 role=prd line=-",
        },
        Row {
            title: "BRQ 200 quality",
            fields: &[],
            tags: &[],
            url: "",
            expect: "sap title sid=BRQ client=200 role=qas line=-",
        },
        Row {
            title: "NHP 100-production",
            fields: &[],
            tags: &[],
            url: "",
            expect: "sap title sid=NHP client=100 role=prd line=-",
        },
        Row {
            title: "NHP\t100",
            fields: &[],
            tags: &[],
            url: "",
            expect: "sap title sid=NHP client=100 role=prd line=-",
        },
        // A SID ending in a digit has no role.
        Row {
            title: "NH1 300",
            fields: &[],
            tags: &[],
            url: "",
            expect: "sap title sid=NH1 client=300 role=- line=-",
        },
        // The title pattern comes before the remote words.
        Row {
            title: "NHP 100",
            fields: &[],
            tags: &["VPN"],
            url: "",
            expect: "sap title sid=NHP client=100 role=prd line=-",
        },
    ]);
}

#[test]
fn remote_table() {
    check(&[
        Row {
            title: "Nordhafen GP",
            fields: &[("REMOTE_CLIENT", "GlobalProtect")],
            tags: &[],
            url: "",
            expect: "remote fields program=GlobalProtect",
        },
        // VPN gives it away but names no program.
        Row {
            title: "Nordhafen VPN",
            fields: &[],
            tags: &[],
            url: "https://vpn.example",
            expect: "remote heuristic program=-",
        },
        Row {
            title: "Citrix Brenner",
            fields: &[],
            tags: &["citrix"],
            url: "",
            expect: "remote heuristic program=Citrix",
        },
        Row {
            title: "Fortigate",
            fields: &[("REMOTE_CLIENT", "forti")],
            tags: &[],
            url: "",
            expect: "remote fields program=Forti",
        },
        Row {
            title: "Partner access",
            fields: &[("REMOTE_CLIENT", " FORTICLIENT ")],
            tags: &[],
            url: "",
            expect: "remote fields program=Forti",
        },
        // A name that is not one of the known ones is kept as written.
        Row {
            title: "Brenner desktop",
            fields: &[("REMOTE_CLIENT", "Cisco AnyConnect")],
            tags: &[],
            url: "",
            expect: "remote fields program=other:Cisco AnyConnect",
        },
        Row {
            title: "Brenner portal",
            fields: &[],
            tags: &[],
            url: "https://portal.example/GlobalProtect/login",
            expect: "remote heuristic program=GlobalProtect",
        },
        Row {
            title: "Workspot desktop",
            fields: &[],
            tags: &[],
            url: "",
            expect: "remote heuristic program=Workspot",
        },
        // A program named after VPN still names the program.
        Row {
            title: "VPN Cisco",
            fields: &[],
            tags: &[],
            url: "",
            expect: "remote heuristic program=Cisco",
        },
        Row {
            title: "cisco-vpn",
            fields: &[],
            tags: &[],
            url: "",
            expect: "remote heuristic program=Cisco",
        },
        Row {
            title: "Brenner",
            fields: &[],
            tags: &["Remote", "vpn"],
            url: "",
            expect: "remote heuristic program=-",
        },
        Row {
            title: "Nordhafen VPN",
            fields: &[],
            tags: &["Citrix gateway"],
            url: "",
            expect: "remote heuristic program=Citrix",
        },
    ]);
}

#[test]
fn web_table() {
    check(&[
        Row {
            title: "Sales Portal",
            fields: &[],
            tags: &[],
            url: "https://sales.example",
            expect: "web fallback url=yes",
        },
        Row {
            title: "Notes",
            fields: &[],
            tags: &[],
            url: "",
            expect: "web fallback url=no",
        },
        Row {
            title: "Blank URL",
            fields: &[],
            tags: &[],
            url: "   ",
            expect: "web fallback url=no",
        },
        // Lowercase is not a SID.
        Row {
            title: "nhp 100",
            fields: &[],
            tags: &[],
            url: "",
            expect: "web fallback url=no",
        },
        // Four digits are not a client.
        Row {
            title: "NHP 1000",
            fields: &[],
            tags: &[],
            url: "",
            expect: "web fallback url=no",
        },
        // A SID starts with a letter.
        Row {
            title: "1HP 100",
            fields: &[],
            tags: &[],
            url: "",
            expect: "web fallback url=no",
        },
        Row {
            title: "NHP100",
            fields: &[],
            tags: &[],
            url: "",
            expect: "web fallback url=no",
        },
        Row {
            title: "NHP 100x",
            fields: &[],
            tags: &[],
            url: "",
            expect: "web fallback url=no",
        },
        Row {
            title: "NHP 100_a",
            fields: &[],
            tags: &[],
            url: "",
            expect: "web fallback url=no",
        },
        Row {
            title: "Spaces",
            fields: &[("SAP_SID", "   ")],
            tags: &[],
            url: "https://x.example",
            expect: "web fallback url=yes",
        },
        Row {
            title: "Lowercase SID",
            fields: &[("SAP_SID", "nhp")],
            tags: &[],
            url: "",
            expect: "web fallback url=no",
        },
        Row {
            title: "Host without instance",
            fields: &[HOST, ("SAP_INSTANCE", "")],
            tags: &[],
            url: "",
            expect: "web fallback url=no",
        },
        // An empty REMOTE_CLIENT does not count.
        Row {
            title: "GitHub",
            fields: &[("REMOTE_CLIENT", "")],
            tags: &[],
            url: "https://github.com",
            expect: "web fallback url=yes",
        },
        Row {
            title: "Blank client",
            fields: &[("REMOTE_CLIENT", "  ")],
            tags: &[],
            url: "",
            expect: "web fallback url=no",
        },
        // Only whole words give remote access away.
        Row {
            title: "Myvpn",
            fields: &[],
            tags: &[],
            url: "https://myvpn.example",
            expect: "web fallback url=yes",
        },
        Row {
            title: "Fortinet portal",
            fields: &[],
            tags: &["citrix_gateway"],
            url: "",
            expect: "web fallback url=no",
        },
    ]);
}

#[test]
fn sid_rejects_lowercase_and_leading_digit() {
    assert_eq!(Sid::parse("NHP").as_ref().map(Sid::as_str), Some("NHP"));
    assert_eq!(Sid::parse("N1P").as_ref().map(Sid::as_str), Some("N1P"));
    assert_eq!(Sid::parse("B00").as_ref().map(Sid::as_str), Some("B00"));
    for invalid in [
        "nhp", "Nhp", "nHP", "1HP", "9AA", "NH", "NHPX", "", " NHP", "NHP ", "N-P", "NÄP",
    ] {
        assert_eq!(Sid::parse(invalid), None, "{invalid:?}");
    }
}

#[test]
fn sap_connection_fields_are_read() {
    let mut entry = entry(
        "Nordhafen",
        &[
            ("SAP_HOST", " nhp-ci.example "),
            ("SAP_INSTANCE", "00"),
            ("SAP_LANG", "DE"),
        ],
    );
    entry.username = "nordhafen-user".to_string();

    let EntryKind::Sap(sap) = classify(&entry, None) else {
        panic!("expected an SAP system");
    };
    assert_eq!(sap.host.as_deref(), Some("nhp-ci.example"));
    assert_eq!(sap.instance.as_deref(), Some("00"));
    assert_eq!(sap.lang.as_deref(), Some("DE"));
    assert_eq!(sap.user.as_deref(), Some("nordhafen-user"));
    assert_eq!(sap.sid, None);
    assert_eq!(sap.role, None);

    // SAP_USER overrides the username, as it does for the launcher.
    entry.custom_fields.push(CustomField {
        key: "SAP_USER".to_string(),
        value: "TECH_USER".to_string(),
        protected: false,
    });
    let EntryKind::Sap(sap) = classify(&entry, None) else {
        panic!("expected an SAP system");
    };
    assert_eq!(sap.user.as_deref(), Some("TECH_USER"));
}

#[test]
fn only_the_production_role_is_production() {
    let kind = |title| classify(&entry(title, &[]), None);
    assert!(kind("NHP 100").is_production());
    for title in ["NHD 100", "NHQ 100", "NHS 100", "NH1 100", "Sales Portal"] {
        assert!(!kind(title).is_production(), "{title}");
    }
}

#[test]
fn recognised_means_title_or_heuristic() {
    let recognised: Vec<Source> = [
        Source::Marker,
        Source::Fields,
        Source::Title,
        Source::Heuristic,
        Source::Fallback,
    ]
    .into_iter()
    .filter(|source| source.is_recognised())
    .collect();
    assert_eq!(recognised, [Source::Title, Source::Heuristic]);
}

#[test]
fn marker_names_the_kind_and_unknown_markers_are_ignored() {
    let entry = VaultEntry {
        url: "https://nhp.example".to_string(),
        ..entry("NHP 100", &[HOST, INSTANCE])
    };
    let with = |marker| summary(&classify(&entry, Some(marker)));

    assert_eq!(with("sap"), "sap marker sid=NHP client=100 role=prd line=-");
    assert_eq!(with(" Web "), "web marker url=yes");
    assert_eq!(with("REMOTE"), "remote marker program=-");
    for ignored in ["", "   ", "server", "sapgui"] {
        assert_eq!(
            with(ignored),
            "sap fields sid=NHP client=100 role=prd line=-",
            "{ignored:?}"
        );
    }
}

/// Recognition is plain data and must not reach for the UI framework. Lines
/// that are comments are skipped, so the docs can still name it.
#[test]
fn no_gpui_in_kinds() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app/kinds");
    let mut scanned = 0;
    for file in fs::read_dir(&dir).expect("read src/app/kinds") {
        let path = file.expect("directory entry").path();
        if path.file_name().is_some_and(|name| name == "tests.rs") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("read source file");
        for (number, line) in source.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            assert!(
                !line.contains("gpui"),
                "{}:{}: {line}",
                path.display(),
                number + 1
            );
        }
        scanned += 1;
    }
    assert!(scanned >= 4, "only {scanned} files under {}", dir.display());
}

/// One entry for each pair of rules that can overlap, matching both. The
/// rule numbers are the ones in the module docs; the earlier rule decides.
#[test]
fn precedence_holds_when_rules_overlap() {
    let cases = [
        (
            "1 over 2",
            Some("web"),
            Row {
                title: "Nordhafen",
                fields: &[HOST, INSTANCE],
                tags: &[],
                url: "",
                expect: "web marker url=no",
            },
        ),
        (
            "1 over 3",
            Some("sap"),
            Row {
                title: "Nordhafen",
                fields: &[("REMOTE_CLIENT", "Citrix")],
                tags: &[],
                url: "",
                expect: "sap marker sid=- client=- role=- line=-",
            },
        ),
        (
            "1 over 4",
            Some("remote"),
            Row {
                title: "NHP 100",
                fields: &[],
                tags: &[],
                url: "",
                expect: "remote marker program=-",
            },
        ),
        (
            "1 over 5",
            Some("web"),
            Row {
                title: "Citrix",
                fields: &[],
                tags: &[],
                url: "",
                expect: "web marker url=no",
            },
        ),
        (
            "2 over 3",
            None,
            Row {
                title: "Nordhafen",
                fields: &[HOST, INSTANCE, ("REMOTE_CLIENT", "GlobalProtect")],
                tags: &[],
                url: "",
                expect: "sap fields sid=- client=- role=- line=-",
            },
        ),
        (
            "2 over 4",
            None,
            Row {
                title: "NHP 100",
                fields: &[("SAP_SID", "BRQ")],
                tags: &[],
                url: "",
                expect: "sap fields sid=BRQ client=100 role=qas line=-",
            },
        ),
        (
            "2 over 5",
            None,
            Row {
                title: "Nordhafen VPN",
                fields: &[("SAP_SID", "NHP")],
                tags: &["citrix"],
                url: "",
                expect: "sap fields sid=NHP client=- role=prd line=-",
            },
        ),
        (
            "3 over 4",
            None,
            Row {
                title: "NHP 100",
                fields: &[("REMOTE_CLIENT", "Citrix")],
                tags: &[],
                url: "",
                expect: "remote fields program=Citrix",
            },
        ),
        (
            "3 over 5",
            None,
            Row {
                title: "Cisco VPN",
                fields: &[("REMOTE_CLIENT", "GlobalProtect")],
                tags: &[],
                url: "",
                expect: "remote fields program=GlobalProtect",
            },
        ),
        (
            "4 over 5",
            None,
            Row {
                title: "NHP 100 Citrix",
                fields: &[],
                tags: &["VPN"],
                url: "https://vpn.example",
                expect: "sap title sid=NHP client=100 role=prd line=-",
            },
        ),
        (
            "5 over 6",
            None,
            Row {
                title: "Sales",
                fields: &[],
                tags: &[],
                url: "https://vpn.example",
                expect: "remote heuristic program=-",
            },
        ),
    ];
    for (rules, marker, row) in cases {
        assert_eq!(
            summary(&classify(&row.entry(), marker)),
            row.expect,
            "rules {rules}"
        );
    }
}

/// A value of nothing but whitespace counts as empty in every rule. Each
/// of these would match a rule if the value were taken as it stands.
#[test]
fn whitespace_counts_as_empty() {
    check(&[
        Row {
            title: "Blank SID",
            fields: &[("SAP_SID", " \t ")],
            tags: &[],
            url: "",
            expect: "web fallback url=no",
        },
        Row {
            title: "Blank host",
            fields: &[("SAP_HOST", "   "), INSTANCE],
            tags: &[],
            url: "",
            expect: "web fallback url=no",
        },
        Row {
            title: "Blank instance",
            fields: &[HOST, ("SAP_INSTANCE", "\n")],
            tags: &[],
            url: "",
            expect: "web fallback url=no",
        },
        Row {
            title: "Blank client",
            fields: &[("REMOTE_CLIENT", "   ")],
            tags: &[],
            url: "",
            expect: "web fallback url=no",
        },
        // Around a value, whitespace is trimmed rather than rejected.
        Row {
            title: "Padded SID",
            fields: &[("SAP_SID", " NHP "), ("SAP_ROLE", " qas ")],
            tags: &[],
            url: "",
            expect: "sap fields sid=NHP client=- role=qas line=-",
        },
    ]);

    // A blank override does not hide the role the SID gives.
    let EntryKind::Sap(sap) = classify(&entry("NHP 100", &[("SAP_ROLE", "  ")]), None) else {
        panic!("expected an SAP system");
    };
    assert_eq!(sap.role, Some(SapRole::Prd));
}

/// Classifying a whole vault leaves its snapshot exactly as it was. The
/// vault is built the way `examples/make_test_vault.rs` builds one and read
/// the way the app reads a file.
#[test]
fn classification_never_writes() {
    let mut db = Database::new();
    {
        let mut root = db.root_mut();
        {
            let mut customers = root.add_group();
            customers.name = "Customers".into();
            {
                let mut sap = customers.add_entry();
                sap.set_unprotected(fields::TITLE, "NHP 100");
                sap.set_unprotected(fields::USERNAME, "nordhafen-user");
                sap.set_protected(fields::PASSWORD, "s3cret");
                sap.set_unprotected("SAP_HOST", "nhp-ci");
                sap.set_unprotected("SAP_INSTANCE", "00");
                sap.set_unprotected("SAP_ROLE", "  ");
            }
            {
                let mut remote = customers.add_entry();
                remote.set_unprotected(fields::TITLE, "Nordhafen VPN");
                remote.set_unprotected("REMOTE_CLIENT", "GlobalProtect");
                remote.tags.push("vpn".into());
            }
            let mut title_only = customers.add_entry();
            title_only.set_unprotected(fields::TITLE, "BRQ 200");
        }
        let mut web = root.add_entry();
        web.set_unprotected(fields::TITLE, "Sales Portal");
        web.set_unprotected(fields::URL, "https://sales.example");
    }
    let snapshot = snapshot_from_database(&db);
    let before = snapshot.clone();

    let kinds = classify_all(&snapshot);

    assert_eq!(snapshot, before);
    let mut by_title: Vec<(String, String)> = kinds
        .iter()
        .map(|(id, kind)| {
            let entry = snapshot
                .find_entry(id)
                .expect("classified id is in the snapshot");
            (entry.title.clone(), summary(kind))
        })
        .collect();
    by_title.sort();
    assert_eq!(
        by_title,
        [
            ("BRQ 200", "sap title sid=BRQ client=200 role=qas line=-"),
            ("NHP 100", "sap fields sid=NHP client=100 role=prd line=-"),
            ("Nordhafen VPN", "remote fields program=GlobalProtect"),
            ("Sales Portal", "web fallback url=yes"),
        ]
        .map(|(title, kind)| (title.to_string(), kind.to_string()))
    );
}

/// Every valid SID, all 33,696 of them: each parses, and the role follows the
/// last character.
#[test]
fn role_from_sid_is_total() {
    let letters = 'A'..='Z';
    let rest: Vec<char> = ('A'..='Z').chain('0'..='9').collect();
    let mut checked = 0;
    for first in letters {
        for &second in &rest {
            for &third in &rest {
                let text: String = [first, second, third].into_iter().collect();
                let sid = Sid::parse(&text).unwrap_or_else(|| panic!("{text} is a valid SID"));
                let expected = match third {
                    'D' => Some(SapRole::Dev),
                    'Q' => Some(SapRole::Qas),
                    'P' => Some(SapRole::Prd),
                    'S' => Some(SapRole::Sbx),
                    _ => None,
                };
                assert_eq!(role_from_sid(&sid), expected, "{text}");
                checked += 1;
            }
        }
    }
    assert_eq!(checked, 26 * 36 * 36);
}

/// The custom fields the rules read, in the order the property test fills
/// them.
const KEYS: [&str; 9] = [
    "SAP_HOST",
    "SAP_INSTANCE",
    "SAP_SID",
    "SAP_CLIENT",
    "SAP_ROLE",
    "SAP_LINE",
    "SAP_USER",
    "SAP_LANG",
    "REMOTE_CLIENT",
];

proptest::proptest! {
    /// Any title, URL, tag, field value or marker, multi-byte characters
    /// included: `classify` returns, and what it returns is consistent. A
    /// kind from the title has a title of that form, a SID it reports is
    /// valid, and a remote program from a field is never blank. Half the
    /// generated strings are shaped like the values the rules look for, so
    /// the slicing in the title parser sees near misses and not only noise.
    #[test]
    fn classify_never_panics(
        title in proptest::prop_oneof![
            proptest::arbitrary::any::<String>(),
            "[A-Za-z0-9\\x{c4}]{1,4}[ \\t\\x{a0}]{0,2}[0-9\\x{663}]{2,4}.{0,3}",
        ],
        url in proptest::prop_oneof![
            proptest::arbitrary::any::<String>(),
            "(https://)?[a-z]{0,6}[._/-]?(vpn|citrix|forti)?[._/-]?[a-z]{0,6}",
        ],
        tags in proptest::collection::vec(proptest::arbitrary::any::<String>(), 0..3),
        values in proptest::collection::vec(
            proptest::option::of(proptest::prop_oneof![
                proptest::arbitrary::any::<String>(),
                "[ \\t]{0,2}[A-Za-z0-9]{0,4}[ \\t]{0,2}",
            ]),
            KEYS.len(),
        ),
        marker in proptest::option::of(proptest::prop_oneof![
            proptest::arbitrary::any::<String>(),
            " ?(sap|SAP|remote|Remote|web|WEB|server) ?",
        ]),
    ) {
        let entry = VaultEntry {
            title,
            url,
            tags,
            custom_fields: KEYS
                .iter()
                .zip(values)
                .filter_map(|(key, value)| {
                    value.map(|value| CustomField {
                        key: key.to_string(),
                        value,
                        protected: false,
                    })
                })
                .collect(),
            ..VaultEntry::default()
        };

        let kind = classify(&entry, marker.as_deref());

        if kind.source() == Source::Title {
            proptest::prop_assert!(title_pattern(&entry.title).is_some());
        }
        match &kind {
            EntryKind::Sap(sap) => {
                if let Some(sid) = &sap.sid {
                    proptest::prop_assert!(Sid::parse(sid.as_str()).is_some());
                }
            }
            EntryKind::Remote(remote) => {
                if let Some(RemoteProgram::Other(name)) = &remote.program {
                    proptest::prop_assert!(!name.trim().is_empty());
                }
            }
            EntryKind::Web(_) => {}
        }
    }
}

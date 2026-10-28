use std::fs;
use std::path::Path;

use super::*;
use crate::domain::CustomField;

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

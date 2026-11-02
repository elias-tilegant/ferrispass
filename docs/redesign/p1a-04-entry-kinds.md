# P1a.04: Entry kinds

FerrisPass now works out what an entry is: an SAP system, remote access or a web login. This step only adds the logic, in `src/app/kinds`. Nothing in the app shows it yet; the entry list and the detail panel will use it for a glyph per kind and a marker on production systems, and search will use it for ranking.

Started from `rebuild` at `79b3f9d` on 2026-10-28.

| Commit | What |
|---|---|
| `c405317` | SAP systems, from their fields or a "SID client" title |
| `bc63976` | Remote access, the order of the rules, `classify_all` |
| the last one | This page, and a note on how custom fields are looked up |

## Nothing is written

The kind is worked out every time it is needed, from what the entry already holds. The functions take shared references to an entry or a vault snapshot and return plain data. Nothing in `src/app/kinds` can reach the vault file or the UI. So an entry looks the same to KeePassXC and to older versions of FerrisPass, whatever kind it is shown as.

Two tests hold this. `classification_never_writes` builds a small vault with keepass-rs, reads it the way the app reads a file, classifies every entry and checks that the snapshot is unchanged. `no_gpui_in_kinds` fails if a line under `src/app/kinds` that is not a comment mentions gpui.

## The rules

The first rule that matches decides. A field that holds only whitespace counts as empty; around a value, whitespace is trimmed.

1. **Type marker.** If the caller passes one, it wins: `sap`, `remote` or `web`, in any case. Any other value is ignored. Entries carry no marker yet, so every caller passes none.
2. **SAP fields.** `SAP_HOST` and `SAP_INSTANCE` both set, or a valid `SAP_SID`: an SAP system.
3. **`REMOTE_CLIENT`** set: remote access. GlobalProtect, Cisco, Forti (also written FortiClient), Citrix and Workspot are known names, in any case. Any other value is kept as the program name.
4. **SAP title.** The title starts with a SID, whitespace and a three-digit client, and no letter, digit or underscore follows the digits. "NHP 100", "BRQ 200 quality" and "NHP 100-production" match; "nhp 100", "NHP 1000", "1HP 100" and "NHP100" don't. As a regular expression: `^([A-Z][A-Z0-9]{2})\s+(\d{3})\b`, where `\d` means ASCII digits only.
5. **Remote words.** The title, the URL or a tag contains VPN, GlobalProtect, Cisco, Forti, FortiClient, Citrix or Workspot as a whole word, in any case. A word is a run of letters, digits and underscores, so `https://vpn.example` counts and `myvpn` doesn't. The program is the first known name found, looking at the title, then the URL, then the tags. VPN alone leaves the program open.
6. **Web login** for everything else. It records whether the entry has a URL.

A valid SID is three characters: an uppercase letter, then two uppercase letters or digits. A lowercase `SAP_SID` is not a SID.

Where rules overlap, the order decides: "NHP 100" tagged VPN is an SAP system, and so is an entry with both `SAP_SID` and `REMOTE_CLIENT`. `precedence_holds_when_rules_overlap` has one entry for each pair of rules that can overlap.

A custom field is looked up by its key, and the first field with that key counts, as in the SAP GUI launcher. An entry read from a vault never has the same key twice, because keepass-rs keeps an entry's fields in a map.

For an SAP system, whichever rule found it, the details come from:

| Detail | From |
|---|---|
| SID | `SAP_SID`, else the title |
| Client | `SAP_CLIENT`, else the title |
| Role | `SAP_ROLE` (DEV, QAS, PRD or SBX, in any case), else the last letter of the SID: D, Q, P or S. Anything else gives no role |
| Line | `SAP_LINE` |
| Host, instance, language | `SAP_HOST`, `SAP_INSTANCE`, `SAP_LANG` |
| User | `SAP_USER`, else the entry's username, the same user the launcher logs in with |

`SAP_ROLE` also counts for an entry that only its title made an SAP system, so a role someone wrote down is never overruled by the letter.

The SAP GUI launcher is unchanged. It still offers itself only for entries with a host and an instance, so an SAP system recognised by its title or its SID alone is shown as one but can't be launched from FerrisPass yet.

## What counts as recognised

Every kind carries the rule that decided it. A kind from the title (rule 4) or from the remote words (rule 5) is a guess: `Source::is_recognised` is true for those two. The UI should show such a kind as recognised rather than as something the user chose, and can offer once to keep it. A kind from a marker, from fields or from the fallback is not a guess.

## Keys

This step reads four keys that FerrisPass didn't know before: `SAP_SID`, `SAP_ROLE`, `SAP_LINE` and `REMOTE_CLIENT`. It writes none of them. Until there is an editor for them, they are added as ordinary custom fields, in FerrisPass or in KeePassXC's additional attributes.

Later steps add:

- The type marker `FerrisPass.Type`, read from the entry. Until then `classify` gets `None`.
- Servers, found by `SRV_HOST` or an `rdp://` or `ssh://` URL, and secure notes. They get their own kinds then; until then such entries are web logins.
- An editor for the SAP, remote and server keys.

## Checks

| Check | Result |
|---|---|
| `cargo fmt --all --check` | pass |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | pass (only the known future-incompat note for `block` 0.1.6) |
| `cargo test --locked` | pass, no new `#[ignore]` |
| `cargo audit` | pass, with the unmaintained and unsound warnings `rebuild` already has |
| Em and en dash check over tracked files and the commit messages in `rebuild..HEAD` | no hits |

Each mutation probe changes one production line and runs `cargo test --locked --lib app::kinds`:

| Change | Fails |
|---|---|
| `.trim()` removed from the field lookup | `whitespace_counts_as_empty`, `web_table`, `remote_table`, `sap_connection_fields_are_read`, `classify_never_panics` |
| Word boundary check after the client removed | `web_table` ("NHP 1000" became an SAP system) |
| SID accepts a lowercase first letter | `sid_rejects_lowercase_and_leading_digit` ("nHP" parsed) |
| Last letter S read as development | `role_from_sid_is_total`, `sap_table` |
| Host and instance check weakened to host or instance | `web_table`, `whitespace_counts_as_empty`, `sap_table` |
| Title rule moved after the remote words | `precedence_holds_when_rules_overlap` (rules 4 over 5), `sap_table` |

The checks ran in a checkout that shares its target directory with other branches of the same package. Cargo decides whether to rebuild from file times, so a build from another checkout can look fresh and run in its place. One gate run and one probe run did exactly that and were repeated. Every run counted here shows this checkout being compiled and its own tests running, and from then on every run touched all tracked files first.

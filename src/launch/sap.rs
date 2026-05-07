//! SAP GUI launcher backend (macOS).
//!
//! Composes a `.sapc` file with the connection string + login params,
//! then asks Launch Services to open it via `open <path>`. SAP GUI
//! for Mac registers itself as the handler for that extension, picks
//! up the params, and lands the user in a logged-in session.
//!
//! ## Field model — decomposed
//!
//! Connection metadata lives across separate custom-field keys
//! instead of one opaque connection string. Mirrors how SAP GUI
//! presents these in its own logon dialog and lets the user fill
//! them out without thinking in `/H/host/S/instance` terms:
//!
//! - `SAP_HOST`     — application server (e.g. `sap.example.com`)
//! - `SAP_INSTANCE` — 2-digit system number (e.g. `00`). The launcher
//!   prefixes `32` to get the dispatcher port (`3200`). 4-digit values
//!   (e.g. `3200`) and non-numeric values pass through unchanged for
//!   non-standard setups.
//! - `SAP_LANG`     — logon language (e.g. `DE`)
//! - `SAP_CLIENT`   — SAP client / mandant (e.g. `100`)
//! - `SAP_USER`     — *optional* username override; falls back to
//!   the entry's standard Username field. Only useful when the
//!   primary identity on the entry differs from the SAP user (e.g.
//!   email-based sign-in elsewhere, technical SAP service account
//!   here).
//! - `SAP_EXPERT`   — *optional* opt-out for `expert=true`; absent
//!   or any non-falsy value leaves it on.
//!
//! The launcher composes `/H/SAP_HOST/S/SAP_INSTANCE` itself, so the
//! user never has to type slashes or remember the prefix.
//!
//! ## Wire format
//!
//! `.sapc` body is **literal** `&`-separated key=value pairs — SAP
//! GUI does NOT URL-decode. Encoding `/` to `%2F` was the bug behind
//! "No valid host specification for connection" — SAP GUI couldn't
//! parse the conn string and fell back to picking text from the
//! file path. We now write characters as-is. Passwords containing
//! `&` or `=` aren't representable in this format; that's a SAP GUI
//! limitation, not ours, and is rare enough in practice to ignore.
//!
//! ## Process-list defence
//!
//! We pass **only** the file path to `open`, never the password as
//! an argument. The password reaches SAP GUI through the file (mode
//! 0600 in our 0700 tempdir).

use std::process::Command;

use crate::domain::{CustomField, VaultEntry};

use super::{LaunchContext, LaunchError, LaunchHandle, Launcher, TempLaunchFile};

/// Reserved custom-field keys. Conventional naming, no namespacing —
/// KeePassXC's "Additional attributes" UI shows them verbatim, so the
/// user can pick the same convention there.
pub const KEY_HOST: &str = "SAP_HOST";
pub const KEY_INSTANCE: &str = "SAP_INSTANCE";
pub const KEY_USER: &str = "SAP_USER";
pub const KEY_LANG: &str = "SAP_LANG";
pub const KEY_CLIENT: &str = "SAP_CLIENT";
/// Optional flag — accepts "false" / "0" / "no" (case-insensitive) to
/// turn off `expert=true` in the .sapc body. Default is on, matching
/// the behaviour the user requested in the example payload.
pub const KEY_EXPERT: &str = "SAP_EXPERT";

/// Keys that the "Add SAP connection" quick-add button materialises
/// in the editor, in display order. The Username override is
/// intentionally absent here — every entry already has the standard
/// Username field, and the launcher uses it by default. Power users
/// who need a different SAP user can still add SAP_USER manually.
pub const QUICK_ADD_KEYS: &[(&str, &str)] = &[
    (KEY_HOST, "sap.example.com"),
    // Placeholder is the 2-digit system number; the launcher
    // prefixes "32" automatically. Anyone with a non-standard port
    // can still type the full 4-digit value and we pass it through.
    (KEY_INSTANCE, "00"),
    (KEY_LANG, "DE"),
    (KEY_CLIENT, "100"),
];

pub static SAP_GUI_MAC: SapGuiMacLauncher = SapGuiMacLauncher;

pub struct SapGuiMacLauncher;

impl Launcher for SapGuiMacLauncher {
    fn id(&self) -> &'static str {
        "sap-gui"
    }

    fn label(&self) -> &'static str {
        "Open in SAP GUI"
    }

    fn supports(&self, entry: &VaultEntry) -> bool {
        // Both HOST and INSTANCE are needed to compose a working
        // conn string. Either alone is meaningless — surface the
        // launcher only when both are present and non-empty.
        let host = lookup(&entry.custom_fields, KEY_HOST)
            .filter(|v| !v.trim().is_empty())
            .is_some();
        let instance = lookup(&entry.custom_fields, KEY_INSTANCE)
            .filter(|v| !v.trim().is_empty())
            .is_some();
        host && instance
    }

    fn launch(&self, ctx: LaunchContext<'_>) -> Result<LaunchHandle, LaunchError> {
        let host = lookup(ctx.custom_fields, KEY_HOST)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or(LaunchError::MissingField(KEY_HOST))?;
        let instance = lookup(ctx.custom_fields, KEY_INSTANCE)
            .map(|v| resolve_instance_port(v.trim()))
            .filter(|v| !v.is_empty())
            .ok_or(LaunchError::MissingField(KEY_INSTANCE))?;
        let password = ctx.password.ok_or(LaunchError::NoPassword)?;
        let user = lookup(ctx.custom_fields, KEY_USER)
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| ctx.entry.username.clone());
        let lang = lookup(ctx.custom_fields, KEY_LANG)
            .map(|v| v.trim().to_string())
            .unwrap_or_default();
        let client = lookup(ctx.custom_fields, KEY_CLIENT)
            .map(|v| v.trim().to_string())
            .unwrap_or_default();
        let expert = expert_flag(ctx.custom_fields);

        let body = render_sapc_body(&host, &instance, &user, &lang, &client, password, expert);
        let temp_file = TempLaunchFile::create("sapc", body.as_bytes())?;

        // We don't `.wait()` — `open` returns as soon as Launch Services
        // hands the file to SAP GUI, which then spends a couple of
        // seconds parsing it and connecting. The cleanup-TTL timer
        // (`AppShell::schedule_launch_cleanup`) holds the file alive
        // until SAP GUI has had time to read it.
        Command::new("open").arg(temp_file.path()).spawn()?;

        Ok(LaunchHandle {
            temp_file: Some(temp_file),
            launcher_id: "sap-gui",
        })
    }
}

/// First-match lookup. Custom-field keys are unique per entry on
/// disk, but the snapshot is just a `Vec` — `find` is O(n) on a
/// list that's tiny in practice (typical entry has <10 custom fields).
fn lookup(fields: &[CustomField], key: &str) -> Option<String> {
    fields
        .iter()
        .find(|f| f.key == key)
        .map(|f| f.value.clone())
}

/// Translate a `SAP_INSTANCE` field value to the dispatcher port that
/// goes into `S/<port>` in the conn string.
///
/// SAP's standard convention: dispatcher port = `32 + <2-digit system
/// number>`. So for system 00 the port is 3200, for 01 it's 3201, etc.
/// The user shouldn't have to type the constant `32` every time —
/// they think in terms of the system number their SAP admin gave them.
///
/// Heuristic, in order:
/// - Exactly two ASCII digits (e.g. `"00"`, `"42"`) → prefix `32` →
///   `"3200"`, `"3242"`. Covers the 99% case.
/// - Four ASCII digits (e.g. `"3200"`) → use as-is. Lets users who
///   already stored full ports keep working, and supports non-standard
///   ports (message server `36xx`, gateway `33xx`, etc.).
/// - Anything else → use as-is. Defensive: if someone has an exotic
///   value we don't recognise, don't silently reshape it. Worst case
///   the launch fails with a clear SAP-side error rather than us
///   producing a wrong port behind their back.
fn resolve_instance_port(raw: &str) -> String {
    let is_all_digits = !raw.is_empty() && raw.chars().all(|c| c.is_ascii_digit());
    match raw.len() {
        2 if is_all_digits => format!("32{raw}"),
        _ => raw.to_string(),
    }
}

/// `SAP_EXPERT` is opt-OUT: explicit "false" / "0" / "no" turns it
/// off, anything else (including the field being absent) leaves it
/// on. The user's reference payload had `expert=true`; we keep that
/// as the default rather than silently changing behaviour.
fn expert_flag(fields: &[CustomField]) -> bool {
    let Some(value) = lookup(fields, KEY_EXPERT) else {
        return true;
    };
    !matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "false" | "0" | "no" | "off"
    )
}

/// Render the `.sapc` body. **No URL-encoding** — see module docs for
/// the bug that taught us this. SAP GUI parses literal `&`-separated
/// `key=value` pairs and would otherwise read `%2F` as a literal
/// percent-2-F instead of `/`, breaking the host parser entirely.
pub(crate) fn render_sapc_body(
    host: &str,
    instance: &str,
    user: &str,
    lang: &str,
    client: &str,
    password: &str,
    expert: bool,
) -> String {
    let mut body = format!("conn=/H/{host}/S/{instance}");
    if !user.is_empty() {
        body.push_str("&user=");
        body.push_str(user);
    }
    if !lang.is_empty() {
        body.push_str("&lang=");
        body.push_str(lang);
    }
    if !client.is_empty() {
        body.push_str("&client=");
        body.push_str(client);
    }
    body.push_str("&pass=");
    body.push_str(password);
    if expert {
        body.push_str("&expert=true");
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cf(key: &str, value: &str) -> CustomField {
        CustomField {
            key: key.into(),
            value: value.into(),
            protected: false,
        }
    }

    /// The body must be **literal** — SAP GUI doesn't URL-decode, so
    /// any percent-encoding leaks straight into the connection
    /// string. Pre-fix this test asserted percent-encoded slashes;
    /// that's exactly what broke "Open in SAP GUI" in the field
    /// ("Connection failed: No valid host specification for
    /// connection: tmp" — SAP GUI couldn't parse and fell back to
    /// reading text from the file path).
    #[test]
    fn render_sapc_body_writes_literal_chars() {
        let body = render_sapc_body(
            "sap.example.com",
            "3200",
            "alice",
            "DE",
            "100",
            "hunter2",
            true,
        );

        // Slashes must be literal — the parser keys off /H/ and /S/.
        assert!(
            body.starts_with("conn=/H/sap.example.com/S/3200"),
            "conn segment must be literal, got: {body}"
        );
        // Each known param appears exactly once with no encoding.
        assert!(body.contains("&user=alice"));
        assert!(body.contains("&lang=DE"));
        assert!(body.contains("&client=100"));
        assert!(body.contains("&pass=hunter2"));
        assert!(body.contains("&expert=true"));
        // No percent-encoded leftover from a prior implementation.
        assert!(!body.contains('%'), "no percent escapes allowed: {body}");
    }

    /// Special characters in passwords must pass through verbatim —
    /// the user's reference payload contained `^$%*` in the password
    /// and worked because SAP GUI takes it literally.
    #[test]
    fn render_sapc_body_passes_special_password_chars_through() {
        let body = render_sapc_body(
            "host",
            "00",
            "u",
            "EN",
            "",
            "Ss^i4Kcw$FeLtzS^HLET33smA%^ywi*",
            true,
        );
        assert!(
            body.contains("&pass=Ss^i4Kcw$FeLtzS^HLET33smA%^ywi*"),
            "password chars must survive: {body}"
        );
    }

    /// Empty optional params (lang, client) are dropped from the
    /// body rather than being written as `&lang=`. Some SAP versions
    /// treat an empty value differently from an absent param.
    #[test]
    fn render_sapc_body_omits_empty_optional_params() {
        let body = render_sapc_body("host", "00", "u", "", "", "p", true);
        assert!(!body.contains("lang="));
        assert!(!body.contains("client="));
        // user + pass + expert still present.
        assert!(body.contains("&user=u"));
        assert!(body.contains("&pass=p"));
        assert!(body.contains("&expert=true"));
    }

    /// Without an explicit `SAP_EXPERT=false`, `expert=true` is on —
    /// matches the user's reference payload.
    #[test]
    fn expert_defaults_on_when_field_absent() {
        let body = render_sapc_body("host", "00", "u", "DE", "100", "p", true);
        assert!(body.contains("&expert=true"));
    }

    /// Explicit opt-out via `SAP_EXPERT=false` removes the param.
    #[test]
    fn expert_can_be_disabled_via_field() {
        assert!(expert_flag(&[]));
        assert!(!expert_flag(&[cf(KEY_EXPERT, "false")]));
        assert!(!expert_flag(&[cf(KEY_EXPERT, "0")]));
        assert!(!expert_flag(&[cf(KEY_EXPERT, "No")]));
        assert!(expert_flag(&[cf(KEY_EXPERT, "yes")]));
        // Junk value falls through to default-on rather than silently
        // disabling — better to surface "looks fine to me" than to
        // turn off a flag the user had a reason to set.
        assert!(expert_flag(&[cf(KEY_EXPERT, "maybe")]));
    }

    /// Detection: both SAP_HOST AND SAP_INSTANCE must be present
    /// (and non-empty). Either alone is unactionable — we can't
    /// compose `/H/host/S/instance` without both halves.
    #[test]
    fn supports_requires_host_and_instance() {
        let mut entry = VaultEntry::default();
        assert!(!SAP_GUI_MAC.supports(&entry), "no fields → not supported");

        entry.custom_fields.push(cf(KEY_HOST, "host.example"));
        assert!(
            !SAP_GUI_MAC.supports(&entry),
            "host alone → not supported (no instance)"
        );

        entry.custom_fields.push(cf(KEY_INSTANCE, "3200"));
        assert!(
            SAP_GUI_MAC.supports(&entry),
            "host + instance → supported"
        );
    }

    /// Whitespace-only fields don't count as set — the editor leaves
    /// blank rows around for the "+" button to fill, and we don't
    /// want those tripping the launcher detection.
    #[test]
    fn supports_rejects_whitespace_only_fields() {
        let mut entry = VaultEntry::default();
        entry.custom_fields.push(cf(KEY_HOST, "   "));
        entry.custom_fields.push(cf(KEY_INSTANCE, "3200"));
        assert!(
            !SAP_GUI_MAC.supports(&entry),
            "whitespace host should not count"
        );
    }

    /// `SAP_USER` overrides the standard Username field. Lets users
    /// keep a "service account" username separate from the entry's
    /// primary identity (e.g. shared SAP technical user vs. the
    /// employee's email used elsewhere). The Quick-Add template
    /// doesn't include this row — most users want the standard
    /// Username, this is power-user territory only.
    #[test]
    fn user_override_via_sap_user_custom_field() {
        let user = lookup(&[cf(KEY_USER, "service-acct")], KEY_USER)
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "primary-name".into());
        let body = render_sapc_body("h", "00", &user, "", "", "p", false);
        assert!(body.contains("&user=service-acct"));
    }

    /// And without `SAP_USER`, the standard Username feeds through.
    #[test]
    fn user_falls_back_to_standard_username() {
        let user = lookup(&[], KEY_USER)
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "primary-name".into());
        let body = render_sapc_body("h", "00", &user, "", "", "p", false);
        assert!(body.contains("&user=primary-name"));
    }

    /// 2-digit system numbers prefix to `32xx`. This is the whole
    /// reason the user pointed out the redundancy: typing "3200"
    /// every time when only the last two digits ever vary.
    #[test]
    fn resolve_instance_port_prefixes_two_digit_input() {
        assert_eq!(resolve_instance_port("00"), "3200");
        assert_eq!(resolve_instance_port("01"), "3201");
        assert_eq!(resolve_instance_port("42"), "3242");
        assert_eq!(resolve_instance_port("99"), "3299");
    }

    /// Already-resolved 4-digit ports pass through unchanged so users
    /// who stored the full value before the heuristic existed don't
    /// have their setups silently re-mangled.
    #[test]
    fn resolve_instance_port_preserves_four_digit_input() {
        assert_eq!(resolve_instance_port("3200"), "3200");
        assert_eq!(resolve_instance_port("3601"), "3601"); // message server
        assert_eq!(resolve_instance_port("3300"), "3300"); // gateway
    }

    /// Non-numeric or otherwise non-conforming values pass through
    /// untouched. We'd rather the launch fail with a clear SAP-side
    /// error than guess the user's intent.
    #[test]
    fn resolve_instance_port_passes_unrecognised_through() {
        assert_eq!(resolve_instance_port(""), "");
        assert_eq!(resolve_instance_port("abc"), "abc");
        assert_eq!(resolve_instance_port("3"), "3");
        assert_eq!(resolve_instance_port("12345"), "12345");
        assert_eq!(resolve_instance_port("0a"), "0a");
    }

    /// QUICK_ADD_KEYS is the public contract for the editor's
    /// "+ Add SAP connection" button. The keys it lists must match
    /// the constants above (a typo would silently produce rows the
    /// launcher then ignores). Pinned here so a future refactor
    /// of the constants forces an audit of the Quick-Add list too.
    #[test]
    fn quick_add_keys_match_constants() {
        let keys: Vec<&str> = QUICK_ADD_KEYS.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec![KEY_HOST, KEY_INSTANCE, KEY_LANG, KEY_CLIENT]);
        // SAP_USER intentionally NOT in the quick-add list — the
        // standard Username field on the entry covers the typical case.
        assert!(!keys.contains(&KEY_USER));
    }
}

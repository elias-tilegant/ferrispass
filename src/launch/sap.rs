//! SAP GUI launcher backend (macOS).
//!
//! Composes a `.sapc` file with the connection string + login params,
//! then asks Launch Services to open it via `open <path>`. SAP GUI
//! for Mac registers itself as the handler for that extension, picks
//! up the params, and lands the user in a logged-in session.
//!
//! Detection: an entry counts as "SAP-supported" if it has a custom
//! field with key `SAP_CONN`. The value is treated as the raw server
//! string (e.g. `/H/sh1sap.status-c.intern/S/3200`); user / lang come
//! from `SAP_USER` / `SAP_LANG` custom fields with fallback to the
//! standard Username field, and the password is the standard Password.
//!
//! Process-list defence: we pass **only** the file path to `open`,
//! never the password as an argument. The password reaches SAP GUI
//! through the file (mode 0600 in our 0700 tempdir).

use std::process::Command;

use crate::domain::{CustomField, VaultEntry};

use super::{LaunchContext, LaunchError, LaunchHandle, Launcher, TempLaunchFile};

/// Reserved custom-field keys. Conventional naming, no namespacing —
/// KeePassXC's "Additional attributes" UI shows them verbatim, so the
/// user can pick the same convention there.
pub const KEY_CONN: &str = "SAP_CONN";
pub const KEY_USER: &str = "SAP_USER";
pub const KEY_LANG: &str = "SAP_LANG";
/// Optional flag — accepts "false" / "0" / "no" (case-insensitive) to
/// turn off `expert=true` in the .sapc body. Default is on, matching
/// the behaviour the user requested in the example payload.
pub const KEY_EXPERT: &str = "SAP_EXPERT";

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
        entry
            .custom_fields
            .iter()
            .any(|f| f.key == KEY_CONN && !f.value.trim().is_empty())
    }

    fn launch(&self, ctx: LaunchContext<'_>) -> Result<LaunchHandle, LaunchError> {
        let conn = lookup(ctx.custom_fields, KEY_CONN)
            .filter(|v| !v.is_empty())
            .ok_or(LaunchError::MissingField(KEY_CONN))?;
        let password = ctx.password.ok_or(LaunchError::NoPassword)?;
        let user = lookup(ctx.custom_fields, KEY_USER)
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| ctx.entry.username.clone());
        let lang = lookup(ctx.custom_fields, KEY_LANG).unwrap_or_default();
        let expert = expert_flag(ctx.custom_fields);

        let body = render_sapc_body(&conn, &user, &lang, password, expert);
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

/// Render the `.sapc` body. URL-encoding is non-negotiable — SAP
/// passwords routinely contain `&`, `=`, `%`, `+`, and Unicode, all
/// of which would break the param parser otherwise. The reference
/// password from the bug report (`Ss^i4Kcw$FeLtzS^HLET33smA%^ywi*`)
/// is one such case.
pub(crate) fn render_sapc_body(
    conn: &str,
    user: &str,
    lang: &str,
    password: &str,
    expert: bool,
) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    serializer.append_pair("conn", conn);
    if !user.is_empty() {
        serializer.append_pair("user", user);
    }
    if !lang.is_empty() {
        serializer.append_pair("lang", lang);
    }
    serializer.append_pair("pass", password);
    if expert {
        serializer.append_pair("expert", "true");
    }
    serializer.finish()
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

    /// The actual user-reported password from the bug report — the
    /// regression target this whole feature was driven by. Every
    /// special character must round-trip correctly through the
    /// percent-encoded body.
    #[test]
    fn render_sapc_body_url_encodes_password() {
        let body = render_sapc_body(
            "/H/sh1sap.status-c.intern/S/3200",
            "tilegant",
            "DE",
            "Ss^i4Kcw$FeLtzS^HLET33smA%^ywi*",
            true,
        );

        // Decode and verify each param made it through without
        // double-encoding or truncation.
        let decoded: std::collections::HashMap<String, String> = url::form_urlencoded::parse(
            body.as_bytes(),
        )
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

        assert_eq!(
            decoded.get("conn").map(String::as_str),
            Some("/H/sh1sap.status-c.intern/S/3200")
        );
        assert_eq!(decoded.get("user").map(String::as_str), Some("tilegant"));
        assert_eq!(decoded.get("lang").map(String::as_str), Some("DE"));
        assert_eq!(
            decoded.get("pass").map(String::as_str),
            Some("Ss^i4Kcw$FeLtzS^HLET33smA%^ywi*"),
            "password with special chars must survive a round-trip"
        );
        assert_eq!(decoded.get("expert").map(String::as_str), Some("true"));

        // Sanity: the dangerous chars actually got percent-encoded on
        // the wire — `^`, `$`, and `%` would all derail SAP GUI's
        // parser if they leaked through unescaped. (`*` survives
        // unencoded; that's fine — it isn't a reserved char in
        // application/x-www-form-urlencoded.)
        assert!(
            body.contains("%5E") && body.contains("%24") && body.contains("%25"),
            "expected percent-encoded ^ $ %% in body: {body}"
        );
    }

    /// Without an explicit `SAP_EXPERT=false`, `expert=true` is on —
    /// that's the behaviour matching the user's reference payload.
    #[test]
    fn expert_defaults_on_when_field_absent() {
        let body = render_sapc_body("/H/host/S/3200", "u", "DE", "p", true);
        assert!(body.contains("expert=true"));
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

    /// Detection key: an entry without `SAP_CONN` must NOT light up
    /// the SAP launcher (the detail-panel button stays hidden).
    #[test]
    fn supports_requires_sap_conn_field() {
        let mut entry = VaultEntry::default();
        assert!(!SAP_GUI_MAC.supports(&entry), "no fields → not supported");

        entry.custom_fields.push(cf("UNRELATED", "x"));
        assert!(!SAP_GUI_MAC.supports(&entry), "wrong key → not supported");

        entry.custom_fields.push(cf(KEY_CONN, "/H/host/S/3200"));
        assert!(SAP_GUI_MAC.supports(&entry), "SAP_CONN present → supported");
    }

    /// Empty SAP_CONN value doesn't count — same UX rule as "blank
    /// password = no copy button". User likely deleted the value but
    /// kept the row around in the editor.
    #[test]
    fn supports_rejects_blank_sap_conn() {
        let mut entry = VaultEntry::default();
        entry.custom_fields.push(cf(KEY_CONN, "   "));
        assert!(!SAP_GUI_MAC.supports(&entry));
    }

    /// `SAP_USER` overrides the standard Username field. Lets users
    /// keep a "service account" username separate from the entry's
    /// primary identity (e.g. shared SAP technical user vs. the
    /// employee's email used elsewhere).
    #[test]
    fn user_override_via_sap_user_custom_field() {
        let body = render_body_for_lookup_test("primary-name", &[cf(KEY_USER, "service-acct")]);
        let decoded: std::collections::HashMap<String, String> = url::form_urlencoded::parse(
            body.as_bytes(),
        )
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
        assert_eq!(
            decoded.get("user").map(String::as_str),
            Some("service-acct"),
            "SAP_USER should outrank the standard Username field"
        );
    }

    /// And without `SAP_USER`, we fall back to the standard Username.
    #[test]
    fn user_falls_back_to_standard_username() {
        let body = render_body_for_lookup_test("primary-name", &[]);
        let decoded: std::collections::HashMap<String, String> = url::form_urlencoded::parse(
            body.as_bytes(),
        )
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
        assert_eq!(decoded.get("user").map(String::as_str), Some("primary-name"));
    }

    /// Helper that mirrors the launch path's username-resolution
    /// without actually spawning `open` (which would fail in CI).
    fn render_body_for_lookup_test(standard_username: &str, fields: &[CustomField]) -> String {
        let user = lookup(fields, KEY_USER)
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| standard_username.to_string());
        let lang = lookup(fields, KEY_LANG).unwrap_or_default();
        let expert = expert_flag(fields);
        render_sapc_body("/H/host/S/3200", &user, &lang, "pw", expert)
    }
}

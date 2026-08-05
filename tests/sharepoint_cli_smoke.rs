//! Opt-in smoke coverage for an existing SharePoint-bound vault.
//!
//! Run with an already-open password descriptor, for example:
//! `FERRISPASS_SMOKE_VAULT=/path/to/vault.kdbx \
//!  FERRISPASS_SMOKE_PASSWORD_FD=3 cargo test --test sharepoint_cli_smoke -- --ignored 3<password.pipe`

use std::{env, process::Command};

use serde_json::Value;

fn cli() -> &'static str {
    env!("CARGO_BIN_EXE_ferrispass-cli")
}

fn smoke_vault() -> String {
    env::var("FERRISPASS_SMOKE_VAULT")
        .expect("set FERRISPASS_SMOKE_VAULT to an existing SharePoint-bound vault")
}

fn json_stdout(output: std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("CLI stdout must be JSON")
}

#[test]
#[ignore = "requires an explicitly configured live SharePoint vault and password descriptor"]
fn status_and_plan_are_safe_against_a_live_binding() {
    let vault = smoke_vault();
    let status = json_stdout(
        Command::new(cli())
            .args(["--vault", &vault, "--format", "json", "sync", "status"])
            .output()
            .expect("run sync status"),
    );
    assert_eq!(status["data"]["configured"], true);
    assert_eq!(status["data"]["network_checked"], false);

    let password_fd = env::var("FERRISPASS_SMOKE_PASSWORD_FD")
        .expect("set FERRISPASS_SMOKE_PASSWORD_FD to an inherited open descriptor");
    let plan = json_stdout(
        Command::new(cli())
            .args([
                "--vault",
                &vault,
                "--master-password-fd",
                &password_fd,
                "--format",
                "json",
                "sync",
                "now",
            ])
            .output()
            .expect("run sync plan"),
    );
    assert_eq!(plan["data"]["committed"], false);
    assert!(
        plan["data"]["plan_token"]
            .as_str()
            .is_some_and(|token| token.starts_with("v1:"))
    );
}

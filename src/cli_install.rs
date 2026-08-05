//! Registration of the CLI shipped inside the FerrisPass macOS app bundle.

use std::{env, fs, io, path::PathBuf, process::Command};

use thiserror::Error;

const CLI_NAME: &str = "ferrispass-cli";
#[cfg(target_os = "macos")]
const MACOS_LINK: &str = "/usr/local/bin/ferrispass-cli";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallStatus {
    Available { target: PathBuf },
    Installed { source: PathBuf, target: PathBuf },
    Conflict { target: PathBuf, detail: String },
    Unavailable(String),
}

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("could not locate the FerrisPass executable: {0}")]
    CurrentExecutable(#[source] io::Error),
    #[error("the bundled ferrispass-cli executable is missing at {0}")]
    MissingSource(PathBuf),
    #[error("{0}")]
    Conflict(String),
    #[error("administrator authorization was cancelled or failed: {0}")]
    Authorization(String),
    #[error("CLI registration is not supported on {0}")]
    Unsupported(&'static str),
}

pub fn status() -> InstallStatus {
    match paths().and_then(|(source, target)| inspect(source, target)) {
        Ok(status) => status,
        Err(error) => InstallStatus::Unavailable(error.to_string()),
    }
}

pub fn install() -> Result<PathBuf, InstallError> {
    let (source, target) = paths()?;
    match inspect(source.clone(), target.clone())? {
        InstallStatus::Installed { .. } => return Ok(target),
        InstallStatus::Available { .. } => {}
        InstallStatus::Conflict { detail, .. } => return Err(InstallError::Conflict(detail)),
        InstallStatus::Unavailable(detail) => return Err(InstallError::Conflict(detail)),
    }

    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "set -eu; target={}; source={}; \
             /bin/mkdir -p /usr/local/bin; \
             if [ -e \"$target\" ] || [ -L \"$target\" ]; then \
               echo 'CLI target appeared during registration' >&2; exit 73; \
             fi; \
             /bin/ln -s \"$source\" \"$target\"",
            shell_quote(&target),
            shell_quote(&source),
        );
        run_as_administrator(&script)?;
        Ok(target)
    }

    #[cfg(not(target_os = "macos"))]
    Err(InstallError::Unsupported(env::consts::OS))
}

pub fn uninstall() -> Result<PathBuf, InstallError> {
    let (source, target) = paths()?;
    match inspect(source.clone(), target.clone())? {
        InstallStatus::Installed { .. } => {}
        InstallStatus::Available { .. } => return Ok(target),
        InstallStatus::Conflict { detail, .. } => return Err(InstallError::Conflict(detail)),
        InstallStatus::Unavailable(detail) => return Err(InstallError::Conflict(detail)),
    }

    #[cfg(target_os = "macos")]
    {
        // Re-check ownership inside the privileged command. This closes the
        // race between the unprivileged inspection and removal.
        let script = format!(
            "set -eu; target={}; source={}; \
             [ -L \"$target\" ] && [ \"$(/usr/bin/readlink \"$target\")\" = \"$source\" ] || \
               {{ echo 'CLI link is no longer owned by FerrisPass' >&2; exit 73; }}; \
             /bin/rm -f \"$target\"",
            shell_quote(&target),
            shell_quote(&source),
        );
        run_as_administrator(&script)?;
        Ok(target)
    }

    #[cfg(not(target_os = "macos"))]
    Err(InstallError::Unsupported(env::consts::OS))
}

fn paths() -> Result<(PathBuf, PathBuf), InstallError> {
    let executable = env::current_exe().map_err(InstallError::CurrentExecutable)?;
    let source = executable
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(CLI_NAME);
    if !fs::symlink_metadata(&source).is_ok_and(|metadata| metadata.file_type().is_file()) {
        return Err(InstallError::MissingSource(source));
    }

    #[cfg(target_os = "macos")]
    let target = PathBuf::from(MACOS_LINK);
    #[cfg(not(target_os = "macos"))]
    return Err(InstallError::Unsupported(env::consts::OS));

    #[cfg(target_os = "macos")]
    Ok((source, target))
}

fn inspect(source: PathBuf, target: PathBuf) -> Result<InstallStatus, InstallError> {
    match fs::symlink_metadata(&target) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok(InstallStatus::Available { target })
        }
        Err(error) => Err(InstallError::Conflict(format!(
            "could not inspect {}: {error}",
            target.display()
        ))),
        Ok(metadata) if !metadata.file_type().is_symlink() => Ok(InstallStatus::Conflict {
            detail: format!(
                "{} already exists and is not a FerrisPass symlink; it was left untouched.",
                target.display()
            ),
            target,
        }),
        Ok(_) => {
            let linked = fs::read_link(&target).map_err(|error| {
                InstallError::Conflict(format!("could not read {}: {error}", target.display()))
            })?;
            if linked == source {
                Ok(InstallStatus::Installed { source, target })
            } else {
                Ok(InstallStatus::Conflict {
                    detail: format!(
                        "{} points to {} and was left untouched.",
                        target.display(),
                        linked.display()
                    ),
                    target,
                })
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn run_as_administrator(shell_script: &str) -> Result<(), InstallError> {
    let apple_script = format!(
        "do shell script \"{}\" with administrator privileges",
        apple_string(shell_script)
    );
    let output = Command::new("/usr/bin/osascript")
        .args(["-e", &apple_script])
        .output()
        .map_err(|error| InstallError::Authorization(error.to_string()))?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(InstallError::Authorization(if detail.is_empty() {
            "macOS denied the operation".into()
        } else {
            detail
        }))
    }
}

#[cfg(target_os = "macos")]
fn shell_quote(path: &std::path::Path) -> String {
    let value = path.as_os_str().to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn apple_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn inspect_recognizes_only_the_exact_owned_link() {
        let root = TempDir::new().unwrap();
        let source = root
            .path()
            .join("FerrisPass.app/Contents/MacOS/ferrispass-cli");
        let target = root.path().join("usr/local/bin/ferrispass-cli");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&source, b"cli").unwrap();

        assert!(matches!(
            inspect(source.clone(), target.clone()).unwrap(),
            InstallStatus::Available { .. }
        ));
        #[cfg(unix)]
        std::os::unix::fs::symlink(&source, &target).unwrap();
        assert!(matches!(
            inspect(source, target).unwrap(),
            InstallStatus::Installed { .. }
        ));
    }

    #[test]
    fn inspect_never_claims_a_foreign_target() {
        let root = TempDir::new().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("ferrispass-cli");
        fs::write(&source, b"cli").unwrap();
        fs::write(&target, b"foreign").unwrap();
        assert!(matches!(
            inspect(source, target).unwrap(),
            InstallStatus::Conflict { .. }
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn quoting_handles_spaces_quotes_and_applescript_metacharacters() {
        let quoted = shell_quote(std::path::Path::new("/Applications/Ferris' Pass.app/$HOME"));
        assert_eq!(quoted, "'/Applications/Ferris'\\'' Pass.app/$HOME'");
        let escaped = apple_string(r#"echo \"$HOME\" \\ done"#);
        assert_eq!(escaped, r#"echo \\\"$HOME\\\" \\\\ done"#);
    }
}

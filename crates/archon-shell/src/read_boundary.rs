//! Named credential/profile read protection. This is not complete same-user
//! isolation: report process access separately from filesystem enforcement.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "read_boundary_spawn.rs"]
mod spawn;
pub use spawn::prepare_spawn;

/// A failed launch-specific authority rule must never turn into an uncontained launch.
pub fn require_grant_rule<T>(rule: Result<T, String>) -> std::io::Result<T> {
    rule.map_err(|reason| {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            crate::write_boundary::refusal("Agent grant-protected launch", &reason),
        )
    })
}

/// Grants require kernel write containment for every agent command class.
/// A snapshot can restore data, but cannot authenticate authority while a child runs.
pub fn operator_grants_supported() -> bool {
    matches!(
        crate::write_boundary::mechanism(),
        Ok(crate::write_boundary::Mechanism::SandboxExec
            | crate::write_boundary::Mechanism::Landlock { .. })
    )
}

/// Operator stores, including shell startup files that could restore secrets.
/// Both lexical and canonical spellings are protected by platform adapters.
pub fn denied_paths(home: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = [
        ".ssh",
        ".aws",
        ".azure",
        ".local/share/keyrings",
        "Library/Keychains",
        "AppData/Roaming/Microsoft/Credentials",
        "AppData/Local/Microsoft/Credentials",
        ".kube",
        ".config/gcloud",
        ".config/gh",
        ".docker/config.json",
        ".npmrc",
        ".netrc",
        ".git-credentials",
        ".cargo/credentials",
        ".cargo/credentials.toml",
        ".profile",
        ".bash_profile",
        ".bash_login",
        ".bashrc",
        ".bash_logout",
        ".zshenv",
        ".zprofile",
        ".zshrc",
        ".zlogin",
        ".config/fish/config.fish",
        ".config/powershell",
        "Documents/PowerShell",
        "Documents/WindowsPowerShell",
    ]
    .iter()
    .map(|path| home.join(path))
    .collect();
    paths.extend(crate::credential_paths::stores_in(home));
    paths
}

pub fn operator_paths() -> Vec<PathBuf> {
    let mut paths = crate::credential_paths::protected_paths();
    for name in ["HOME", "USERPROFILE"] {
        if let Some(home) = std::env::var_os(name) {
            paths.extend(denied_paths(Path::new(&home)));
        }
    }
    // Alternative toolchain config locations can contain credentials too.
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        paths.extend([
            Path::new(&home).join("credentials"),
            Path::new(&home).join("credentials.toml"),
        ]);
    }
    paths
}

/// Add to an existing macOS sandbox profile, after all allow rules.
pub fn macos_rules(paths: &[PathBuf]) -> String {
    let mut rules = String::new();
    let mut ancestors = std::collections::HashSet::new();
    for path in paths {
        for path in path_spellings(path) {
            rules.push_str(&format!("(deny file-read* (subpath {}))\n", quoted(&path)));
            deny_macos_writes(&mut rules, &path, &mut ancestors);
        }
    }
    for path in path_spellings(&crate::credential_paths::operator_grant_root()) {
        deny_macos_writes(&mut rules, &path, &mut ancestors);
    }
    rules
}

fn quoted(path: &Path) -> String {
    format!(
        "\"{}\"",
        path.to_string_lossy()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    )
}

fn deny_macos_writes(
    rules: &mut String,
    path: &Path,
    ancestors: &mut std::collections::HashSet<PathBuf>,
) {
    rules.push_str(&format!("(deny file-write* (subpath {}))\n", quoted(path)));
    // Unlink covers ancestor removal and rename; the protected subtree keeps
    // its full write denial without preventing initial ancestor creation.
    // Preserve ancestor metadata and prevent symlink redirection while allowing mkdir.
    for ancestor in path.ancestors().skip(1) {
        if ancestors.insert(ancestor.to_path_buf()) {
            rules.push_str(&format!(
                "(deny file-write-unlink file-write-mode file-write-owner file-write-flags file-write-setugid file-write-xattr (literal {0}))\n\
                 (deny file-write-create (require-all (literal {0}) (vnode-type SYMLINK)))\n",
                quoted(ancestor)
            ));
        }
    }
}

// Resolve the longest existing prefix too: an atomic sibling may not exist
// yet, while its parent is reached through an operator-selected symlink.
fn path_spellings(path: &Path) -> Vec<PathBuf> {
    let mut paths = vec![path.to_path_buf()];
    let mut prefix = path;
    let mut rest = Vec::new();
    while prefix.symlink_metadata().is_err() {
        let (Some(parent), Some(name)) = (prefix.parent(), prefix.file_name()) else {
            return paths;
        };
        rest.push(name.to_os_string());
        prefix = parent;
    }
    if let Ok(mut real) = prefix.canonicalize() {
        real.extend(rest.into_iter().rev());
        if !paths.contains(&real) {
            paths.push(real);
        }
    }
    paths
}

/// Compose the host's read denials into a host-owned macOS write profile.
/// No program-name test can establish enforcement.
#[cfg(target_os = "macos")]
pub fn macos_command(program: &OsStr, profile: &str) -> Command {
    let mut command = crate::spawn::command(crate::write_boundary::SANDBOX_EXEC);
    command
        .args([
            "-p",
            &format!("{profile}{}", macos_rules(&operator_paths())),
        ])
        // A caller-selected program beginning with '-' is not a sandbox option.
        .arg("--")
        .arg(program);
    tracing::info!(
        credential_reads = "named-paths-denied",
        process_reads = "not-contained",
        "agent command launch with host-applied profile"
    );
    command
}

/// Build a child under the supported read mechanism. Unsupported containment
/// is explicit in a launch event; environment isolation remains in force.
pub fn command(program: &OsStr) -> Command {
    #[cfg(target_os = "macos")]
    if matches!(
        crate::write_boundary::mechanism(),
        Ok(crate::write_boundary::Mechanism::SandboxExec)
    ) {
        return macos_command(program, "(version 1)(allow default)");
    }
    #[allow(unused_mut)]
    let mut command = crate::spawn::command(program);
    #[cfg(target_os = "linux")]
    {
        let reads =
            match crate::write_boundary::landlock::LandlockSandbox::build_read(&operator_paths()) {
                Ok(boundary) => {
                    boundary.install_std(&mut command);
                    "named-paths-denied"
                }
                Err(_) => "not-contained",
            };
        let writes = match crate::write_boundary::landlock::LandlockSandbox::build(
            &[crate::credential_paths::operator_grant_root()],
            &[],
            &[],
        ) {
            Ok(boundary) => {
                boundary.install_std(&mut command);
                "denied"
            }
            Err(reason) if operator_grants_supported() => {
                crate::write_boundary::refusal("Agent launch", &reason);
                use std::os::unix::process::CommandExt;
                // SAFETY: fail the child before exec without allocating or touching host state.
                unsafe {
                    command.pre_exec(|| Err(std::io::Error::from_raw_os_error(libc::EPERM)));
                }
                "launch-refused"
            }
            Err(_) => "not-contained",
        };
        tracing::warn!(
            credential_reads = reads,
            host_grant_writes = writes,
            process_reads = "not-contained",
            operator_env_grants = if operator_grants_supported() {
                "enabled"
            } else {
                "disabled: kernel write containment unavailable"
            },
            "agent command launch: process read isolation unavailable"
        );
        command
    }
    #[cfg(not(target_os = "linux"))]
    {
        tracing::warn!(
            host_grant_writes = "not-contained",
            credential_reads = "not-contained",
            process_reads = "not-contained",
            operator_env_grants = "disabled: kernel write containment unavailable",
            "agent command launch: filesystem/process read isolation unavailable on this host"
        );
        command
    }
}

#[cfg(test)]
#[path = "read_boundary_tests.rs"]
mod tests;

#[cfg(test)]
mod r5_tests {
    #[test]
    fn r5_failed_launch_grant_rule_is_refused() {
        let error = super::require_grant_rule::<()>(Err(
            "injected launch-specific rule construction failure".into(),
        ))
        .expect_err("a failed grant rule must refuse launch");
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("It was NOT run"));
    }
}

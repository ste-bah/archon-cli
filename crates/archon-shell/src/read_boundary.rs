//! Named credential/profile read protection. This is not complete same-user
//! isolation: report process access separately from filesystem enforcement.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    for path in paths {
        for path in path_spellings(path) {
            let quoted = path
                .to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"");
            rules.push_str(&format!("(deny file-read* (subpath \"{quoted}\"))\n"));
        }
    }
    for path in path_spellings(&crate::credential_paths::operator_grant_root()) {
        let quoted = |p: &Path| {
            p.to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        };
        rules.push_str(&format!(
            "(deny file-write* (subpath \"{}\"))\n",
            quoted(&path)
        ));
        for ancestor in path.ancestors().skip(1) {
            rules.push_str(&format!(
                "(deny file-write* (literal \"{}\"))\n",
                quoted(ancestor)
            ));
        }
    }
    rules
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
mod tests {
    use super::*;

    #[test]
    fn unavailable_boundary_does_not_claim_enforcement_for_a_program_name() {
        if crate::write_boundary::mechanism().is_ok() {
            return;
        }
        #[derive(Clone)]
        struct Capture(std::sync::Arc<std::sync::Mutex<bool>>);
        impl tracing::Subscriber for Capture {
            fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
                true
            }
            fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
                tracing::span::Id::from_u64(1)
            }
            fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
            fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
            fn event(&self, event: &tracing::Event<'_>) {
                struct Fields<'a>(&'a std::sync::Mutex<bool>);
                impl tracing::field::Visit for Fields<'_> {
                    fn record_debug(
                        &mut self,
                        field: &tracing::field::Field,
                        value: &dyn std::fmt::Debug,
                    ) {
                        if field.name() == "credential_reads"
                            && format!("{value:?}").contains("named-paths-denied")
                        {
                            *self.0.lock().unwrap() = true;
                        }
                    }
                }
                event.record(&mut Fields(&self.0));
            }
            fn enter(&self, _: &tracing::span::Id) {}
            fn exit(&self, _: &tracing::span::Id) {}
        }
        let claimed = std::sync::Arc::new(std::sync::Mutex::new(false));
        tracing::subscriber::with_default(Capture(claimed.clone()), || {
            let _ = command(OsStr::new(crate::write_boundary::SANDBOX_EXEC));
        });
        assert!(
            !*claimed.lock().unwrap(),
            "host did not enforce any read policy"
        );
    }

    #[test]
    fn relocated_host_credentials_and_atomic_siblings_are_protected() {
        const OVERRIDE: &str = "ARCHON_CREDENTIALS_FILE";
        if std::env::var_os("ARCHON_R2_PATH_PROBE").is_none() {
            let temp = tempfile::tempdir().unwrap();
            let status = crate::spawn::command(std::env::current_exe().unwrap())
                .args(["--exact", "read_boundary::tests::relocated_host_credentials_and_atomic_siblings_are_protected"])
                .env("ARCHON_R2_PATH_PROBE", "1")
                .env(OVERRIDE, temp.path().join("relocated.json"))
                .status().unwrap();
            assert!(
                status.success(),
                "relocated credential store must be denied"
            );
            return;
        }
        let relocated = PathBuf::from(std::env::var_os(OVERRIDE).unwrap());
        let paths = operator_paths();
        assert!(
            paths.contains(&relocated),
            "host credential override must be protected"
        );
        assert!(
            paths.contains(&relocated.with_extension("json.tmp")),
            "atomic sibling must be protected"
        );
    }
    #[test]
    fn credential_and_profile_paths_are_denied_by_absolute_path() {
        let home = Path::new("/synthetic-operator");
        let paths = denied_paths(home);
        for name in [
            ".archon/.credentials.json",
            ".aws",
            ".ssh",
            ".profile",
            ".bashrc",
            ".zshenv",
            ".cargo/credentials.toml",
            ".codex/auth.json",
        ] {
            assert!(paths.contains(&home.join(name)));
        }
        let rules = macos_rules(&paths);
        assert!(rules.contains("(deny file-read* (subpath \"/synthetic-operator/.aws\"))"));
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn credential_file_is_unreadable_when_kernel_boundary_is_available() {
        if crate::write_boundary::mechanism().is_err() {
            eprintln!(
                "kernel read probe unavailable in this host sandbox; profile/path policy is tested separately"
            );
            return;
        }
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join(".aws")).unwrap();
        let secret = home.path().join(".aws/credentials");
        std::fs::write(&secret, "sk-test-file-355").unwrap();
        #[cfg(target_os = "linux")]
        let mut command = crate::spawn::command("/bin/sh");
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = crate::spawn::command(crate::write_boundary::SANDBOX_EXEC);
            command.args([
                "-p",
                &format!(
                    "(version 1)(allow default){}",
                    macos_rules(&denied_paths(home.path()))
                ),
                "/bin/sh",
            ]);
            command
        };
        #[cfg(target_os = "linux")]
        crate::write_boundary::landlock::LandlockSandbox::build_read(&denied_paths(home.path()))
            .unwrap()
            .install_std(&mut command);
        let status = command
            .args(["-c", "test ! -r \"$1\" || ! cat \"$1\" >/dev/null", "probe"])
            .arg(&secret)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "credential read must be denied");
    }
}

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

//! Named credential/profile read protection. This is not complete same-user
//! isolation: report process access separately from filesystem enforcement.
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "read_boundary_report.rs"]
mod launch_report;
#[path = "read_boundary_spawn.rs"]
mod spawn;
pub use launch_report::LaunchReport;
#[cfg(any(target_os = "linux", test))]
#[path = "read_boundary_status_record.rs"]
pub(crate) mod status_record;
pub use spawn::prepare_spawn;

#[path = "read_boundary_config.rs"]
mod config;
#[cfg(any(target_os = "linux", target_os = "macos", all(test, unix)))]
#[path = "read_boundary_location.rs"]
mod location;

#[cfg(target_os = "linux")]
pub use config::prepared_write_paths;

/// A failed launch-specific authority rule must never turn into an uncontained launch.
pub fn require_grant_rule<T>(rule: Result<T, String>) -> std::io::Result<T> {
    rule.map_err(|reason| {
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            crate::write_boundary::refusal("Agent grant-protected launch", &reason),
        )
    })
}

/// Grants require kernel read and write containment for every agent command class.
/// A snapshot can restore data, but cannot authenticate authority while a child runs.
pub fn operator_grants_supported() -> bool {
    match crate::write_boundary::mechanism() {
        Ok(crate::write_boundary::Mechanism::SandboxExec) => true,
        #[cfg(target_os = "linux")]
        Ok(crate::write_boundary::Mechanism::Landlock { .. }) => {
            crate::write_boundary::landlock::LandlockSandbox::build_read(&operator_paths()).is_ok()
        }
        _ => false,
    }
}

/// Credential stores; startup files receive separate write-only protection.
/// Both lexical and canonical spellings are protected by platform adapters.
pub fn denied_paths(home: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = [
        ".ssh",
        ".aws",
        ".azure",
        ".local/share/keyrings",
        #[cfg(target_os = "macos")]
        "Library/Keychains",
        #[cfg(windows)]
        "AppData/Roaming/Microsoft/Credentials",
        #[cfg(windows)]
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
    ]
    .iter()
    .map(|path| home.join(path))
    .collect();
    paths.extend(crate::credential_paths::stores_in(home));
    paths
}

pub fn operator_paths() -> Vec<PathBuf> {
    let mut paths = crate::credential_paths::protected_paths();
    paths.extend(denied_paths(&crate::credential_paths::home()));
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
    paths.sort();
    paths.dedup();
    paths
}

/// Add to an existing macOS sandbox profile, after all allow rules.
pub fn macos_rules(paths: &[PathBuf]) -> String {
    let grant_root = crate::credential_paths::operator_grant_root();
    #[cfg(target_os = "macos")]
    prepare_macos_ancestors(&crate::credential_paths::home(), paths, &grant_root);
    let mut rules = macos_rules_for(paths, &grant_root);
    let writes = config::write_paths();
    #[cfg(target_os = "macos")]
    prepare_macos_ancestors(&crate::credential_paths::home(), &writes, &grant_root);
    let mut ancestors = std::collections::HashSet::new();
    for path in writes.iter().flat_map(|path| path_spellings(path)) {
        deny_macos_writes(&mut rules, &path, &mut ancestors);
    }
    rules
}

fn macos_rules_for(paths: &[PathBuf], grant_root: &Path) -> String {
    let mut rules = String::new();
    let mut ancestors = std::collections::HashSet::new();
    for path in paths {
        for path in path_spellings(path) {
            rules.push_str(&format!("(deny file-read* (subpath {}))\n", quoted(&path)));
            deny_macos_writes(&mut rules, &path, &mut ancestors);
        }
    }
    for path in path_spellings(grant_root) {
        deny_macos_writes(&mut rules, &path, &mut ancestors);
    }
    rules
}

// Materialize only ancestors, never credential files or protected subtrees.
// Use individual mkdir calls so an absent HOME (or anything above it) is not created.
#[cfg(any(target_os = "macos", all(test, unix)))]
fn prepare_macos_ancestors(home: &Path, paths: &[PathBuf], grant_root: &Path) {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let mut directories = std::collections::BTreeSet::new();
    for path in paths.iter().map(PathBuf::as_path).chain([grant_root]) {
        directories.extend(
            path.ancestors()
                .skip(1)
                .filter(|ancestor| !home.starts_with(ancestor))
                .map(Path::to_path_buf),
        );
    }
    directories.insert(grant_root.to_path_buf());
    let mut failure = None;
    for directory in directories {
        if !directory.exists() && location::inside_work(&directory) {
            tracing::warn!(path = %directory.display(), "absent path not prepared (inside work tree)");
            continue;
        }
        let result = match std::fs::DirBuilder::new().mode(0o700).create(&directory) {
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if directory.is_dir() {
                    Ok(())
                } else {
                    Err(error)
                }
            }
            result => result,
        };
        let result = result.and_then(|()| {
            if directory == grant_root {
                std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            } else {
                Ok(())
            }
        });
        if let Err(error) = result {
            failure.get_or_insert((directory, error));
        }
    }
    if let Some((path, error)) = failure {
        tracing::warn!(path = %path.display(), %error,
            "could not prepare read-boundary ancestors; full write denials retained");
    }
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
    // The host creates missing ancestors before profile construction; only an
    // existing directory is safe from rename-into forgery.
    // Unlink covers ancestor removal and rename; the protected subtree keeps
    // its full write denial without preventing initial ancestor creation.
    // Preserve ancestor metadata and prevent symlink redirection while allowing mkdir.
    for ancestor in path.ancestors().skip(1) {
        if ancestors.insert(ancestor.to_path_buf()) {
            rules.push_str(&format!(
                "(deny file-write* (literal {}))\n",
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
    if let Err(reason) = crate::spawn::initialize_host_tools() {
        tracing::warn!(%reason, "host configuration unavailable; retaining launch behavior");
    }
    let mut command = crate::spawn::command(crate::write_boundary::SANDBOX_EXEC);
    command
        .args([
            "-p",
            &format!("{profile}{}", macos_rules(&operator_paths())),
        ])
        // A caller-selected program beginning with '-' is not a sandbox option.
        .arg("--")
        .arg(crate::spawn::host_program(program));
    tracing::info!(
        credential_reads = "named-paths-denied",
        operator_config_writes = "denied",
        process_reads = "not-contained",
        "agent command launch with host-applied profile"
    );
    command
}

/// Build a child under the supported read mechanism. Unsupported containment
/// is explicit in a launch event; environment isolation remains in force.
pub fn command(program: &OsStr) -> (Command, LaunchReport) {
    command_with_report(program)
}

/// The report reads optional operation outcomes when queried after spawn.
/// Required rule construction failures retain the existing launch refusal.
pub fn command_with_report(program: &OsStr) -> (Command, LaunchReport) {
    let initialization = crate::spawn::initialize_host_tools();
    let (command, boundary) = boundary_command(program);
    match initialization {
        Ok(()) => (command, boundary),
        Err(reason) => {
            let mut boundary = boundary;
            boundary.prepared.push_str(&format!(
                " preparation: host configuration unavailable ({reason})"
            ));
            tracing::warn!(%boundary);
            (command, boundary)
        }
    }
}

fn boundary_command(program: &OsStr) -> (Command, LaunchReport) {
    #[cfg(target_os = "macos")]
    if matches!(
        crate::write_boundary::mechanism(),
        Ok(crate::write_boundary::Mechanism::SandboxExec)
    ) {
        return (
            macos_command(program, "(version 1)(allow default)"),
            report("named-paths-denied", "named-paths-denied").into(),
        );
    }
    #[allow(unused_mut)]
    let mut command = crate::spawn::command(program);
    #[cfg(target_os = "linux")]
    {
        let active = matches!(
            crate::write_boundary::mechanism(),
            Ok(crate::write_boundary::Mechanism::Landlock { .. })
        );
        let writes = install_operator_seals(&mut command, active);
        let grants = install_linux_boundary(
            &mut command,
            crate::write_boundary::landlock::LandlockSandbox::build(
                &[crate::credential_paths::operator_grant_root()],
                &[],
                &[],
            ),
            active,
            "denied",
        );
        let reads = install_linux_boundary(
            &mut command,
            crate::write_boundary::landlock::LandlockSandbox::build_read(&operator_paths()),
            active,
            "named-paths-denied",
        );
        tracing::warn!(
            credential_reads = reads,
            host_grant_writes = %writes,
            process_reads = "not-contained",
            operator_env_grants =
                if active && reads == "named-paths-denied" && grants == "denied" {
                    "enabled"
                } else {
                    "disabled: kernel read/write containment unavailable"
                },
            "agent command launch: process read isolation unavailable"
        );
        let reads = if grants.starts_with("launch-refused") {
            grants
        } else {
            reads
        };
        (
            command,
            LaunchReport {
                prepared: report(&reads, &writes.prepared),
                seals: writes.seals,
            },
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        tracing::warn!(
            host_grant_writes = "not-contained",
            credential_reads = "not-contained",
            process_reads = "not-contained",
            operator_env_grants = "disabled: kernel read/write containment unavailable",
            "agent command launch: filesystem/process read isolation unavailable on this host"
        );
        (command, report("not-contained", "not-contained").into())
    }
}

/// Keep full launch diagnostics in logs and attach them to observations only
/// when filesystem containment is incomplete or preparation refused launch.
pub fn observation_note(report: impl ToString) -> Option<String> {
    let report = report.to_string();
    (report.contains("launch-refused")
        || report.contains("Filesystem read boundary: not-contained")
        || report.contains("writes: not-contained")
        || report.contains("mount-sealed; bypassable")
        || report.contains("preparation:")
        || report.contains("failed step")
        || report.contains("bypassable")
        || report.contains("all optional seals absent")
        || report.contains("seal outcome unknown")
        || report.contains("seal outcome pending"))
    .then_some(report)
}

#[cfg(target_os = "linux")]
const INACTIVE_MOUNT_WRITES: &str = "mount-sealed; bypassable through /proc/<pid>/{root,cwd,fd} or ptrace of same-user host processes without Landlock";

fn report(reads: &str, writes: &str) -> String {
    #[cfg(windows)]
    let limitation = " Windows Startup folders and registry Run keys are not contained by file rules on this host.";
    #[cfg(target_os = "macos")]
    let limitation = " macOS launchctl submit and shell configuration directories (including ZDOTDIR) are not-contained by these file rules.";
    #[cfg(target_os = "linux")]
    let limitation = " Linux process-based autostart and metadata changes are not contained. Absent startup files are not sealed, including dangling startup links. When installed, private read-only mounts seal existing protected files and directories; their ancestor mounts prevent renaming sealed paths; renames and hard links between different sealed-path ancestor directories, or between one of them and any other directory (including $TMPDIR on the same filesystem), return EXDEV. This affects git clone --local and cross-mount renames into the repo; registered repository configuration writes (submodule init, remote add, upstream tracking) fail with EBUSY. When seals are installed, set-user-ID and file-capability programs do not gain privileges. Absent credential locations (including .ssh and .npmrc) are not sealed.";
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    let limitation = "";
    let report = format!(
        "Filesystem read boundary: {reads}; operator config/autostart writes: {writes}; process reads: not-contained.{limitation}"
    );
    let report = format!(
        "{report} Host programs use absolute locations captured from startup PATH; empty and relative entries are skipped. PATH directories are not sealed."
    );
    let report = if writes.starts_with("named-paths-denied") {
        format!(
            "{report} Successfully prepared existing registered repository and operator configuration files are write-sealed; unprepared paths listed in preparation diagnostics are not. Host repository commands use a startup global-configuration snapshot with execution overrides and disabled system configuration."
        )
    } else {
        report
    };
    let report = if reads == "not-contained" {
        format!("{report} Running with an unbounded filesystem read surface.")
    } else {
        report
    };
    if reads == "not-contained" || writes == "not-contained" {
        tracing::warn!(boundary = %report, "command runs without filesystem containment");
    } else {
        tracing::info!(boundary = %report, "command boundary prepared");
    }
    report
}

#[cfg(target_os = "linux")]
fn install_linux_boundary(
    command: &mut Command,
    boundary: Result<crate::write_boundary::landlock::LandlockSandbox, String>,
    active: bool,
    contained: &'static str,
) -> String {
    match boundary {
        Ok(boundary) => {
            boundary.install_std(command);
            contained.to_owned()
        }
        Err(_) if !active => "not-contained".to_owned(),
        Err(reason) => {
            tracing::error!(%reason, "Boundary launch refused: required filesystem boundary could not be prepared; the command will not execute");
            use std::os::unix::process::CommandExt;
            // SAFETY: fail the child before exec without allocating or touching host state.
            unsafe {
                command.pre_exec(|| Err(std::io::Error::from_raw_os_error(libc::EPERM)));
            }
            format!("launch-refused: {reason}; It was NOT run")
        }
    }
}

#[cfg(target_os = "linux")]
#[path = "read_boundary_seals.rs"]
mod seals;
#[cfg(target_os = "linux")]
pub use seals::{install_operator_seals, install_prepared_seals};

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

#[cfg(all(test, target_os = "linux"))]
#[path = "read_boundary_kernel_tests.rs"]
mod kernel_tests;

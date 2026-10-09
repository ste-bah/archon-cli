//! Conservative acceptance-check verdict reuse. Reuse is allowed only when
//! the host can name every input the check can read.

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Assessment {
    pub(super) reusable: bool,
    pub(super) reason: &'static str,
}

/// Current host logic that judges acceptance checks.
pub(crate) fn logic_identity() -> Option<(u32, String, String)> {
    let versions = crate::command::workflow_host_command_logic::versions();
    let digests = crate::command::workflow_host_command_logic::digests();
    Some((
        versions.get("freeze-acceptance").copied()?,
        digests.get("freeze-acceptance")?.0.clone(),
        crate::command::workflow_host_command_logic::THIS_BUILD.to_string(),
    ))
}

/// The shell that evaluates the literal `test` predicate is part of the
/// execution environment even when its path and PATH variable stay constant.
pub(crate) fn shell_binary_digest(path: Option<&str>) -> Option<String> {
    let paths = path.map(std::ffi::OsStr::new)?;
    shell_identity_for(
        archon_shell::resolve_posix_shell(),
        &std::env::split_paths(paths).collect::<Vec<_>>(),
    )
}

/// Resolve the selected shell as the process launcher will and commit to its
/// location, executable mode, and bytes. An unknown identity makes the caller
/// volatile instead of allowing a verdict to survive an unproven shell.
fn shell_identity_for(
    selected: &std::path::Path,
    search_paths: &[std::path::PathBuf],
) -> Option<String> {
    let candidate = if selected.is_absolute() {
        selected.to_path_buf()
    } else if selected.components().count() == 1 {
        search_paths
            .iter()
            .map(|directory| directory.join(selected))
            .find(|path| executable_file(path))?
    } else {
        return None;
    };
    let shell = candidate.canonicalize().ok()?;
    let metadata = std::fs::metadata(&shell).ok()?;
    if !metadata.is_file() || !executable_metadata(&metadata) {
        return None;
    }
    let bytes = std::fs::read(&shell).ok()?;
    let path = shell.to_str()?;
    Some(super::content_digest(
        serde_json::json!([
            path,
            executable_mode(&metadata),
            super::content_digest(&bytes)
        ])
        .to_string()
        .as_bytes(),
    ))
}

fn executable_file(path: &std::path::Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    metadata.is_file() && executable_metadata(&metadata)
}

#[cfg(unix)]
fn executable_metadata(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable_metadata(_metadata: &std::fs::Metadata) -> bool {
    true
}

#[cfg(unix)]
fn executable_mode(metadata: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn executable_mode(_metadata: &std::fs::Metadata) -> u32 {
    0
}

/// Only one literal repository-relative filesystem predicate has a bounded
/// read set. Shell, command substitution, environment expansion, and arbitrary
/// executables are volatile because their reads cannot be established here.
pub(crate) fn bounded_path(check: &str) -> Option<&str> {
    let words: Vec<&str> = check.split_whitespace().collect();
    match words.as_slice() {
        ["test", "-f" | "-e" | "-d" | "-s", path]
            if !path.starts_with('/')
                && !path.split('/').any(|part| part == "..")
                && !path.starts_with('-')
                && path.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/')
                })
                && !path.is_empty() =>
        {
            Some(path)
        }
        _ => None,
    }
}

pub(super) fn assess(check: &str, tree: &str, logic: &str, environment: &str) -> Assessment {
    if bounded_path(check).is_none() {
        return Assessment {
            reusable: false,
            reason: "check read closure is unbounded",
        };
    }
    if tree.is_empty() {
        return Assessment {
            reusable: false,
            reason: "repository tree digest is unavailable",
        };
    }
    if logic.is_empty() {
        return Assessment {
            reusable: false,
            reason: "host logic version is unavailable",
        };
    }
    if environment.is_empty() {
        return Assessment {
            reusable: false,
            reason: "environment inputs are unavailable",
        };
    }
    Assessment {
        reusable: true,
        reason: "all inputs are host-proven identical",
    }
}

/// Stable key over check bytes, repository tree digest, judge logic and the
/// complete environment supplied to the check.
pub(super) fn reuse_key(check: &str, tree: &str, logic: &str, environment: &str) -> String {
    super::content_digest(
        serde_json::json!([check.as_bytes(), tree, logic, environment])
            .to_string()
            .as_bytes(),
    )
}

#[cfg(test)]
#[path = "workflow_acceptance_check_reuse_tests.rs"]
mod tests;

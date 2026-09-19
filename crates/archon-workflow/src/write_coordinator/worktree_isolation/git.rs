//! Low-level `git` process helpers for the write coordinator.
//!
//! `run_git` applies NO commit-only flags (`--no-gpg-sign` / `--no-verify`) and
//! sets the working directory via `Command::current_dir`, never a `-C` prepend.
//! Callers that need commit-only flags add them explicitly to `args`.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use super::IsolationError;

pub(crate) fn run_git(args: &[&str], cwd: &Path) -> Result<Output, IsolationError> {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .map_err(spawn_error)?;
    check(output)
}

pub(crate) fn run_git_with_stdin(
    args: &[&str],
    cwd: &Path,
    stdin: &[u8],
) -> Result<Output, IsolationError> {
    check(raw_git_with_stdin(args, cwd, stdin)?)
}

fn raw_git_with_stdin(args: &[&str], cwd: &Path, stdin: &[u8]) -> Result<Output, IsolationError> {
    use std::io::Write;

    let mut child = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(spawn_error)?;
    child
        .stdin
        .take()
        .ok_or_else(|| IsolationError::ProcessFailed {
            stderr: "git stdin unavailable".into(),
        })?
        .write_all(stdin)?;
    Ok(child.wait_with_output()?)
}

/// The subset of `paths` (root-relative) that the repository at `cwd` ignores
/// and does not track: `git check-ignore` never reports a tracked file, so a
/// tracked file matched by an ignore pattern is not returned. Exit status 1 is
/// git's own "nothing ignored", an answer rather than a failure.
pub(crate) fn check_ignore(cwd: &Path, paths: &[String]) -> Result<Vec<String>, IsolationError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let mut stdin = Vec::new();
    for path in paths {
        stdin.extend_from_slice(path.as_bytes());
        stdin.push(0);
    }
    let output = raw_git_with_stdin(&["check-ignore", "--stdin", "-z"], cwd, &stdin)?;
    if !matches!(output.status.code(), Some(0 | 1)) {
        return Err(IsolationError::ProcessFailed {
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|raw| !raw.is_empty())
        .map(|raw| String::from_utf8_lossy(raw).into_owned())
        .filter(|found| paths.contains(found))
        .collect())
}

fn spawn_error(err: std::io::Error) -> IsolationError {
    if err.kind() == std::io::ErrorKind::NotFound {
        IsolationError::GitMissing
    } else {
        IsolationError::Io(err)
    }
}

fn check(output: Output) -> Result<Output, IsolationError> {
    if output.status.success() {
        Ok(output)
    } else {
        Err(IsolationError::ProcessFailed {
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

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
    let output = child.wait_with_output()?;
    check(output)
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

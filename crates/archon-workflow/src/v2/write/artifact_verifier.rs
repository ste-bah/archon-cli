//! A write branch's declared artifact verifiers, run on the host.
//!
//! The verifiers are the task set's own commands (`artifact_verification_
//! commands` on the item), fed to the POSIX shell in the branch's worktree.
//! Batch G ran them under the project-input tripwire; Batch G2 supervises
//! them like every other host-run command -- a wall clock, the whole process
//! group killed when it runs out or the command ends, bounded output -- and
//! keeps what the ENVIRONMENT did out of the branch's verdict:
//!
//! - a verifier that exits non-zero is the branch's failure, as before;
//! - one that could not be started or waited on, did not finish in time, or
//!   ran while the project's inputs changed (the host puts them back) gave
//!   no verdict: the verifiers are re-run once, alone, and a second such
//!   answer is the host's own operational error
//!   (`crate::error::HOST_OPERATIONAL_ERROR_MARKER`), classified with
//!   transport failures and never charged to the task.

use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use crate::v2::{WorkflowV2Result, WorkflowV2Status};

/// Upper bound on one declared artifact verifier. They check files a branch
/// produced; one past this is wedged, not slow.
pub(crate) const ARTIFACT_VERIFIER_TIMEOUT: Duration = Duration::from_secs(900);
/// Bytes kept of each stream.
const MAX_STREAM_BYTES: usize = 1024 * 1024;

/// Why a verifier run gave no pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VerifierFailure {
    /// It ran and said the branch's artifacts do not hold.
    Product(String),
    /// It gave no verdict: the environment's, never the branch's.
    Environment(String),
}

pub(super) fn result_requires_declared_artifact_verification(result: &WorkflowV2Result) -> bool {
    matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) || result
        .data
        .get("idempotent_noop")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

/// Run the branch's declared artifact verifiers under the project-input
/// tripwire of the run at `run_root`; see the module docs for what each
/// answer means. `Err` carries the text the branch's result is built from:
/// a product failure, or the host's operational error.
pub(crate) fn verify_declared_artifacts_for_result(
    input: &serde_json::Value,
    result: &WorkflowV2Result,
    workspace_root: &Path,
    run_root: Option<&Path>,
) -> Result<(), String> {
    verify_with_timeout(
        input,
        result,
        workspace_root,
        run_root,
        ARTIFACT_VERIFIER_TIMEOUT,
    )
}

pub(super) fn verify_with_timeout(
    input: &serde_json::Value,
    result: &WorkflowV2Result,
    workspace_root: &Path,
    run_root: Option<&Path>,
    timeout: Duration,
) -> Result<(), String> {
    if !result_requires_declared_artifact_verification(result) {
        return Ok(());
    }
    let mut unanswered = String::new();
    for _attempt in 0..2 {
        let (outcome, violation) = crate::write_coordinator::input_tripwire::watch_sync(
            run_root,
            "declared artifact verifier",
            || run_verifiers(input, workspace_root, timeout, run_root),
        );
        unanswered = match (violation, outcome) {
            (Some(violation), _) => violation.message(),
            (None, Ok(())) => return Ok(()),
            (None, Err(VerifierFailure::Product(failure))) => return Err(failure),
            (None, Err(VerifierFailure::Environment(reason))) => reason,
        };
    }
    Err(format!(
        "{} the declared artifact verifier gave no verdict twice; the host re-ran it once: {unanswered}",
        crate::error::HOST_OPERATIONAL_ERROR_MARKER
    ))
}

/// Every declared verifier, unwatched, stopping at the first failure; an
/// environment failure as its text (for callers that only need a verdict).
#[cfg(test)]
pub(crate) fn run_declared_artifact_verifiers(
    input: &serde_json::Value,
    workspace_root: &Path,
) -> Result<(), String> {
    run_verifiers(input, workspace_root, ARTIFACT_VERIFIER_TIMEOUT, None).map_err(|failure| {
        match failure {
            VerifierFailure::Product(text) | VerifierFailure::Environment(text) => text,
        }
    })
}

fn run_verifiers(
    input: &serde_json::Value,
    workspace_root: &Path,
    timeout: Duration,
    run_root: Option<&Path>,
) -> Result<(), VerifierFailure> {
    let commands = input
        .get("item")
        .and_then(|item| item.get("artifact_verification_commands"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty());
    for command in commands {
        run_supervised(command, workspace_root, timeout, run_root, Some(input))?;
    }
    Ok(())
}

/// One command in its own process group, bounded by `timeout`; the group is
/// killed when the command ends or runs out, so nothing it started outlives
/// its verdict. For a run (`run_root`), under the host's OS write boundary
/// (`write_coordinator::host_sandbox`): every host root sealed, only `cwd`
/// re-opened -- and not even that when `cwd` is itself a host root.
pub(crate) fn run_supervised(
    command: &str,
    cwd: &Path,
    timeout: Duration,
    run_root: Option<&Path>,
    input: Option<&serde_json::Value>,
) -> Result<(), VerifierFailure> {
    // Issue-227: `_boundary` lives until the child has been reaped below.
    let (mut process, _boundary) = crate::write_coordinator::host_sandbox::command(
        archon_shell::resolve_posix_shell(),
        run_root,
        &crate::write_coordinator::host_sandbox::verifier_writable(cwd, input),
    )
    .map_err(VerifierFailure::Environment)?;
    process
        .arg("-lc")
        .arg(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut process, 0);
    let mut child = process.spawn().map_err(|error| {
        VerifierFailure::Environment(format!("artifact verifier could not start: {error}"))
    })?;
    let pid = child.id();
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() >= deadline => {
                crate::v2::write::test_baseline_run::kill_group(Some(pid));
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!(
                    "artifact verifier did not finish within {}s: `{command}`",
                    timeout.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(error) => {
                crate::v2::write::test_baseline_run::kill_group(Some(pid));
                let _ = child.kill();
                break Err(format!("artifact verifier could not be waited on: {error}"));
            }
        }
    };
    // Reap anything the shell left behind, which also closes its pipes.
    crate::v2::write::test_baseline_run::kill_group(Some(pid));
    let out = collect(stdout);
    let err = collect(stderr);
    match status {
        Err(reason) => Err(VerifierFailure::Environment(reason)),
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(VerifierFailure::Product(format!(
            "declared artifact verifier failed with {status}: {}{}",
            out.trim(),
            err.trim(),
        ))),
    }
}

type Drain = Option<std::sync::mpsc::Receiver<String>>;

fn drain(pipe: Option<impl Read + Send + 'static>) -> Drain {
    let mut pipe = pipe?;
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buffer = [0u8; 8192];
        while let Ok(read) = pipe.read(&mut buffer) {
            if read == 0 {
                break;
            }
            let room = MAX_STREAM_BYTES.saturating_sub(kept.len());
            kept.extend_from_slice(&buffer[..read.min(room)]);
        }
        let _ = send.send(String::from_utf8_lossy(&kept).into_owned());
    });
    Some(receive)
}

/// What a stream held; a pipe a detached grandchild still holds open is not
/// waited on past a short grace.
fn collect(drain: Drain) -> String {
    drain
        .and_then(|receive| receive.recv_timeout(Duration::from_secs(5)).ok())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "artifact_verifier_tests.rs"]
mod tests;

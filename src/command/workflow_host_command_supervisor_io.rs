//! The host command's pipes: output drained under its limits, stdin written
//! once, and the failures either can report (Issue 272).
//!
//! A failure is reported twice on purpose. The event lets the supervisor stop
//! a child that is still running. The value each task returns lets it decide
//! the same failure after the child's exit has won the race with the event,
//! which is the only order a child that exits at once produces.

use archon_workflow::{WorkflowError, WorkflowResult};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use super::super::workflow_host_command_catalog::ResolvedHostCommand;
use super::REAP_DEADLINE;

#[derive(Debug)]
pub(super) enum SupervisorEvent {
    OutputLimit {
        stream: &'static str,
        limit: u64,
    },
    /// The rest of the message after "host command '<id>' ".
    Failed(String),
}

#[derive(Debug)]
pub(super) struct CapturedPipe {
    pub(super) bytes: Vec<u8>,
    /// Every byte the child wrote, kept or not.
    pub(super) total: u64,
    pub(super) read_error: Option<String>,
}

pub(super) async fn drain_pipe(
    mut pipe: impl AsyncRead + Unpin,
    limit: u64,
    stream: &'static str,
    events: mpsc::UnboundedSender<SupervisorEvent>,
) -> CapturedPipe {
    let mut retained = Vec::new();
    let mut total = 0u64;
    let mut read_error = None;
    let mut reported = false;
    let mut chunk = [0u8; 8192];
    let keep = usize::try_from(limit).unwrap_or(usize::MAX);
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) => break,
            Ok(read) => {
                total = total.saturating_add(read as u64);
                if retained.len() < keep {
                    let remaining = keep - retained.len();
                    retained.extend_from_slice(&chunk[..read.min(remaining)]);
                }
                if total > limit && !reported {
                    let _ = events.send(SupervisorEvent::OutputLimit { stream, limit });
                    reported = true;
                }
            }
            Err(error) => {
                let detail = format!("{stream} could not be read: {error}");
                let _ = events.send(SupervisorEvent::Failed(detail.clone()));
                read_error = Some(detail);
                break;
            }
        }
    }
    CapturedPipe {
        bytes: retained,
        total,
        read_error,
    }
}

/// Writes `bytes` to the child's stdin and closes it. The task's value is the
/// failure, if any, so it survives the event being left unread.
pub(super) fn spawn_stdin(
    mut stdin: tokio::process::ChildStdin,
    bytes: Vec<u8>,
    events: mpsc::UnboundedSender<SupervisorEvent>,
) -> tokio::task::JoinHandle<Option<String>> {
    tokio::spawn(async move {
        let result = async {
            stdin.write_all(&bytes).await?;
            stdin.shutdown().await
        }
        .await;
        let failure = result
            .err()
            .map(|error| format!("stdin delivery failed: {error}"));
        if let Some(detail) = &failure {
            let _ = events.send(SupervisorEvent::Failed(detail.clone()));
        }
        failure
    })
}

/// The stdin writer's failure, once the child has exited. Its tree is gone by
/// then, so the writer has either finished or fails at once on a closed pipe.
pub(super) async fn finish_stdin(
    task: Option<tokio::task::JoinHandle<Option<String>>>,
) -> WorkflowResult<Option<String>> {
    let Some(task) = task else {
        return Ok(None);
    };
    tokio::time::timeout(REAP_DEADLINE, task)
        .await
        .map_err(|_| {
            WorkflowError::StageFailed(
                "host command stdin delivery exceeded cleanup deadline".to_string(),
            )
        })?
        .map_err(|error| {
            WorkflowError::StageFailed(format!("host command stdin task failed: {error}"))
        })
}

pub(super) async fn finish_pipe_tasks(
    stdout: tokio::task::JoinHandle<CapturedPipe>,
    stderr: tokio::task::JoinHandle<CapturedPipe>,
) -> WorkflowResult<(CapturedPipe, CapturedPipe)> {
    tokio::time::timeout(REAP_DEADLINE, async {
        let stdout = stdout.await.map_err(|error| {
            WorkflowError::StageFailed(format!("host command stdout task failed: {error}"))
        })?;
        let stderr = stderr.await.map_err(|error| {
            WorkflowError::StageFailed(format!("host command stderr task failed: {error}"))
        })?;
        Ok((stdout, stderr))
    })
    .await
    .map_err(|_| {
        WorkflowError::StageFailed("host command pipe drain exceeded cleanup deadline".to_string())
    })?
}

pub(super) fn abort_stdin(task: Option<tokio::task::JoinHandle<Option<String>>>) {
    if let Some(task) = task {
        task.abort();
    }
}

/// The error for `detail` about `request`'s command.
pub(super) fn failed(request: &ResolvedHostCommand, detail: &str) -> WorkflowError {
    WorkflowError::StageFailed(format!("host command '{}' {detail}", request.command_id))
}

/// The error for output past a limit, exactly as the event path raises it.
pub(super) fn over_limit(
    request: &ResolvedHostCommand,
    stream: &'static str,
    limit: u64,
) -> WorkflowError {
    failed(request, &format!("{stream} output exceeded {limit} bytes"))
}

/// An overflow on either pipe, from the bytes the child actually wrote.
pub(super) fn overflow(
    request: &ResolvedHostCommand,
    stdout: &CapturedPipe,
    stderr: &CapturedPipe,
) -> Option<WorkflowError> {
    [
        ("stdout", stdout, request.max_stdout_bytes),
        ("stderr", stderr, request.max_stderr_bytes),
    ]
    .into_iter()
    .find(|(_, pipe, limit)| pipe.total > *limit)
    .map(|(stream, _, limit)| over_limit(request, stream, limit))
}

/// What fails a command that exited on its own: an overflow first, then a
/// stdin delivery failure, then an unreadable pipe.
pub(super) fn completion_failure(
    request: &ResolvedHostCommand,
    stdout: &CapturedPipe,
    stderr: &CapturedPipe,
    stdin: Option<String>,
) -> Option<WorkflowError> {
    overflow(request, stdout, stderr).or_else(|| {
        stdin
            .or_else(|| stdout.read_error.clone())
            .or_else(|| stderr.read_error.clone())
            .map(|detail| failed(request, &detail))
    })
}

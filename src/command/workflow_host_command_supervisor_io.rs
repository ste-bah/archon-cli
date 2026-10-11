//! The host command's pipes: output drained under its limits, stdin written
//! once, and the failures either can report (Issue 272).
//!
//! A failure is reported twice on purpose. The event lets the supervisor stop
//! a child that is still running. The value each task returns lets it decide
//! the same failure after the child's exit has won the race with the event,
//! which is the only order a child that exits at once produces.

use archon_workflow::{WorkflowError, WorkflowResult};
use std::io::Write;
use std::path::PathBuf;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use super::super::workflow_host_command_catalog::ResolvedHostCommand;
use super::REAP_DEADLINE;

#[derive(Debug)]
pub(super) enum SupervisorEvent {
    /// The rest of the message after "host command '<id>' ".
    Failed(String),
    Checkpoint(String),
}

#[derive(Debug)]
pub(super) struct CapturedPipe {
    /// Bounded head/tail display, with a marker when the file is longer.
    pub(super) bytes: Vec<u8>,
    /// Every byte the child wrote, kept or not.
    pub(super) total: u64,
    pub(super) retained: u64,
    pub(super) truncated: bool,
    pub(super) read_error: Option<String>,
    pub(super) path: Option<PathBuf>,
}

/// Checks UTF-8 incrementally without retaining the complete stream. An
/// incomplete code point at a read boundary is carried into the next chunk.
#[derive(Default)]
struct Utf8StreamCheck {
    pending: Vec<u8>,
    valid: bool,
}

impl Utf8StreamCheck {
    fn push(&mut self, bytes: &[u8]) {
        if !self.valid {
            return;
        }
        let mut combined = std::mem::take(&mut self.pending);
        combined.extend_from_slice(bytes);
        match std::str::from_utf8(&combined) {
            Ok(_) => {}
            Err(error) if error.error_len().is_none() => {
                self.pending
                    .extend_from_slice(&combined[error.valid_up_to()..]);
            }
            Err(_) => self.valid = false,
        }
    }

    fn is_valid(&self) -> bool {
        self.valid && self.pending.is_empty()
    }
}

pub(super) fn spill_file(
    request: &ResolvedHostCommand,
    stream: &'static str,
) -> WorkflowResult<(Option<std::fs::File>, Option<PathBuf>, Option<String>)> {
    let Some(directory) = &request.spill_dir else {
        return Ok((None, None, None));
    };
    create_spill_directories(directory)?;
    let path = directory.join(format!("{stream}.bin"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(&path).map_err(|source| WorkflowError::Io {
        path: path.clone(),
        source,
    })?;
    let attempt = directory.file_name().and_then(|name| name.to_str());
    let call_id = directory
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("call");
    Ok((
        Some(file),
        Some(path),
        Some(match attempt {
            Some(attempt) if attempt.starts_with("attempt-") => {
                format!("host-command-results/{call_id}/{attempt}/{stream}.bin")
            }
            _ => format!("host-command-results/{call_id}/{stream}.bin"),
        }),
    ))
}

/// Create each directory below the run's spill root without following links.
fn create_spill_directories(directory: &std::path::Path) -> WorkflowResult<()> {
    let results_root = directory
        .ancestors()
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name == "host-command-results")
        })
        .ok_or_else(|| {
            WorkflowError::StageFailed(format!(
                "host-command spill path has no results root: {}",
                directory.display()
            ))
        })?;
    let run_root = results_root.parent().ok_or_else(|| {
        WorkflowError::StageFailed(format!(
            "host-command results root has no run directory: {}",
            results_root.display()
        ))
    })?;
    std::fs::create_dir_all(run_root).map_err(|source| WorkflowError::Io {
        path: run_root.to_path_buf(),
        source,
    })?;
    let mut current = PathBuf::new();
    let mut in_spill_tree = false;
    for component in directory.components() {
        current.push(component.as_os_str());
        in_spill_tree |= component.as_os_str() == "host-command-results";
        if !in_spill_tree {
            continue;
        }
        match std::fs::create_dir(&current) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(WorkflowError::Io {
                    path: current,
                    source,
                });
            }
        }
        let metadata = std::fs::symlink_metadata(&current).map_err(|source| WorkflowError::Io {
            path: current.clone(),
            source,
        })?;
        if !metadata.file_type().is_dir() {
            return Err(WorkflowError::StageFailed(format!(
                "host-command spill path component is not a real directory: {}",
                current.display()
            )));
        }
    }
    if !in_spill_tree {
        return Err(WorkflowError::StageFailed(format!(
            "host-command spill path has no results root: {}",
            directory.display()
        )));
    }
    Ok(())
}

pub(super) async fn drain_pipe(
    mut pipe: impl AsyncRead + Unpin,
    limit: u64,
    stream: &'static str,
    events: mpsc::UnboundedSender<SupervisorEvent>,
    progress: archon_shell::progress::Progress,
    mut spill: Option<std::fs::File>,
    spill_path: Option<PathBuf>,
    relative_path: Option<String>,
) -> CapturedPipe {
    let mut head = Vec::new();
    let mut tail = Vec::new();
    let mut small = Vec::new();
    let mut total = 0u64;
    let mut read_error = None;
    let mut utf8 = Utf8StreamCheck {
        valid: true,
        ..Utf8StreamCheck::default()
    };
    let mut chunk = [0u8; 8192];
    let keep = usize::try_from(limit).unwrap_or(usize::MAX);
    let head_limit = keep.div_ceil(2);
    let tail_limit = keep.saturating_sub(head_limit);
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) => break,
            Ok(read) => {
                progress.record();
                total = total.saturating_add(read as u64);
                utf8.push(&chunk[..read]);
                if let Some(file) = spill.as_mut() {
                    if let Err(error) = file.write_all(&chunk[..read]) {
                        let detail = format!("{stream} spill could not be written: {error}");
                        let _ = events.send(SupervisorEvent::Failed(detail.clone()));
                        read_error = Some(detail);
                        break;
                    }
                }
                if head.len() < head_limit {
                    let remaining = head_limit - head.len();
                    head.extend_from_slice(&chunk[..read.min(remaining)]);
                }
                if small.len() < keep {
                    let remaining = keep - small.len();
                    small.extend_from_slice(&chunk[..read.min(remaining)]);
                }
                if tail_limit > 0 {
                    tail.extend_from_slice(&chunk[..read]);
                    if tail.len() > tail_limit {
                        tail.drain(..tail.len() - tail_limit);
                    }
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
    let truncated = total > keep as u64;
    if truncated && utf8.is_valid() {
        // A valid stream may have had either retained edge cut in the middle
        // of a code point. Move each edge inward to keep the display valid.
        if let Err(error) = std::str::from_utf8(&head) {
            head.truncate(error.valid_up_to());
        }
        let tail_start = tail
            .iter()
            .position(|byte| byte & 0b1100_0000 != 0b1000_0000)
            .unwrap_or(tail.len());
        tail.drain(..tail_start);
    }
    let mut bytes = if !truncated { small } else { head };
    let retained = bytes.len() as u64 + if truncated { tail.len() as u64 } else { 0 };
    if truncated {
        let omitted = total
            .saturating_sub(bytes.len() as u64)
            .saturating_sub(tail.len() as u64);
        bytes.extend_from_slice(&tail);
        if let Some(relative) = &relative_path {
            bytes.extend_from_slice(
                format!("\noutput truncated: {omitted} bytes omitted; full output: {relative}\n")
                    .as_bytes(),
            );
        }
    }
    CapturedPipe {
        bytes,
        total,
        retained,
        truncated,
        read_error,
        path: spill_path,
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
/// then, so the writer has either finished or fails at once on a closed pipe;
/// a writer that does neither is a teardown stall (`Err`, the evidence), not
/// a failure of the call.
pub(super) async fn finish_stdin(
    task: Option<tokio::task::JoinHandle<Option<String>>>,
) -> Result<Option<String>, String> {
    let Some(mut task) = task else {
        return Ok(None);
    };
    match tokio::time::timeout(REAP_DEADLINE, &mut task).await {
        Ok(Ok(failure)) => Ok(failure),
        Ok(Err(error)) => Err(format!("host command stdin task failed: {error}")),
        Err(_) => {
            task.abort();
            Err("host command stdin delivery did not end after teardown".to_string())
        }
    }
}

/// Both pipes, drained to end of file. Pipes that stay open after teardown
/// are held by a process outside the tree: a stall (`Err`, the evidence),
/// never a failure of the call. The drains are aborted then, not left behind.
pub(super) async fn finish_pipe_tasks(
    mut stdout: tokio::task::JoinHandle<CapturedPipe>,
    mut stderr: tokio::task::JoinHandle<CapturedPipe>,
) -> Result<(CapturedPipe, CapturedPipe), String> {
    let drained = tokio::time::timeout(REAP_DEADLINE, async {
        let out = (&mut stdout)
            .await
            .map_err(|error| format!("host command stdout task failed: {error}"))?;
        let err = (&mut stderr)
            .await
            .map_err(|error| format!("host command stderr task failed: {error}"))?;
        Ok((out, err))
    })
    .await;
    drained.unwrap_or_else(|_| {
        stdout.abort();
        stderr.abort();
        Err(
            "host command output pipes stayed open after teardown: a process still holds them"
                .to_string(),
        )
    })
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
/// What fails a command that exited on its own: stdin delivery, then unreadable pipe.
pub(super) fn completion_failure(
    request: &ResolvedHostCommand,
    stdout: &CapturedPipe,
    stderr: &CapturedPipe,
    stdin: Option<String>,
) -> Option<WorkflowError> {
    stdin
        .or_else(|| stdout.read_error.clone())
        .or_else(|| stderr.read_error.clone())
        .map(|detail| failed(request, &detail))
}

use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use archon_tools::execution_deadline::abort_pipe_tasks;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;

use super::{CommandOutput, RunError};

#[path = "executor_process_spawn.rs"]
mod spawn;
use spawn::{OwnedChild, spawn_hook_process};

use super::NoProgressWindow;

const HOOK_OUTPUT_BYTES: usize = 64 * 1024;
const READ_CHUNK_BYTES: usize = 8 * 1024;
const TRUNCATION_MARKER: &str = "\n[hook output truncated at 65536 bytes]";

pub(super) async fn run_command(
    command: &str,
    payload: &[u8],
    cwd: &Path,
    session_id: &str,
    event_name: &str,
    timeout_secs: u32,
) -> Result<CommandOutput, RunError> {
    let spawned = spawn_hook_process(command, cwd, session_id, event_name).await?;
    // Observe output only after the process exists; process creation has its
    // own no-answer guard. There is no total runtime budget.
    let deadline = NoProgressWindow::new(Duration::from_secs(u64::from(timeout_secs)));
    tracing::debug!(
        spawn_ms = spawned.spawn_latency.as_millis(),
        timeout_secs,
        "hook process spawned"
    );
    let mut child = spawned.child;
    let budget = Arc::new(AtomicUsize::new(HOOK_OUTPUT_BYTES));
    let mut stdout = drain_pipe(
        child.stdout().take(),
        Arc::clone(&budget),
        Arc::clone(&deadline),
    );
    let mut stderr = drain_pipe(child.stderr().take(), budget, Arc::clone(&deadline));
    let _readers = PipeReaders::new(&stdout, &stderr);
    let write_error =
        match within_window(&deadline, "stdin write", write_payload(&mut child, payload)).await {
            Ok(error) => error,
            Err(error) => {
                let cleanup_error = timeout_with_cleanup(&mut child, &stdout, &stderr).await;
                return Err(combine_cleanup_error(error, cleanup_error));
            }
        };
    let status = match wait_or_terminate(&mut child, &deadline).await {
        Ok(status) => status,
        Err(error) => {
            abort_pipe_tasks(&stdout, &stderr);
            return Err(error);
        }
    };
    let (stdout, stderr) = match join_pipes(&deadline, &mut stdout, &mut stderr).await {
        Ok(pipes) => pipes,
        Err(error) => {
            let cleanup_error = terminate_process_tree(&mut child).await;
            return Err(combine_cleanup_error(error, cleanup_error));
        }
    };
    if let Some(error) = stdout.read_error.as_ref().or(stderr.read_error.as_ref()) {
        let cleanup_error = terminate_process_tree(&mut child).await;
        return Err(combine_cleanup_error(
            RunError::Io(format!("pipe read failed: {error}")),
            cleanup_error,
        ));
    }
    match within_window(&deadline, "process group", child.wait_group_empty()).await {
        Ok(Ok(())) => {}
        result => {
            let error = match result {
                Ok(Err(error)) => RunError::Io(error.to_string()),
                Err(error) => error,
                Ok(Ok(())) => unreachable!(),
            };
            let cleanup = terminate_process_tree(&mut child).await;
            return Err(combine_cleanup_error(error, cleanup));
        }
    }
    // The group was just seen empty: never signal its ID again, on any path.
    child.disarm_group();
    check_write_error(write_error, status.success())?;
    Ok(CommandOutput::from_pipes(
        status.code().unwrap_or(-1),
        stdout,
        stderr,
    ))
}

async fn timeout_with_cleanup(
    child: &mut OwnedChild,
    stdout: &JoinHandle<PipeOutput>,
    stderr: &JoinHandle<PipeOutput>,
) -> Option<String> {
    let cleanup_error = terminate_process_tree(child).await;
    abort_pipe_tasks(stdout, stderr);
    cleanup_error
}

/// All phases observe the same renewable no-progress window.
async fn within_window<T>(
    window: &NoProgressWindow,
    phase: &'static str,
    future: impl Future<Output = T>,
) -> Result<T, RunError> {
    window.wait(phase, future).await
}

async fn write_payload(child: &mut OwnedChild, payload: &[u8]) -> Option<std::io::Error> {
    let mut stdin = child.stdin().take()?;
    if let Err(error) = stdin.write_all(payload).await {
        return Some(error);
    }
    if let Err(error) = stdin.flush().await {
        return Some(error);
    }
    drop(stdin);
    None
}

async fn wait_or_terminate(
    child: &mut OwnedChild,
    deadline: &NoProgressWindow,
) -> Result<std::process::ExitStatus, RunError> {
    match within_window(deadline, "process wait", child.wait()).await {
        Ok(Ok(status)) => Ok(status),
        Ok(Err(error)) => {
            let cleanup_error = terminate_process_tree(child).await;
            Err(combine_cleanup_error(
                RunError::Io(error.to_string()),
                cleanup_error,
            ))
        }
        Err(error) => {
            let cleanup_error = terminate_process_tree(child).await;
            Err(combine_cleanup_error(error, cleanup_error))
        }
    }
}

async fn terminate_process_tree(child: &mut OwnedChild) -> Option<String> {
    // Unix: the group guard checks the ID is still this hook's group.
    #[cfg(unix)]
    let kill_error = child.kill_group().map(|error| error.to_string());
    #[cfg(not(unix))]
    let kill_error = child.start_kill().err().map(|error| error.to_string());
    let wait_error = match tokio::time::timeout(Duration::from_secs(2), child.wait()).await {
        Ok(Ok(_)) => None,
        Ok(Err(error)) => Some(error.to_string()),
        Err(_) => Some("process reap exceeded 2 second cleanup deadline".to_string()),
    };
    let cleanup_error = kill_error.or(wait_error);
    if let Some(error) = &cleanup_error {
        tracing::warn!(error, "hook process-tree cleanup failed");
    }
    cleanup_error
}

fn combine_cleanup_error(error: RunError, cleanup_error: Option<String>) -> RunError {
    match cleanup_error {
        Some(cleanup_error) => RunError::Cleanup(Box::new(error), cleanup_error),
        None => error,
    }
}

fn check_write_error(error: Option<std::io::Error>, success: bool) -> Result<(), RunError> {
    if let Some(error) = error
        && (error.kind() != std::io::ErrorKind::BrokenPipe || !success)
    {
        return Err(RunError::Io(error.to_string()));
    }
    Ok(())
}

struct PipeOutput {
    bytes: Vec<u8>,
    truncated: bool,
    read_error: Option<String>,
}

fn drain_pipe<T>(
    pipe: Option<T>,
    budget: Arc<AtomicUsize>,
    progress: Arc<NoProgressWindow>,
) -> JoinHandle<PipeOutput>
where
    T: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut output = PipeOutput {
            bytes: Vec::new(),
            truncated: false,
            read_error: None,
        };
        let Some(mut pipe) = pipe else { return output };
        let mut chunk = [0; READ_CHUNK_BYTES];
        loop {
            let read = match pipe.read(&mut chunk).await {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) => {
                    output.read_error = Some(error.to_string());
                    break;
                }
            };
            progress.record_output();
            let retained = reserve_bytes(&budget, read);
            output.bytes.extend_from_slice(&chunk[..retained]);
            output.truncated |= retained < read;
        }
        output
    })
}

fn reserve_bytes(budget: &AtomicUsize, requested: usize) -> usize {
    let mut available = budget.load(Ordering::Relaxed);
    loop {
        let retained = available.min(requested);
        match budget.compare_exchange_weak(
            available,
            available - retained,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return retained,
            Err(current) => available = current,
        }
    }
}

async fn join_pipes(
    deadline: &NoProgressWindow,
    stdout: &mut JoinHandle<PipeOutput>,
    stderr: &mut JoinHandle<PipeOutput>,
) -> Result<(PipeOutput, PipeOutput), RunError> {
    match within_window(deadline, "pipe drain", async {
        tokio::try_join!(&mut *stdout, &mut *stderr)
    })
    .await
    {
        Ok(Ok(pipes)) => Ok(pipes),
        Ok(Err(error)) => {
            abort_pipe_tasks(stdout, stderr);
            Err(RunError::Io(format!("pipe task failed: {error}")))
        }
        Err(error) => {
            abort_pipe_tasks(stdout, stderr);
            Err(error)
        }
    }
}

impl CommandOutput {
    fn from_pipes(exit_code: i32, stdout: PipeOutput, stderr: PipeOutput) -> Self {
        let mut output = Self {
            exit_code,
            stdout: String::from_utf8_lossy(&stdout.bytes).into_owned(),
            stderr: String::from_utf8_lossy(&stderr.bytes).into_owned(),
        };
        let needs_marker =
            stdout.truncated || stderr.truncated || total_output_bytes(&output) > HOOK_OUTPUT_BYTES;
        output = output.with_truncation_marker(needs_marker, stdout.truncated && !stderr.truncated);
        output
    }

    fn with_truncation_marker(mut self, truncated: bool, mark_stdout: bool) -> Self {
        if !truncated {
            return self;
        }
        let retained = HOOK_OUTPUT_BYTES.saturating_sub(TRUNCATION_MARKER.len());
        truncate_combined_output(&mut self.stdout, &mut self.stderr, retained);
        if mark_stdout {
            self.stdout.push_str(TRUNCATION_MARKER);
        } else {
            self.stderr.push_str(TRUNCATION_MARKER);
        }
        self
    }
}

fn truncate_combined_output(stdout: &mut String, stderr: &mut String, retained: usize) {
    let stdout_len = stdout.len().min(retained);
    stdout.truncate(valid_utf8_boundary(stdout, stdout_len));
    let stderr_len = retained.saturating_sub(stdout.len());
    stderr.truncate(valid_utf8_boundary(stderr, stderr_len));
}

fn valid_utf8_boundary(text: &str, limit: usize) -> usize {
    let mut boundary = limit.min(text.len());
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

fn total_output_bytes(output: &CommandOutput) -> usize {
    output.stdout.len() + output.stderr.len()
}

#[cfg(test)]
#[path = "executor_process_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "executor_cancellation_tests.rs"]
mod cancellation_tests;

/// Abort detached pipe reads on every exit, including cancellation of their
/// owning execution future. AbortHandle does not consume the join handles.
struct PipeReaders([tokio::task::AbortHandle; 2]);
impl PipeReaders {
    fn new(stdout: &JoinHandle<PipeOutput>, stderr: &JoinHandle<PipeOutput>) -> Self {
        Self([stdout.abort_handle(), stderr.abort_handle()])
    }
}
impl Drop for PipeReaders {
    fn drop(&mut self) {
        for reader in &self.0 {
            reader.abort();
        }
    }
}

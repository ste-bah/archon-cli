use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use archon_tools::execution_deadline::{ExecutionDeadline, abort_pipe_tasks, join_pipe_tasks};
use process_wrap::tokio::ChildWrapper;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;

use super::{CommandOutput, RunError};

#[path = "executor_process_spawn.rs"]
mod spawn;
use spawn::spawn_hook_process;

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
    // The work budget starts here, once the process exists — see `SPAWN_BUDGET`
    // for why getting it running is not time the hook asked for.
    let deadline = ExecutionDeadline::new(Duration::from_secs(u64::from(timeout_secs)));
    tracing::debug!(
        hook = %command,
        spawn_ms = spawned.spawn_latency.as_millis(),
        timeout_secs,
        "hook process spawned"
    );
    let mut child = spawned.child;
    let process_group = child.id();
    let budget = Arc::new(AtomicUsize::new(HOOK_OUTPUT_BYTES));
    let mut stdout = drain_pipe(child.stdout().take(), Arc::clone(&budget));
    let mut stderr = drain_pipe(child.stderr().take(), budget);
    let write_error =
        match within_deadline(&deadline, "stdin write", write_payload(&mut child, payload)).await {
            Ok(error) => error,
            Err(error) => {
                let cleanup_error =
                    timeout_with_cleanup(&mut child, process_group, &stdout, &stderr).await;
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
            let cleanup_error = terminate_process_tree(&mut child, process_group).await;
            return Err(combine_cleanup_error(error, cleanup_error));
        }
    };
    if let Some(error) = stdout.read_error.as_ref().or(stderr.read_error.as_ref()) {
        let cleanup_error = terminate_process_tree(&mut child, process_group).await;
        return Err(combine_cleanup_error(
            RunError::Io(format!("pipe read failed: {error}")),
            cleanup_error,
        ));
    }
    check_write_error(write_error, status.success())?;
    Ok(CommandOutput::from_pipes(
        status.code().unwrap_or(-1),
        stdout,
        stderr,
    ))
}

async fn timeout_with_cleanup(
    child: &mut Box<dyn ChildWrapper>,
    process_group: Option<u32>,
    stdout: &JoinHandle<PipeOutput>,
    stderr: &JoinHandle<PipeOutput>,
) -> Option<String> {
    let cleanup_error = terminate_process_tree(child, process_group).await;
    abort_pipe_tasks(stdout, stderr);
    cleanup_error
}

/// Await one phase of a hook run under its work deadline, with the deadline
/// authoritative over the phase's own result.
///
/// [`tokio::time::timeout_at`] polls the inner future *first* and only consults
/// the deadline if that future is pending, so anything ready at the moment of a
/// poll resolves as `Ok` even when the budget ran out beforehand. For a hook
/// that inverts the verdict: a process that exits after its deadline — because
/// the runtime was starved past it, or because the kill and the exit landed in
/// the same window — gets classified from its exit code by
/// [`super::interpret_exit_code`], so a hook that had to be killed reports
/// `Success` and a blocking hook silently allows the turn. Once the budget is
/// gone the phase is a timeout no matter what the future produced.
async fn within_deadline<T>(
    deadline: &ExecutionDeadline,
    phase: &'static str,
    future: impl Future<Output = T>,
) -> Result<T, RunError> {
    match deadline.wait(future).await {
        Some(value) if !deadline.expired() => Ok(value),
        _ => Err(RunError::Timeout(phase)),
    }
}

async fn write_payload(
    child: &mut Box<dyn ChildWrapper>,
    payload: &[u8],
) -> Option<std::io::Error> {
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
    child: &mut Box<dyn ChildWrapper>,
    deadline: &ExecutionDeadline,
) -> Result<std::process::ExitStatus, RunError> {
    match within_deadline(deadline, "process wait", child.wait()).await {
        Ok(Ok(status)) => Ok(status),
        Ok(Err(error)) => {
            let cleanup_error = terminate_process_tree(child, child.id()).await;
            Err(combine_cleanup_error(
                RunError::Io(error.to_string()),
                cleanup_error,
            ))
        }
        Err(error) => {
            let cleanup_error = terminate_process_tree(child, child.id()).await;
            Err(combine_cleanup_error(error, cleanup_error))
        }
    }
}

async fn terminate_process_tree(
    child: &mut Box<dyn ChildWrapper>,
    process_group: Option<u32>,
) -> Option<String> {
    #[cfg(not(unix))]
    let _ = process_group;
    #[cfg(unix)]
    let kill_error = process_group.and_then(|pid| {
        // SAFETY: the wrapped command is the process-group leader created above.
        let result = unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
        (result != 0).then(|| std::io::Error::last_os_error().to_string())
    });
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
        Some(cleanup_error) => {
            RunError::Io(format!("{error}; process cleanup failed: {cleanup_error}"))
        }
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

fn drain_pipe<T>(pipe: Option<T>, budget: Arc<AtomicUsize>) -> JoinHandle<PipeOutput>
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
    deadline: &ExecutionDeadline,
    stdout: &mut JoinHandle<PipeOutput>,
    stderr: &mut JoinHandle<PipeOutput>,
) -> Result<(PipeOutput, PipeOutput), RunError> {
    let drained = join_pipe_tasks(deadline, stdout, stderr).await;
    // `join_pipe_tasks` is built on the same inner-future-first timeout as
    // `within_deadline`, so a drain that completed past the budget still reports
    // success. It is the last phase before the exit code is interpreted, which
    // makes it the last place a timed-out hook could be read as a clean run.
    match drained.filter(|_| !deadline.expired()) {
        Some(pipes) => Ok(pipes),
        None => {
            abort_pipe_tasks(stdout, stderr);
            Err(RunError::Timeout("pipe drain"))
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
mod tests {
    use super::*;

    #[test]
    fn stdin_timeout_preserves_cleanup_failure_context() {
        let error = combine_cleanup_error(
            RunError::Timeout("stdin write"),
            Some("fixture cleanup failure".to_string()),
        );

        assert_eq!(
            error.to_string(),
            "I/O error: timed out during stdin write; process cleanup failed: fixture cleanup failure"
        );
    }

    #[tokio::test]
    async fn an_expired_budget_outranks_a_phase_that_already_finished() {
        let deadline = ExecutionDeadline::new(Duration::ZERO);

        // This is the primitive the whole executor waits on, and on its own it
        // says the phase finished: `timeout_at` polls the inner future before
        // the deadline, so anything ready at poll time resolves as `Some` no
        // matter how long ago the budget ran out. If this ever stops holding,
        // the guard below is redundant rather than wrong.
        assert!(
            deadline.wait(std::future::ready(0_i32)).await.is_some(),
            "an expired deadline still admits a ready future"
        );

        // A hook run may not. `run_command` hands the phase result to
        // `interpret_exit_code`, so accepting a late `wait()` means a hook that
        // had to be killed for overrunning gets classified by its exit code —
        // exit 0 becomes `Success`, and a hook whose whole job is to block the
        // turn silently allows it.
        let phase = within_deadline(&deadline, "process wait", std::future::ready(0_i32)).await;

        assert!(
            matches!(phase, Err(RunError::Timeout("process wait"))),
            "expected a timeout once the budget is gone, got {phase:?}"
        );
    }

    #[tokio::test]
    async fn a_hook_with_no_work_budget_cannot_report_a_clean_run() {
        let dir = tempfile::tempdir().unwrap();

        // `true` exits 0 immediately, so every phase after the spawn is ready on
        // its first poll. With a zero work budget the only honest verdict is a
        // timeout: the spawn succeeded, the hook was never entitled to run.
        let result = run_command("exit 0", b"{}", dir.path(), "deadline", "PreToolUse", 0).await;

        assert!(
            matches!(result, Err(RunError::Timeout(_))),
            "expected a timeout, got {result:?}"
        );
    }

    #[tokio::test]
    async fn stdout_only_truncation_stays_within_the_exact_shared_bound() {
        let stdout = truncated_pipe(b"stdout", 3).await;
        let output = CommandOutput::from_pipes(0, stdout, empty_pipe());

        assert_output_bound(&output);
        assert_eq!(marker_count(&output), 1);
        assert!(output.stdout.contains(TRUNCATION_MARKER));
        assert!(!output.stderr.contains(TRUNCATION_MARKER));
    }

    #[tokio::test]
    async fn stderr_only_truncation_stays_within_the_exact_shared_bound() {
        let stderr = truncated_pipe(b"stderr", 3).await;
        let output = CommandOutput::from_pipes(0, empty_pipe(), stderr);

        assert_output_bound(&output);
        assert_eq!(marker_count(&output), 1);
        assert!(!output.stdout.contains(TRUNCATION_MARKER));
        assert!(output.stderr.contains(TRUNCATION_MARKER));
    }

    #[tokio::test]
    async fn simultaneous_truncation_has_one_combined_marker_within_bound() {
        let budget = Arc::new(AtomicUsize::new(6));
        let (mut out_writer, out_reader) = tokio::io::duplex(64);
        let (mut err_writer, err_reader) = tokio::io::duplex(64);
        let mut stdout_task = drain_pipe(Some(out_reader), Arc::clone(&budget));
        let mut stderr_task = drain_pipe(Some(err_reader), budget);
        tokio::join!(
            async { out_writer.write_all(b"stdout").await.unwrap() },
            async { err_writer.write_all(b"stderr").await.unwrap() }
        );
        drop(out_writer);
        drop(err_writer);

        let deadline = ExecutionDeadline::new(Duration::from_secs(1));
        let (stdout, stderr) = join_pipes(&deadline, &mut stdout_task, &mut stderr_task)
            .await
            .unwrap();
        let output = CommandOutput::from_pipes(0, stdout, stderr);

        assert_output_bound(&output);
        assert_eq!(marker_count(&output), 1);
    }

    async fn truncated_pipe(bytes: &[u8], budget: usize) -> PipeOutput {
        let (mut writer, reader) = tokio::io::duplex(64);
        let task = drain_pipe(Some(reader), Arc::new(AtomicUsize::new(budget)));
        writer.write_all(bytes).await.unwrap();
        drop(writer);
        task.await.unwrap()
    }

    fn assert_output_bound(output: &CommandOutput) {
        assert!(total_output_bytes(output) <= HOOK_OUTPUT_BYTES);
    }

    fn marker_count(output: &CommandOutput) -> usize {
        output.stdout.matches(TRUNCATION_MARKER).count()
            + output.stderr.matches(TRUNCATION_MARKER).count()
    }

    fn empty_pipe() -> PipeOutput {
        PipeOutput {
            bytes: Vec::new(),
            truncated: false,
            read_error: None,
        }
    }
}

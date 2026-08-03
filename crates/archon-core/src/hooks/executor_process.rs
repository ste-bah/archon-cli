use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use archon_tools::execution_deadline::{ExecutionDeadline, abort_pipe_tasks, join_pipe_tasks};
#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::task::JoinHandle;

use super::{CommandOutput, RunError};

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
    let deadline = ExecutionDeadline::new(Duration::from_secs(u64::from(timeout_secs)));
    let mut child = spawn_command(command, cwd, session_id, event_name)?;
    let process_group = child.id();
    let budget = Arc::new(AtomicUsize::new(HOOK_OUTPUT_BYTES));
    let mut stdout = drain_pipe(child.stdout().take(), Arc::clone(&budget));
    let mut stderr = drain_pipe(child.stderr().take(), budget);
    let write_error = match deadline.wait(write_payload(&mut child, payload)).await {
        Some(error) => error,
        None => {
            let cleanup_error =
                timeout_with_cleanup(&mut child, process_group, &stdout, &stderr).await;
            return Err(combine_cleanup_error(
                RunError::Timeout("stdin write"),
                cleanup_error,
            ));
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

fn spawn_command(
    command: &str,
    cwd: &Path,
    session_id: &str,
    event_name: &str,
) -> Result<Box<dyn ChildWrapper>, RunError> {
    let shell = crate::hooks::shell::resolve_hook_shell();
    let mut command_builder = Command::new(&shell.program);
    command_builder
        .arg(shell.command_arg)
        .arg(command)
        .current_dir(cwd)
        .env_clear()
        .envs(archon_tools::bash::sanitized_env())
        .env("ARCHON_SESSION_ID", session_id)
        .env("ARCHON_CWD", cwd)
        .env("ARCHON_HOOK_EVENT", event_name)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut command_wrapper = CommandWrap::from(command_builder);
    command_wrapper.wrap(KillOnDrop);
    #[cfg(unix)]
    command_wrapper.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    command_wrapper.wrap(JobObject);
    command_wrapper
        .spawn()
        .map_err(|error| RunError::Spawn(format!("{command}: {error}")))
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
    match deadline.wait(child.wait()).await {
        Some(status) => match status {
            Ok(status) => Ok(status),
            Err(error) => {
                let cleanup_error = terminate_process_tree(child, child.id()).await;
                Err(combine_cleanup_error(
                    RunError::Io(error.to_string()),
                    cleanup_error,
                ))
            }
        },
        None => {
            let cleanup_error = terminate_process_tree(child, child.id()).await;
            Err(combine_cleanup_error(
                RunError::Timeout("process wait"),
                cleanup_error,
            ))
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
    let Some(pipes) = join_pipe_tasks(deadline, stdout, stderr).await else {
        abort_pipe_tasks(stdout, stderr);
        return Err(RunError::Timeout("pipe drain"));
    };
    Ok(pipes)
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

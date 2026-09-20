//! Running one declared focused test command in a branch worktree, bounded.
//!
//! The command is the task's own declared string, fed to the POSIX shell
//! with the worktree as its working directory, the host process environment
//! plus whatever the dispatch port adds for a host-run command (the leased
//! build cache, so the run builds where the coder's will and never inside
//! the worktree or the shared tree). Timeout and output are bounded, the
//! whole process group is killed on timeout so a runner's children do not
//! outlive the verdict, and every outcome is a record: a timeout or a spawn
//! failure is written down and the wave goes on (Obs-31 — the baseline must
//! never block the wave it informs).

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt};

use crate::agent_dispatch_port::WorkflowAgentDispatch;

/// Bytes retained per stream; enough for a large libtest run's verdict
/// lines, bounded so a runaway runner cannot fill memory.
const MAX_STREAM_BYTES: usize = 8 * 1024 * 1024;

/// One command's run, however it ended.
#[derive(Debug, Clone, Default)]
pub(crate) struct CommandRun {
    pub(crate) exit_code: Option<i32>,
    pub(crate) timed_out: bool,
    pub(crate) duration_ms: u64,
    /// stdout then stderr, each bounded by [`MAX_STREAM_BYTES`].
    pub(crate) output: String,
    /// Why there is no verdict: spawn failure, or the timeout that ended it.
    pub(crate) error: Option<String>,
}

pub(crate) async fn run_in_worktree(
    dispatch: &dyn WorkflowAgentDispatch,
    worktree: &Path,
    command: &str,
) -> CommandRun {
    let started = Instant::now();
    let env = dispatch.host_command_env(worktree).await;
    let mut process = tokio::process::Command::new(archon_shell::resolve_posix_shell());
    process
        .arg("-c")
        .arg(command)
        .current_dir(worktree)
        .envs(env.vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    process.process_group(0);
    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(error) => {
            return CommandRun {
                error: Some(format!("baseline command could not start: {error}")),
                duration_ms: elapsed_ms(started),
                ..CommandRun::default()
            };
        }
    };
    let pid = child.id();
    let stdout = tokio::spawn(drain(child.stdout.take()));
    let stderr = tokio::spawn(drain(child.stderr.take()));
    let limit = dispatch.baseline_test_timeout().unwrap_or(FALLBACK_TIMEOUT);
    let (status, timed_out) = match tokio::time::timeout(limit, child.wait()).await {
        Ok(status) => (status, false),
        Err(_) => {
            kill_group(pid);
            let _ = child.kill().await;
            (child.wait().await, true)
        }
    };
    // Reap anything the shell left behind before the lease is released.
    kill_group(pid);
    let out = stdout.await.ok().flatten().unwrap_or_default();
    let err = stderr.await.ok().flatten().unwrap_or_default();
    drop(env);
    let mut output = out;
    if !err.is_empty() {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&err);
    }
    let (exit_code, error) = match (status, timed_out) {
        (_, true) => (
            None,
            Some(format!(
                "baseline command timed out after {}s",
                limit.as_secs()
            )),
        ),
        (Ok(status), false) => (status.code(), None),
        (Err(error), false) => (
            None,
            Some(format!("baseline command could not be waited on: {error}")),
        ),
    };
    CommandRun {
        exit_code,
        timed_out,
        duration_ms: elapsed_ms(started),
        output,
        error,
    }
}

async fn drain(pipe: Option<impl AsyncRead + Unpin>) -> Option<String> {
    let mut pipe = pipe?;
    let mut retained: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = pipe.read(&mut buffer).await.ok()?;
        if n == 0 {
            break;
        }
        let keep = n.min(MAX_STREAM_BYTES.saturating_sub(retained.len()));
        retained.extend_from_slice(&buffer[..keep]);
    }
    Some(String::from_utf8_lossy(&retained).into_owned())
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(unix)]
fn kill_group(pid: Option<u32>) {
    if let Some(pid) = pid
        && let Ok(pid) = i32::try_from(pid)
        && pid > 0
    {
        // SAFETY: a plain signal to the process group this host created with
        // `process_group(0)`; no memory is shared with the callee.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn kill_group(_pid: Option<u32>) {}

/// The bound for a host that configures no per-dispatch timeout: a baseline
/// is never waited on forever.
pub(crate) const FALLBACK_TIMEOUT: Duration = Duration::from_secs(3_600);

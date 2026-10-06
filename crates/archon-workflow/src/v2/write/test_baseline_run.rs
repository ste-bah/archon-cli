//! Running one declared focused test command in a branch worktree, bounded.
//!
//! The command is the task's own declared string, fed to the POSIX shell
//! with the worktree as its working directory, the default check environment
//! plus filtered toolchain/cache locators from the dispatch port (the leased
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

/// Batch G: the command runs on the host, outside any agent boundary, so it
/// runs under the project-input tripwire of the run at `run_root`; a command
/// that changed an input has no verdict (its change is restored and logged).
pub(crate) async fn run_in_worktree(
    dispatch: &dyn WorkflowAgentDispatch,
    worktree: &Path,
    command: &str,
    run_root: Option<&Path>,
) -> CommandRun {
    let label = format!(
        "host-run test command `{command}` in {}",
        worktree.display()
    );
    let (mut run, violation) = crate::write_coordinator::input_tripwire::watch(
        run_root,
        &label,
        run_unwatched(dispatch, worktree, command),
    )
    .await;
    if let Some(violation) = violation {
        run.exit_code = None;
        run.error = Some(violation.message());
    }
    run
}

async fn run_unwatched(
    dispatch: &dyn WorkflowAgentDispatch,
    worktree: &Path,
    command: &str,
) -> CommandRun {
    let started = Instant::now();
    let env = dispatch.host_command_env(worktree).await;
    let mut host = crate::acceptance_check_environment::host_environment();
    for (name, value) in &env.vars {
        // Match std's Windows case-insensitive replacement before applying policy.
        while let Some((previous, _)) = crate::acceptance_check_environment::lookup(&host, name) {
            let previous = previous.clone();
            host.remove(&previous);
        }
        host.insert(name.clone(), value.clone());
    }
    let environment =
        match crate::acceptance_check_environment::CommandEnvironment::from_host(&host) {
            Ok(environment) => environment,
            Err(error) => {
                return CommandRun {
                    error: Some(error),
                    duration_ms: elapsed_ms(started),
                    ..CommandRun::default()
                };
            }
        };
    let mut process = environment.tokio_command(archon_shell::resolve_posix_shell());
    process
        .arg("-c")
        .arg(command)
        .current_dir(worktree)
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
    // Issue-134: this future dropped mid-run (a pause, a timeout above it)
    // takes the whole group with it, not only the shell kill-on-drop reaches.
    let group = GroupKillOnDrop(pid);
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
    drop(group);
    let out = stdout.await.ok().flatten().unwrap_or_default();
    let err = stderr.await.ok().flatten().unwrap_or_default();
    let environment_error = if !timed_out && status.as_ref().is_ok_and(|status| !status.success()) {
        environment.failure(&[out.as_bytes(), err.as_bytes()])
    } else {
        None
    };
    drop(env);
    let mut output = out;
    if !err.is_empty() {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(&err);
    }
    let (mut exit_code, mut error) = match (status, timed_out) {
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
    if error.is_none()
        && let Some(reason) = environment_error
    {
        exit_code = None;
        error = Some(reason);
    }
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
pub(crate) fn kill_group(pid: Option<u32>) {
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
pub(crate) fn kill_group(_pid: Option<u32>) {}

/// Kills the process group it names when dropped, however the owning
/// future ends (Issue-134).
pub(crate) struct GroupKillOnDrop(pub(crate) Option<u32>);

impl Drop for GroupKillOnDrop {
    fn drop(&mut self) {
        kill_group(self.0.take());
    }
}

/// The bound for a host that configures no per-dispatch timeout: a baseline
/// is never waited on forever.
pub(crate) const FALLBACK_TIMEOUT: Duration = Duration::from_secs(3_600);

/// Whether the process whose pid `pidfile` holds is still alive, after a
/// short grace for the group kill to land; kills it if so, so a failing
/// test leaves nothing behind.
#[cfg(all(test, unix))]
pub(crate) fn child_alive_for_tests(pidfile: &Path) -> bool {
    std::thread::sleep(Duration::from_millis(300));
    let Some(pid) = std::fs::read_to_string(pidfile)
        .ok()
        .and_then(|text| text.trim().parse::<i32>().ok())
    else {
        return false;
    };
    // SAFETY: signals to a pid this test's own command started.
    let alive = unsafe { libc::kill(pid, 0) } == 0;
    if alive {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
    alive
}

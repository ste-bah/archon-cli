//! Getting a hook process running, on a budget of its own.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use tokio::process::Command;

use super::RunError;

/// Wall-clock budget for *creating* a hook process, kept separate from the
/// hook's configured timeout.
///
/// Process creation is not work the hook asked for, and its cost tracks how busy
/// the whole machine is rather than anything about the hook. On Windows each
/// spawn is `CreateProcess`, then a job-object association, then
/// `resume_threads`, and that last step walks a *system-wide* thread snapshot
/// (`CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD)`) resuming the ones that belong
/// to the new process — so it gets slower as every other process on the box adds
/// threads. Measured on a loaded 32-core Windows host: 0.9s to 5.0s to spawn a
/// hook whose command then ran in ~0.3s.
///
/// Charging that to the hook's timeout let machine load decide hook verdicts. A
/// two-second hook spent its entire budget inside `CreateProcess` and was
/// reported as having timed out, which under a `Block` failure policy on a
/// gating event is a refusal caused by nothing but contention. Splitting the two
/// budgets is what makes the configured timeout mean "time for the hook to do
/// its job".
///
/// This budget still exists because a spawn that never returns must not hang the
/// turn. It is deliberately far above any plausible spawn: exceeding it means
/// process creation itself is wedged, and it is reported as its own phase rather
/// than as the hook overrunning.
const SPAWN_BUDGET: Duration = Duration::from_secs(30);

pub(super) struct SpawnedHook {
    pub child: Box<dyn ChildWrapper>,
    pub spawn_latency: Duration,
}

/// Spawn the hook's shell, bounded by [`SPAWN_BUDGET`] and off the async runtime.
///
/// The spawn runs on a blocking thread because every step of it is a blocking
/// syscall. Left inline it stalls the runtime it is called from, which on the
/// single-threaded runtimes used by hook tests and by short-lived callers means
/// nothing else — including the hook's own timers — is polled for the whole
/// spawn. That starvation is what lets a deadline pass unnoticed between polls.
pub(super) async fn spawn_hook_process(
    command: &str,
    cwd: &Path,
    session_id: &str,
    event_name: &str,
) -> Result<SpawnedHook, RunError> {
    let request = SpawnRequest {
        command: command.to_owned(),
        cwd: cwd.to_path_buf(),
        session_id: session_id.to_owned(),
        event_name: event_name.to_owned(),
    };
    let started = Instant::now();
    // Dropping this handle on timeout leaves the blocking task to finish and
    // drop its child, and `KillOnDrop` plus the job object tear the tree down.
    let spawning = tokio::task::spawn_blocking(move || request.spawn());
    let child = match tokio::time::timeout(SPAWN_BUDGET, spawning).await {
        Ok(Ok(Ok(child))) => child,
        Ok(Ok(Err(error))) => return Err(RunError::Spawn(format!("{command}: {error}"))),
        Ok(Err(error)) => {
            return Err(RunError::Spawn(format!(
                "{command}: spawn task failed: {error}"
            )));
        }
        Err(_) => return Err(RunError::Timeout("process spawn")),
    };
    Ok(SpawnedHook {
        child,
        spawn_latency: started.elapsed(),
    })
}

struct SpawnRequest {
    command: String,
    cwd: PathBuf,
    session_id: String,
    event_name: String,
}

impl SpawnRequest {
    fn spawn(self) -> std::io::Result<Box<dyn ChildWrapper>> {
        let shell = archon_shell::resolve_shell();
        let mut command_builder = Command::new(&shell.program);
        command_builder
            .arg(shell.command_arg)
            .arg(&self.command)
            .current_dir(&self.cwd)
            .env_clear()
            .envs(archon_tools::bash::sanitized_env())
            .env("ARCHON_SESSION_ID", &self.session_id)
            .env("ARCHON_CWD", &self.cwd)
            .env("ARCHON_HOOK_EVENT", &self.event_name)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut command_wrapper = CommandWrap::from(command_builder);
        command_wrapper.wrap(KillOnDrop);
        #[cfg(unix)]
        command_wrapper.wrap(ProcessGroup::leader());
        #[cfg(windows)]
        command_wrapper.wrap(JobObject);
        command_wrapper.spawn()
    }
}

//! Getting a hook process running, on a budget of its own.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};

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
    pub child: OwnedChild,
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
    // Cancellation may leave the blocking task to finish. Its result owns
    // group cleanup even before the child is handed back to the async caller.
    let spawning = tokio::task::spawn_blocking(move || request.spawn());
    let child = match tokio::time::timeout(SPAWN_BUDGET, spawning).await {
        Ok(Ok(Ok(child))) => child,
        Ok(Ok(Err(error))) => {
            return Err(RunError::Spawn(format!(
                "hook process could not start: {error}"
            )));
        }
        Ok(Err(error)) => {
            return Err(RunError::Spawn(format!(
                "hook process spawn task failed: {error}"
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
    fn spawn(self) -> std::io::Result<OwnedChild> {
        let shell = archon_shell::resolve_shell();
        let mut command_builder = archon_shell::spawn::tokio_command(&shell.program);
        command_builder
            .arg(shell.command_arg)
            .arg(&self.command)
            .current_dir(&self.cwd)
            .env_clear()
            .envs(archon_tools::bash::isolated_env())
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
        let child = command_wrapper.spawn()?;
        Ok(OwnedChild {
            #[cfg(unix)]
            process_group: child.id(),
            #[cfg(unix)]
            leader_reaped: false,
            child,
        })
    }
}

/// Own the original group ID even after the parent has been reaped. This
/// guard is created inside spawn_blocking so an unclaimed result is safe too.
pub(super) struct OwnedChild {
    child: Box<dyn ChildWrapper>,
    /// Unix only: on Windows the Job Object owns the whole tree.
    #[cfg(unix)]
    process_group: Option<u32>,
    /// Unix only: once the leader is reaped, its ID stays reserved only while
    /// the group still has a member.
    #[cfg(unix)]
    leader_reaped: bool,
}

impl OwnedChild {
    /// Wait for (and reap) the group leader. This shadows the wrapper's
    /// `wait` so the reap is always recorded.
    pub(super) async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        let status = self.child.wait().await?;
        #[cfg(unix)]
        {
            self.leader_reaped = true;
        }
        Ok(status)
    }

    /// Whether the group ID may now name another group.
    ///
    /// The kernel never reuses a process ID while a process group with that
    /// ID still exists (POSIX; Linux and XNU both enforce it). So once the
    /// leader is reaped, a live process holding the leader's ID proves this
    /// group has ended and the ID was reused. Before the reap, the leader
    /// (running or zombie) holds the ID, so it is always ours.
    #[cfg(unix)]
    fn group_reused(&self, pid: u32) -> bool {
        self.leader_reaped && archon_shell::process_liveness::process_alive(pid)
    }

    /// SIGKILL the owned group, never a group that reused its ID. Disarms
    /// on success, when the group is already gone, and when the ID was
    /// reused; a real failure stays armed so Drop retries.
    #[cfg(unix)]
    pub(super) fn kill_group(&mut self) -> Option<std::io::Error> {
        let pid = self.process_group?;
        if self.group_reused(pid) {
            self.process_group = None;
            return None;
        }
        // SAFETY: the ID names the group this spawn created: the leader holds
        // it, or (after the reap) the group still exists and was not reused.
        // The residual is a reuse between the check above and this call.
        if unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) } == 0 {
            self.process_group = None;
            return None;
        }
        let error = std::io::Error::last_os_error();
        if self.leader_reaped && error.raw_os_error() == Some(libc::ESRCH) {
            self.process_group = None;
            return None;
        }
        Some(error)
    }

    /// Keep supervising the original group after the leader and pipes finish.
    pub(super) async fn wait_group_empty(&mut self) -> std::io::Result<()> {
        #[cfg(unix)]
        if let Some(pid) = self.process_group {
            loop {
                if self.group_reused(pid) {
                    // The group ended and another process took its ID.
                    break;
                }
                // SAFETY: signal 0 observes the owned group without signalling it.
                if unsafe { libc::kill(-(pid as libc::pid_t), 0) } != 0 {
                    let error = std::io::Error::last_os_error();
                    match error.raw_os_error() {
                        Some(libc::ESRCH) => break,
                        Some(libc::EPERM) => {} // It still exists.
                        _ => return Err(error),
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        // Windows ChildWrapper::wait already waits on the job object.
        Ok(())
    }

    pub(super) fn disarm_group(&mut self) {
        #[cfg(unix)]
        {
            self.process_group = None;
        }
    }

    /// Test only: point the guard at another group, as if the leader's ID
    /// had been reused after the reap.
    #[cfg(all(test, unix))]
    pub(super) fn retarget_group_for_test(&mut self, pid: u32) {
        self.process_group = Some(pid);
    }
}

impl std::ops::Deref for OwnedChild {
    type Target = Box<dyn ChildWrapper>;
    fn deref(&self) -> &Self::Target {
        &self.child
    }
}

impl std::ops::DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}

// Windows retains KillOnDrop and the kill-on-close job object.
#[cfg(unix)]
impl Drop for OwnedChild {
    fn drop(&mut self) {
        // Drop must not await: signal the whole group synchronously. The
        // inner KillOnDrop child provides Tokio's parent reaping fallback.
        let _ = self.kill_group();
        self.process_group = None;
    }
}

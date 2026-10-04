//! Direct-process supervision for trusted host-command capabilities.
//!
//! No shell is involved. The catalog supplies the executable, argv, cwd,
//! environment, limits, and timeout; model bytes can reach only piped stdin.

use std::process::Stdio;
use std::time::Duration;

use archon_workflow::{WorkflowError, WorkflowResult};
use tokio::sync::{mpsc, watch};

use super::workflow_host_command_catalog::ResolvedHostCommand;
#[path = "workflow_host_command_supervisor_io.rs"]
mod io;
use io::{SupervisorEvent, abort_stdin, drain_pipe, finish_pipe_tasks, spawn_stdin};
#[path = "workflow_host_command_termination.rs"]
mod termination;
use termination::{
    audit_no_descendants, kill_on_drop, terminate_and_reap, terminate_completed_group,
};

#[cfg(unix)]
const CLEANUP_GRACE: Duration = Duration::from_millis(100);
const REAP_DEADLINE: Duration = Duration::from_secs(2);
// A killed member stays visible as a zombie until its parent is reaped and it
// is reparented, so the window has to outlast that on a loaded machine rather
// than fail a call that terminated correctly.
#[cfg(unix)]
const DESCENDANT_AUDIT_ATTEMPTS: u32 = 25;
#[cfg(unix)]
const DESCENDANT_AUDIT_INTERVAL: Duration = Duration::from_millis(40);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostCommandSignal {
    Paused,
    Cancelled,
}

impl HostCommandSignal {
    #[cfg(all(test, unix))]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Paused => "paused",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct HostCommandControlHandle {
    sender: watch::Sender<Option<HostCommandSignal>>,
}

impl HostCommandControlHandle {
    pub(crate) fn signal(&self, signal: HostCommandSignal) -> WorkflowResult<()> {
        self.sender.send(Some(signal)).map_err(|_| {
            WorkflowError::StageFailed(
                "host command control receiver closed before signal delivery".to_string(),
            )
        })
    }
}

#[derive(Debug)]
pub(crate) struct HostCommandControl {
    receiver: watch::Receiver<Option<HostCommandSignal>>,
}

impl HostCommandControl {
    pub(crate) fn new() -> (Self, HostCommandControlHandle) {
        let (sender, receiver) = watch::channel(None);
        (Self { receiver }, HostCommandControlHandle { sender })
    }

    pub(crate) async fn wait(mut self) -> HostCommandSignal {
        loop {
            if let Some(signal) = *self.receiver.borrow_and_update() {
                return signal;
            }
            if self.receiver.changed().await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SupervisedProcessOutput {
    pub(crate) exit_code: Option<i32>,
    /// Killed at the catalog wall clock: an operational limit the executor
    /// classifies (`workflow_host_command_operational`), not a work failure.
    pub(crate) timed_out: bool,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) stdout_bytes: u64,
    pub(crate) stderr_bytes: u64,
}

impl SupervisedProcessOutput {
    /// Whether the child wrote more (stdout, stderr) than was kept, from the
    /// byte counts. A call whose output overflowed fails in the supervisor,
    /// so a returned output is not expected to be truncated; a record reports
    /// these counts rather than assuming that.
    pub(crate) fn truncation(&self) -> (bool, bool) {
        (
            self.stdout_bytes > self.stdout.len() as u64,
            self.stderr_bytes > self.stderr.len() as u64,
        )
    }
}

/// `group_records`, when given, keeps a record of the group while it is
/// owned, so a resume after a parent kill can see it (Issue 251).
pub(crate) async fn supervise_process_group(
    mut request: ResolvedHostCommand,
    control: HostCommandControl,
    group_records: Option<&std::path::Path>,
) -> WorkflowResult<SupervisedProcessOutput> {
    let mut command = tokio::process::Command::new(&request.program);
    command
        .args(&request.args)
        .current_dir(&request.cwd)
        .env_clear()
        .envs(&request.environment)
        .stdin(if request.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Its own session, not only its own group: a nested runner gives each of
    // its checks a group of its own, and the session still holds those
    // (Issue 270). On Linux it also becomes the reaper of its orphans, so a
    // descendant that leaves the session as well stays its descendant.
    #[cfg(unix)]
    // SAFETY: setsid and prctl are async-signal-safe syscalls.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            archon_shell::process_tree::become_subreaper()
        });
    }

    let mut child = command.spawn().map_err(|source| WorkflowError::Io {
        path: request.program.clone(),
        source,
    })?;
    let process_group = child.id();
    // The select below observes timeout, control and completion. It cannot
    // observe this future being dropped - task cancellation, a panic, or an
    // early return on a path that never reaches termination - and a dropped
    // supervisor used to leave the whole process group running.
    let mut group_guard = ProcessGroupGuard::new(process_group);
    let _record = super::workflow_host_command_groups::record_in(
        group_records,
        process_group,
        &request.command_id,
    )?;
    let stdout = child.stdout.take().ok_or_else(|| {
        WorkflowError::StageFailed("host command stdout pipe was not created".to_string())
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        WorkflowError::StageFailed("host command stderr pipe was not created".to_string())
    })?;
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let stdout_task = tokio::spawn(drain_pipe(
        stdout,
        request.max_stdout_bytes,
        "stdout",
        event_tx.clone(),
    ));
    let stderr_task = tokio::spawn(drain_pipe(
        stderr,
        request.max_stderr_bytes,
        "stderr",
        event_tx.clone(),
    ));
    let stdin_task = match request.stdin.take() {
        Some(bytes) => {
            let stdin = child.stdin.take().ok_or_else(|| {
                WorkflowError::StageFailed("host command stdin pipe was not created".to_string())
            })?;
            Some(spawn_stdin(stdin, bytes, event_tx.clone()))
        }
        None => None,
    };
    drop(event_tx);

    enum Outcome {
        Completed(std::io::Result<std::process::ExitStatus>),
        TimedOut,
        Controlled(HostCommandSignal),
        Event(SupervisorEvent),
    }

    let outcome = {
        let wait = child.wait();
        tokio::pin!(wait);
        let timeout = tokio::time::sleep(Duration::from_secs(request.timeout_secs));
        tokio::pin!(timeout);
        let control = control.wait();
        tokio::pin!(control);
        tokio::select! {
            biased;
            status = &mut wait => Outcome::Completed(status),
            signal = &mut control => Outcome::Controlled(signal),
            // A closed channel only means the drain tasks are done, which
            // happens whenever the child closes its pipes before it exits.
            // The pattern disables this branch then, leaving the wait.
            Some(event) = event_rx.recv() => Outcome::Event(event),
            _ = &mut timeout => Outcome::TimedOut,
        }
    };

    let status = match outcome {
        Outcome::Completed(status) => {
            group_guard.reaped();
            let status = status.map_err(|error| {
                WorkflowError::StageFailed(format!("waiting for host command failed: {error}"))
            })?;
            terminate_completed_group(process_group).await?;
            group_guard.disarm();
            status
        }
        Outcome::TimedOut => {
            let killed = terminate_and_reap(&mut child, process_group).await?;
            group_guard.reaped();
            audit_no_descendants(process_group, killed).await?;
            group_guard.disarm();
            abort_stdin(stdin_task);
            // Issue #255: returned, not raised. The output the child wrote
            // before the kill is the evidence (its progress marker) the
            // executor's retry-or-pause decision reads.
            let (stdout, stderr) = finish_pipe_tasks(stdout_task, stderr_task).await?;
            // An overflow the drain had not reported when the clock ran out is
            // still an overflow: the outcome must not depend on that order.
            if let Some(error) = io::overflow(&request, &stdout, &stderr) {
                return Err(error);
            }
            return Ok(SupervisedProcessOutput {
                exit_code: None,
                timed_out: true,
                stdout: stdout.bytes,
                stderr: stderr.bytes,
                stdout_bytes: stdout.total,
                stderr_bytes: stderr.total,
            });
        }
        Outcome::Controlled(signal) => {
            let killed = terminate_and_reap(&mut child, process_group).await?;
            group_guard.reaped();
            // Audited, but never allowed to replace the control signal. A pause
            // or cancel that comes back as `StageFailed` is not recognised as an
            // interruption, so no interrupted-call record is written and a clean
            // user cancel is recorded as a run failure.
            match audit_no_descendants(process_group, killed).await {
                Ok(()) => group_guard.disarm(),
                Err(error) => {
                    tracing::warn!(%error, "surviving process tree member after control interruption")
                }
            }
            abort_stdin(stdin_task);
            finish_pipe_tasks(stdout_task, stderr_task).await?;
            return Err(match signal {
                HostCommandSignal::Paused => WorkflowError::ControlPaused(format!(
                    "host command '{}' paused while in flight",
                    request.command_id
                )),
                HostCommandSignal::Cancelled => WorkflowError::ControlCancelled(format!(
                    "host command '{}' cancelled while in flight",
                    request.command_id
                )),
            });
        }
        Outcome::Event(event) => {
            let killed = terminate_and_reap(&mut child, process_group).await?;
            group_guard.reaped();
            audit_no_descendants(process_group, killed).await?;
            group_guard.disarm();
            abort_stdin(stdin_task);
            finish_pipe_tasks(stdout_task, stderr_task).await?;
            return Err(match event {
                SupervisorEvent::OutputLimit { stream, limit } => {
                    io::over_limit(&request, stream, limit)
                }
                SupervisorEvent::Failed(detail) => io::failed(&request, &detail),
            });
        }
    };

    // The exit is polled first, so an overflow or a stdin failure may still
    // be queued, or not yet seen at all, when it wins. Each is decided again
    // here from what the pipes and the stdin writer actually report, with
    // the outcome the event path gives (Issue 272).
    let stdin = io::finish_stdin(stdin_task).await?;
    let (stdout, stderr) = finish_pipe_tasks(stdout_task, stderr_task).await?;
    if let Some(error) = io::completion_failure(&request, &stdout, &stderr, stdin) {
        return Err(error);
    }
    Ok(SupervisedProcessOutput {
        exit_code: status.code(),
        timed_out: false,
        stdout: stdout.bytes,
        stderr: stderr.bytes,
        stdout_bytes: stdout.total,
        stderr_bytes: stderr.total,
    })
}

/// Kills the process tree if the supervisor stops running for a reason the
/// select cannot see. Disarmed once the tree is confirmed empty.
struct ProcessGroupGuard {
    leader: Option<u32>,
    /// Once the leader is reaped its pid may be reused, so the tree is no
    /// longer reached through it by ancestry.
    reaped: bool,
}

impl ProcessGroupGuard {
    fn new(leader: Option<u32>) -> Self {
        Self {
            leader,
            reaped: false,
        }
    }

    fn reaped(&mut self) {
        self.reaped = true;
    }

    /// Stops the guard signalling. Called once the tree is confirmed empty:
    /// the pid is free from that moment, so a later blind kill of the same
    /// group or session id could reach an unrelated process that claimed it.
    fn disarm(&mut self) {
        self.leader = None;
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        if let Some(leader) = self.leader {
            kill_on_drop(leader, !self.reaped);
        }
    }
}

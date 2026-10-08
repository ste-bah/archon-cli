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
#[path = "workflow_host_command_supervisor_guard.rs"]
mod guard;
use guard::{ProcessGroupGuard, stalled_output};
#[path = "workflow_host_command_termination.rs"]
mod termination;
#[cfg(all(test, unix))]
pub(crate) use termination::SCANS_PAUSED;
use termination::{confine, leader_exit, reap, terminate_and_reap, terminate_completed_group};

#[cfg(unix)]
const CLEANUP_GRACE: Duration = Duration::from_millis(100);
const REAP_DEADLINE: Duration = Duration::from_secs(2);
/// How often the tree is scanned while the command runs (Issue 270 round
/// 2): a descendant that leaves the group and the session is tied to the
/// command only while its parent lives, so it has to be seen meanwhile.
const TREE_SCAN_INTERVAL: Duration = Duration::from_millis(500);

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
    /// Killed after no child output for the catalog's no-progress window.
    /// An operational limit the executor classifies, never a work failure.
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
    let mut command = archon_shell::spawn::tokio_command(&request.program);
    archon_shell::spawn::replace_environment(command.as_std_mut(), &request.environment);
    command
        .args(&request.args)
        .current_dir(&request.cwd)
        // Its stderr renews this call's no-progress window: it may report
        // observed activity there (`archon_shell::progress`).
        .env(archon_shell::progress::SUPERVISED_ENV, "1")
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
    // It inherits nothing but its stdio (Issues 334, 340). On Apple targets
    // `tokio_command` sweeps every other descriptor before this hook runs.
    // Elsewhere this hook already makes std fork, so it sweeps here as well
    // (one `close_range` on Linux) and keeps the Issue 334 guarantee for any
    // descriptor that was opened without close-on-exec.
    #[cfg(all(unix, not(target_vendor = "apple")))]
    let ceiling =
        archon_shell::process_tree::descriptor_ceiling().map_err(|source| WorkflowError::Io {
            path: request.program.clone(),
            source,
        })?;
    #[cfg(unix)]
    // SAFETY: setsid, prctl, fcntl and close_range are async-signal-safe
    // syscalls; the ceiling was read before the fork.
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            archon_shell::process_tree::become_subreaper()?;
            #[cfg(not(target_vendor = "apple"))]
            archon_shell::process_tree::inherit_only_stdio(ceiling)?;
            Ok(())
        });
    }
    // Suspended until it is in the job `confine` makes (Issue 273), so that
    // nothing it starts can be outside the job.
    #[cfg(windows)]
    command.creation_flags(archon_shell::job_object::CREATE_SUSPENDED_FLAG);
    // A second guard behind the tree guard: the direct child dies with its
    // handle even if confinement never got as far as a tree.
    command.kill_on_drop(true);

    let mut child = command.spawn().map_err(|source| WorkflowError::Io {
        path: request.program.clone(),
        source,
    })?;
    // The select below observes timeout, control and completion. It cannot
    // observe this future being dropped - task cancellation, a panic, or an
    // early return on a path that never reaches termination - and a dropped
    // supervisor used to leave the whole process group running.
    let tree = confine(&mut child)?;
    #[cfg(unix)]
    let mut group_guard = ProcessGroupGuard::new(tree, child);
    #[cfg(not(unix))]
    let mut group_guard = ProcessGroupGuard::new(tree);
    group_guard.hold_record(super::workflow_host_command_groups::record_in(
        group_records,
        group_guard.tree.leader(),
        group_guard.tree.job_name(),
        &request.command_id,
    )?);
    #[cfg(unix)]
    let child = &mut *group_guard.child;
    // Borrowed on every platform, as the Unix guard hands it out.
    #[cfg(not(unix))]
    let child = &mut child;
    let stdout = child.stdout.take().ok_or_else(|| {
        WorkflowError::StageFailed("host command stdout pipe was not created".to_string())
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        WorkflowError::StageFailed("host command stderr pipe was not created".to_string())
    })?;
    let progress = archon_shell::progress::Progress::default();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let stdout_task = tokio::spawn(drain_pipe(
        stdout,
        request.max_stdout_bytes,
        "stdout",
        event_tx.clone(),
        progress.clone(),
    ));
    let stderr_task = tokio::spawn(drain_pipe(
        stderr,
        request.max_stderr_bytes,
        "stderr",
        event_tx.clone(),
        progress.clone(),
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
        Completed(std::io::Result<()>),
        TimedOut,
        Controlled(HostCommandSignal),
        Event(SupervisorEvent),
    }

    let outcome = {
        // The leader's exit, observed without reaping it (Unix): the tree is
        // torn down while the unreaped leader still holds its pid.
        let wait = leader_exit(child);
        tokio::pin!(wait);
        let timeout = progress.bound(
            Duration::from_secs(request.timeout_secs),
            std::future::pending::<()>(),
        );
        tokio::pin!(timeout);
        let control = control.wait();
        tokio::pin!(control);
        // The first scan waits one interval: at spawn the tree is the leader.
        let mut scan = tokio::time::interval_at(
            tokio::time::Instant::now() + TREE_SCAN_INTERVAL,
            TREE_SCAN_INTERVAL,
        );
        scan.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut scanning: Option<tokio::task::JoinHandle<()>> = None;
        loop {
            tokio::select! {
                biased;
                status = &mut wait => break Outcome::Completed(status),
                signal = &mut control => break Outcome::Controlled(signal),
                // A closed channel only means the drain tasks are done, which
                // happens whenever the child closes its pipes before it exits.
                // The pattern disables this branch then, leaving the wait.
                Some(event) = event_rx.recv() => break Outcome::Event(event),
                _ = &mut timeout => break Outcome::TimedOut,
                // A scan runs on its own thread; the select keeps watching
                // the exit, control and the clock meanwhile.
                _ = scan.tick() => {
                    if scanning.as_ref().is_none_or(tokio::task::JoinHandle::is_finished) {
                        scanning = group_guard.tree.spawn_refresh();
                    }
                }
            }
        }
    };

    match outcome {
        Outcome::Completed(exit) => {
            // Torn down before the leader is reaped, then reaped.
            let mut teardown = terminate_completed_group(&group_guard.tree).await;
            let status = reap(child).await;
            group_guard.reaped();
            if let Err(error) = &exit {
                teardown =
                    teardown.and_stalled(format!("waiting for host command failed: {error}"));
            }
            if let Err(evidence) = &status {
                teardown = teardown.and_stalled(evidence.clone());
            }
            // The exit is polled first, so an overflow or a stdin failure may
            // still be queued, or not yet seen at all, when it wins. Each is
            // decided again here from what the pipes and the stdin writer
            // actually report, with the outcome the event path gives (Issue
            // 272). Pipes or a writer that do not finish once the tree is
            // gone are held from outside it: part of the stall, settled with
            // it, before the resume record may go.
            let stdin = io::finish_stdin(stdin_task).await;
            let pipes = finish_pipe_tasks(stdout_task, stderr_task).await;
            for evidence in [stdin.as_ref().err(), pipes.as_ref().err()]
                .into_iter()
                .flatten()
            {
                teardown = teardown.and_stalled(evidence.clone());
            }
            if let Some(evidence) = group_guard.settle(teardown) {
                return Ok(stalled_output(&evidence, pipes.ok(), false));
            }
            // Every error above is part of a stall that was settled; this arm
            // only gives the compiler the values.
            let (Ok(stdin), Ok((stdout, stderr)), Ok(status)) = (stdin, pipes, status) else {
                return Ok(stalled_output(
                    "host command teardown evidence lost",
                    None,
                    false,
                ));
            };
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
        Outcome::TimedOut => {
            let teardown = terminate_and_reap(child, &group_guard.tree).await;
            group_guard.reaped();
            abort_stdin(stdin_task);
            // Issue #255: returned, not raised. The output the child wrote
            // before the kill is the evidence (its progress marker) the
            // executor's retry-or-pause decision reads.
            let pipes = finish_pipe_tasks(stdout_task, stderr_task).await;
            let teardown = match &pipes {
                Ok(_) => teardown,
                Err(evidence) => teardown.and_stalled(evidence.clone()),
            };
            if let Some(evidence) = group_guard.settle(teardown) {
                return Ok(stalled_output(&evidence, pipes.ok(), true));
            }
            let (stdout, stderr) = pipes.map_err(WorkflowError::StageFailed)?;
            // An overflow the drain had not reported when the clock ran out is
            // still an overflow: the outcome must not depend on that order.
            if let Some(error) = io::overflow(&request, &stdout, &stderr) {
                return Err(error);
            }
            Ok(SupervisedProcessOutput {
                exit_code: None,
                timed_out: true,
                stdout: stdout.bytes,
                stderr: stderr.bytes,
                stdout_bytes: stdout.total,
                stderr_bytes: stderr.total,
            })
        }
        Outcome::Controlled(signal) => {
            let teardown = terminate_and_reap(child, &group_guard.tree).await;
            group_guard.reaped();
            abort_stdin(stdin_task);
            let pipes = finish_pipe_tasks(stdout_task, stderr_task).await;
            // Recorded as evidence, but never allowed to replace the control
            // signal: a pause or cancel that comes back as anything else is
            // not recognised as an interruption, so no interrupted-call
            // record is written and a clean user cancel becomes a failure.
            let teardown = match pipes {
                Ok(_) => teardown,
                Err(evidence) => teardown.and_stalled(evidence),
            };
            if let Some(evidence) = group_guard.settle(teardown) {
                tracing::warn!(%evidence, "host command teardown stalled after a control interruption");
            }
            Err(match signal {
                HostCommandSignal::Paused => WorkflowError::ControlPaused(format!(
                    "host command '{}' paused while in flight",
                    request.command_id
                )),
                HostCommandSignal::Cancelled => WorkflowError::ControlCancelled(format!(
                    "host command '{}' cancelled while in flight",
                    request.command_id
                )),
            })
        }
        Outcome::Event(event) => {
            let teardown = terminate_and_reap(child, &group_guard.tree).await;
            group_guard.reaped();
            abort_stdin(stdin_task);
            let error = match event {
                SupervisorEvent::OutputLimit { stream, limit } => {
                    io::over_limit(&request, stream, limit)
                }
                SupervisorEvent::Failed(detail) => io::failed(&request, &detail),
            };
            let pipes = finish_pipe_tasks(stdout_task, stderr_task).await;
            let teardown = match &pipes {
                Ok(_) => teardown,
                Err(evidence) => teardown.and_stalled(evidence.clone()),
            };
            // A stall outranks the failure: the tree is not gone, and only a
            // resumable outcome keeps the run from ending on it.
            if let Some(evidence) = group_guard.settle(teardown) {
                let evidence = format!("{error}; {evidence}");
                return Ok(stalled_output(&evidence, pipes.ok(), false));
            }
            Err(error)
        }
    }
}

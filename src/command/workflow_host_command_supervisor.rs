//! Direct-process supervision for trusted host-command capabilities.
//!
//! No shell is involved. The catalog supplies the executable, argv, cwd,
//! environment, limits, and timeout; model bytes can reach only piped stdin.

use std::process::Stdio;
use std::time::Duration;

use archon_workflow::{WorkflowError, WorkflowResult};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, watch};

use super::workflow_host_command_catalog::ResolvedHostCommand;

const CLEANUP_GRACE: Duration = Duration::from_millis(100);
const REAP_DEADLINE: Duration = Duration::from_secs(2);
// A killed member stays visible as a zombie until its parent is reaped and it
// is reparented, so the window has to outlast that on a loaded machine rather
// than fail a call that terminated correctly.
const DESCENDANT_AUDIT_ATTEMPTS: u32 = 25;
const DESCENDANT_AUDIT_INTERVAL: Duration = Duration::from_millis(40);

#[cfg(unix)]
const SIGKILL_VALUE: libc::c_int = libc::SIGKILL;
#[cfg(not(unix))]
const SIGKILL_VALUE: libc::c_int = 0;

/// True when no process remains in the group.
#[cfg(unix)]
fn group_is_empty(pgid: u32) -> WorkflowResult<bool> {
    // Signal 0 performs error checking without delivering anything.
    if unsafe { libc::kill(-(pgid as libc::pid_t), 0) } == 0 {
        return Ok(false);
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::ESRCH) => Ok(true),
        // The group exists but is not ours to signal. That is still a survivor.
        Some(libc::EPERM) => Ok(false),
        _ => Err(WorkflowError::StageFailed(format!(
            "auditing host command process group {pgid} failed: {error}"
        ))),
    }
}

#[cfg(not(unix))]
fn group_is_empty(_pgid: u32) -> WorkflowResult<bool> {
    Ok(true)
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostCommandSignal {
    Paused,
    Cancelled,
}

impl HostCommandSignal {
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
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) stdout_bytes: u64,
    pub(crate) stderr_bytes: u64,
}

#[derive(Debug)]
enum SupervisorEvent {
    OutputLimit { stream: &'static str, limit: u64 },
    StdinFailure(String),
}

#[derive(Debug)]
struct CapturedPipe {
    bytes: Vec<u8>,
    total: u64,
}

pub(crate) async fn supervise_process_group(
    request: ResolvedHostCommand,
    control: HostCommandControl,
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
    #[cfg(unix)]
    command.process_group(0);

    let mut child = command.spawn().map_err(|source| WorkflowError::Io {
        path: request.program.clone(),
        source,
    })?;
    let process_group = child.id();
    // The select below observes timeout, control and completion. It cannot
    // observe this future being dropped - task cancellation, a panic, or an
    // early return on a path that never reaches termination - and a dropped
    // supervisor used to leave the whole process group running.
    let mut group_guard = ProcessGroupGuard(process_group);
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
    let stdin_task = request.stdin.map(|bytes| {
        let mut stdin = child.stdin.take().expect("piped stdin exists");
        let tx = event_tx.clone();
        tokio::spawn(async move {
            let result = async {
                stdin.write_all(&bytes).await?;
                stdin.shutdown().await
            }
            .await;
            if let Err(error) = result {
                let _ = tx.send(SupervisorEvent::StdinFailure(error.to_string()));
            }
        })
    });
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
            let status = status.map_err(|error| {
                WorkflowError::StageFailed(format!("waiting for host command failed: {error}"))
            })?;
            terminate_completed_group(process_group).await?;
            group_guard.disarm();
            status
        }
        Outcome::TimedOut => {
            terminate_and_reap(&mut child, process_group).await?;
            audit_no_descendants(process_group).await?;
            abort_stdin(stdin_task);
            finish_pipe_tasks(stdout_task, stderr_task).await?;
            return Err(WorkflowError::StageFailed(format!(
                "host command '{}' timed out after {}s",
                request.command_id, request.timeout_secs
            )));
        }
        Outcome::Controlled(signal) => {
            terminate_and_reap(&mut child, process_group).await?;
            // Audited, but never allowed to replace the control signal. A pause
            // or cancel that comes back as `StageFailed` is not recognised as an
            // interruption, so no interrupted-call record is written and a clean
            // user cancel is recorded as a run failure.
            if let Err(error) = audit_no_descendants(process_group).await {
                tracing::warn!(%error, "surviving process group member after control interruption");
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
        Outcome::Event(SupervisorEvent::OutputLimit { stream, limit }) => {
            terminate_and_reap(&mut child, process_group).await?;
            audit_no_descendants(process_group).await?;
            abort_stdin(stdin_task);
            finish_pipe_tasks(stdout_task, stderr_task).await?;
            return Err(WorkflowError::StageFailed(format!(
                "host command '{}' {stream} output exceeded {limit} bytes",
                request.command_id
            )));
        }
        Outcome::Event(SupervisorEvent::StdinFailure(error)) => {
            terminate_and_reap(&mut child, process_group).await?;
            abort_stdin(stdin_task);
            finish_pipe_tasks(stdout_task, stderr_task).await?;
            return Err(WorkflowError::StageFailed(format!(
                "host command '{}' stdin delivery failed: {error}",
                request.command_id
            )));
        }
    };

    if let Some(task) = stdin_task {
        task.await.map_err(|error| {
            WorkflowError::StageFailed(format!("host command stdin task failed: {error}"))
        })?;
    }
    let (stdout, stderr) = finish_pipe_tasks(stdout_task, stderr_task).await?;
    Ok(SupervisedProcessOutput {
        exit_code: status.code(),
        stdout: stdout.bytes,
        stderr: stderr.bytes,
        stdout_bytes: stdout.total,
        stderr_bytes: stderr.total,
    })
}

async fn drain_pipe(
    mut pipe: impl AsyncRead + Unpin,
    limit: u64,
    stream: &'static str,
    events: mpsc::UnboundedSender<SupervisorEvent>,
) -> CapturedPipe {
    let mut retained = Vec::new();
    let mut total = 0u64;
    let mut reported = false;
    let mut chunk = [0u8; 8192];
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) => break,
            Ok(read) => {
                total = total.saturating_add(read as u64);
                if retained.len() < limit as usize {
                    let remaining = limit as usize - retained.len();
                    retained.extend_from_slice(&chunk[..read.min(remaining)]);
                }
                if total > limit && !reported {
                    let _ = events.send(SupervisorEvent::OutputLimit { stream, limit });
                    reported = true;
                }
            }
            Err(error) => {
                let _ = events.send(SupervisorEvent::StdinFailure(format!(
                    "reading {stream}: {error}"
                )));
                break;
            }
        }
    }
    CapturedPipe {
        bytes: retained,
        total,
    }
}

async fn finish_pipe_tasks(
    stdout: tokio::task::JoinHandle<CapturedPipe>,
    stderr: tokio::task::JoinHandle<CapturedPipe>,
) -> WorkflowResult<(CapturedPipe, CapturedPipe)> {
    tokio::time::timeout(REAP_DEADLINE, async {
        let stdout = stdout.await.map_err(|error| {
            WorkflowError::StageFailed(format!("host command stdout task failed: {error}"))
        })?;
        let stderr = stderr.await.map_err(|error| {
            WorkflowError::StageFailed(format!("host command stderr task failed: {error}"))
        })?;
        Ok((stdout, stderr))
    })
    .await
    .map_err(|_| {
        WorkflowError::StageFailed("host command pipe drain exceeded cleanup deadline".to_string())
    })?
}

fn abort_stdin(task: Option<tokio::task::JoinHandle<()>>) {
    if let Some(task) = task {
        task.abort();
    }
}

async fn terminate_and_reap(
    child: &mut tokio::process::Child,
    process_group: Option<u32>,
) -> WorkflowResult<()> {
    signal_group(process_group, libc::SIGTERM)?;
    tokio::time::sleep(CLEANUP_GRACE).await;
    signal_group(process_group, libc::SIGKILL)?;
    tokio::time::timeout(REAP_DEADLINE, child.wait())
        .await
        .map_err(|_| {
            WorkflowError::StageFailed(
                "host command process reap exceeded cleanup deadline".to_string(),
            )
        })?
        .map_err(|error| {
            WorkflowError::StageFailed(format!("host command process reap failed: {error}"))
        })?;
    Ok(())
}

async fn terminate_completed_group(process_group: Option<u32>) -> WorkflowResult<()> {
    signal_group(process_group, libc::SIGKILL)?;
    audit_no_descendants(process_group).await
}

/// Kills the process group if the supervisor stops running for a reason the
/// select cannot see. Disarmed state is unnecessary: on the ordinary paths the
/// group is already gone, so the signal is a no-op.
struct ProcessGroupGuard(Option<u32>);

impl ProcessGroupGuard {
    /// Stops the guard signalling. Called once the group is confirmed empty:
    /// the pid is free from that moment, so a later blind `SIGKILL` to the same
    /// group id could reach an unrelated process that has since claimed it.
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        let _ = signal_group(self.0, SIGKILL_VALUE);
    }
}

/// Confirms no member of the group survived termination.
///
/// Killing a group and verifying nothing survived it are different obligations.
/// A child that called `setsid` left the group and never received either
/// signal, so reaping the direct child says nothing about it. Every capability
/// declares `detaches: false`, which makes a survivor here a broken invariant
/// rather than an expected case.
///
/// The retry exists because a just-killed member can still be a zombie in the
/// process table for a moment; only a member that outlives the whole window is
/// reported.
async fn audit_no_descendants(process_group: Option<u32>) -> WorkflowResult<()> {
    let Some(pid) = process_group else {
        return Ok(());
    };
    for attempt in 0..DESCENDANT_AUDIT_ATTEMPTS {
        if group_is_empty(pid)? {
            return Ok(());
        }
        if attempt + 1 < DESCENDANT_AUDIT_ATTEMPTS {
            tokio::time::sleep(DESCENDANT_AUDIT_INTERVAL).await;
        }
    }
    Err(WorkflowError::StageFailed(format!(
        "host command process group {pid} still has live members after termination"
    )))
}

#[cfg(unix)]
fn signal_group(process_group: Option<u32>, signal: libc::c_int) -> WorkflowResult<()> {
    let Some(pid) = process_group else {
        return Ok(());
    };
    let result = unsafe { libc::kill(-(pid as libc::pid_t), signal) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    signal_group_members(pid, signal, &error.to_string())
}

#[cfg(unix)]
fn signal_group_members(pgid: u32, signal: libc::c_int, aggregate: &str) -> WorkflowResult<()> {
    let members = process_group_members(pgid);
    if members.is_empty() {
        return Ok(());
    }
    for pid in &members {
        let _ = unsafe { libc::kill(*pid as libc::pid_t, libc::SIGSTOP) };
    }
    for pid in members.iter().rev() {
        let result = unsafe { libc::kill(*pid as libc::pid_t, signal) };
        if result == -1 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                tracing::debug!(pid, %error, "host command member signal failed");
            }
        }
    }
    let survivors = process_group_members(pgid);
    if survivors.is_empty() {
        Ok(())
    } else {
        Err(WorkflowError::StageFailed(format!(
            "signalling host command process group {pgid} failed: {aggregate} (still alive: {})",
            survivors
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }
}

#[cfg(unix)]
fn process_group_members(pgid: u32) -> Vec<u32> {
    let Ok(output) = std::process::Command::new("ps")
        .args(["-axo", "pid=,pgid=,stat="])
        .output()
    else {
        return Vec::new();
    };
    let own = std::process::id();
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid: u32 = fields.next()?.parse().ok()?;
            let group: u32 = fields.next()?.parse().ok()?;
            let zombie = fields.next().is_some_and(|state| state.starts_with('Z'));
            (group == pgid && pid != own && pid > 1 && !zombie).then_some(pid)
        })
        .collect()
}

#[cfg(not(unix))]
fn signal_group(_process_group: Option<u32>, _signal: libc::c_int) -> WorkflowResult<()> {
    Ok(())
}

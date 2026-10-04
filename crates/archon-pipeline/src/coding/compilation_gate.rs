//! Async compilation-gate command execution.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::Duration;

#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

/// Minimal command description used by the compilation gate and its lifecycle tests.
pub(crate) struct CommandSpec {
    program: PathBuf,
    args: Vec<OsString>,
    current_dir: PathBuf,
    env: Vec<(OsString, OsString)>,
}

impl CommandSpec {
    pub(crate) fn new(
        program: impl Into<PathBuf>,
        args: impl IntoIterator<Item = String>,
        current_dir: &Path,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            current_dir: current_dir.to_owned(),
            env: Vec::new(),
        }
    }

    #[cfg(test)]
    fn with_env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub(crate) fn program_display(&self) -> String {
        self.program.display().to_string()
    }

    pub(crate) fn display(&self) -> String {
        std::iter::once(self.program.as_os_str())
            .chain(self.args.iter().map(OsString::as_os_str))
            .map(|part| part.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

pub(crate) enum CommandExecution {
    Completed(Output),
    TimedOut {
        child: CleanupOutcome,
        tree: TreeTermination,
    },
}

/// What cleanup established about the rest of the timed-out process tree.
///
/// Asking the tree to die is not the same as it being dead: `killpg` returns
/// once SIGKILL is queued, and a member can keep running user code for a while
/// after that (issue #240, loaded macOS box: median ~5ms, max ~56ms).
/// `TerminateJobObject` does not wait either (issue #242). So the gate reports
/// only what it went on to observe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TreeTermination {
    /// The process group was observed empty (`killpg` reported ESRCH), or the
    /// job object's accounting reported no active process.
    Confirmed,
    /// Members were still present when `TREE_EXIT_BOUND` ran out.
    StillPresent,
    /// The group or job could not be identified or queried, or the group
    /// still answered EPERM when the bound ran out.
    CheckFailed,
}

impl TreeTermination {
    pub(crate) fn evidence(self) -> &'static str {
        match self {
            Self::Confirmed => "process group confirmed empty",
            Self::StillPresent => "process group still had members after the termination bound",
            Self::CheckFailed => "process group termination check failed",
        }
    }
}

/// Real-time bound on waiting for a killed process group to empty.
///
/// SIGKILL cannot be caught, so only a member stuck in an uninterruptible
/// kernel wait, or a zombie its new parent has not reaped, can reach this. It
/// is measured with `std::time::Instant`, not tokio time, so a paused or
/// auto-advancing test clock cannot cut it short.
///
/// The wait runs on a blocking thread, so cancelling the gate future does not
/// stop it: a runtime dropped mid-wait waits for that thread, for up to this
/// bound, before shutdown completes.
const TREE_EXIT_BOUND: Duration = Duration::from_secs(5);

/// Pacing between group checks; a pacing interval, not a budget.
#[cfg(unix)]
const TREE_EXIT_POLL: Duration = Duration::from_millis(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CleanupOutcome {
    AlreadyExited,
    TerminationRequestAccepted {
        reap: ChildReap,
    },
    TerminationRequestFailed {
        reap: ChildReap,
    },
    InspectionFailed {
        termination: TerminationRequest,
        reap: ChildReap,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildReap {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TerminationRequest {
    Accepted,
    Failed,
}

impl CleanupOutcome {
    pub(crate) fn evidence(self) -> &'static str {
        match self {
            Self::AlreadyExited => "direct child already exited and was reaped",
            Self::TerminationRequestAccepted {
                reap: ChildReap::Succeeded,
            } => "direct child termination request accepted; direct child reaped",
            Self::TerminationRequestAccepted {
                reap: ChildReap::Failed,
            } => "direct child termination request accepted; direct child reap failed",
            Self::TerminationRequestFailed {
                reap: ChildReap::Succeeded,
            } => "direct child termination request failed; direct child reaped",
            Self::TerminationRequestFailed {
                reap: ChildReap::Failed,
            } => "direct child termination request failed; direct child reap failed",
            Self::InspectionFailed {
                termination: TerminationRequest::Accepted,
                reap: ChildReap::Succeeded,
            } => {
                "direct child status inspection failed; direct child termination request accepted; direct child reaped"
            }
            Self::InspectionFailed {
                termination: TerminationRequest::Accepted,
                reap: ChildReap::Failed,
            } => {
                "direct child status inspection failed; direct child termination request accepted; direct child reap failed"
            }
            Self::InspectionFailed {
                termination: TerminationRequest::Failed,
                reap: ChildReap::Succeeded,
            } => {
                "direct child status inspection failed; direct child termination request failed; direct child reaped"
            }
            Self::InspectionFailed {
                termination: TerminationRequest::Failed,
                reap: ChildReap::Failed,
            } => {
                "direct child status inspection failed; direct child termination request failed; direct child reap failed"
            }
        }
    }
}

pub(crate) async fn execute(spec: CommandSpec, limit: Duration) -> io::Result<CommandExecution> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .current_dir(&spec.current_dir)
        .envs(spec.env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Windows: suspended until it is in a job the gate owns (issue #242), so
    // nothing it starts is outside the job, and the gate can confirm the job
    // is empty after a timeout. process-wrap's own job is private to it.
    #[cfg(windows)]
    command.creation_flags(archon_shell::job_object::CREATE_SUSPENDED_FLAG);
    let mut command = CommandWrap::from(command);
    command.wrap(KillOnDrop);
    #[cfg(unix)]
    command.wrap(ProcessGroup::leader());
    let mut child = command.spawn()?;
    // Dropping the job kills every process still in it.
    #[cfg(windows)]
    let job = confine_in_job(child.as_ref())?;
    // `ProcessGroup::leader()` makes the child's pid the group id. Read it now:
    // once the direct child is reaped, `id()` no longer reports it.
    #[cfg(unix)]
    let group = child.id();
    let stdout = child.stdout().take();
    let stderr = child.stderr().take();

    let completed = tokio::time::timeout(limit, async {
        let (stdout, stderr, status) =
            tokio::join!(read_stream(stdout), read_stream(stderr), child.wait());
        Ok::<_, io::Error>(Output {
            status: status?,
            stdout: stdout?,
            stderr: stderr?,
        })
    })
    .await;

    match completed {
        Ok(output) => output.map(CommandExecution::Completed),
        Err(_) => {
            let child_outcome = cleanup_child(&mut child).await;
            #[cfg(unix)]
            let tree = confirm_tree_terminated(group).await;
            #[cfg(windows)]
            let tree = confirm_job_terminated(job).await;
            Ok(CommandExecution::TimedOut {
                child: child_outcome,
                tree,
            })
        }
    }
}

/// Kill the timed-out process group until it is observed empty.
///
/// The direct child has been reaped by now, but members it left behind were
/// reparented away from this process, so no `wait` here can see them leave.
/// `killpg` is the one call that can: it fails with ESRCH exactly when no
/// member is left to signal. Each round re-sends SIGKILL rather than probing
/// with signal 0, so a member forked while the first SIGKILL landed is killed
/// too.
///
/// The group id cannot belong to anyone else while this loop signals it: an id
/// is not reused while a group carrying it exists, and the loop stops at the
/// first ESRCH. Reuse inside one poll interval would need the pid space to
/// wrap in a millisecond.
#[cfg(unix)]
async fn confirm_tree_terminated(group: Option<u32>) -> TreeTermination {
    // 0 would signal this process's own group and 1 is init: never valid here.
    let Some(pgid) = group
        .and_then(|id| libc::pid_t::try_from(id).ok())
        .filter(|pgid| *pgid > 1)
    else {
        return TreeTermination::CheckFailed;
    };
    // Off the runtime thread: the wait is real time, and the gate's caller may
    // be a current-thread runtime with other work to run.
    tokio::task::spawn_blocking(move || await_group_exit(pgid, TREE_EXIT_BOUND))
        .await
        .unwrap_or(TreeTermination::CheckFailed)
}

/// Re-send SIGKILL to `pgid` until the group is gone or `bound` runs out.
///
/// EPERM is not final. macOS returns it while every member left is a zombie -
/// killed, but not yet reaped by its new parent - and ESRCH only once they are
/// reaped (measured on Darwin 25). A zombie runs no code, but EPERM also means
/// "a member this process may not signal", and the two cannot be told apart
/// from here. So the loop waits for ESRCH, and reports a check failure, not an
/// empty group, if EPERM is still the answer when the bound runs out.
#[cfg(unix)]
fn await_group_exit(pgid: libc::pid_t, bound: Duration) -> TreeTermination {
    let deadline = std::time::Instant::now() + bound;
    loop {
        // SAFETY: `killpg` takes plain integers and touches no memory of ours.
        let pending = if unsafe { libc::killpg(pgid, libc::SIGKILL) } == 0 {
            TreeTermination::StillPresent
        } else {
            match io::Error::last_os_error().raw_os_error() {
                Some(libc::ESRCH) => return TreeTermination::Confirmed,
                Some(libc::EPERM) => TreeTermination::CheckFailed,
                _ => return TreeTermination::CheckFailed,
            }
        };
        if std::time::Instant::now() >= deadline {
            return pending;
        }
        std::thread::sleep(TREE_EXIT_POLL);
    }
}

/// Put the suspended child in a fresh job, killed on close, and let it run.
#[cfg(windows)]
fn confine_in_job(
    child: &dyn ChildWrapper,
) -> io::Result<std::sync::Arc<archon_shell::job_object::Job>> {
    let inner = child.inner_child();
    let (Some(pid), Some(handle)) = (inner.id(), inner.raw_handle()) else {
        return Err(io::Error::other(
            "compilation child has no process handle to confine",
        ));
    };
    let job = archon_shell::job_object::Job::create(None)?;
    job.adopt_suspended(handle, pid)?;
    Ok(std::sync::Arc::new(job))
}

/// Terminate the timed-out job until its accounting reports no active
/// process, within `TREE_EXIT_BOUND`, off the runtime thread (the same
/// real-time wait as the unix group check).
#[cfg(windows)]
async fn confirm_job_terminated(
    job: std::sync::Arc<archon_shell::job_object::Job>,
) -> TreeTermination {
    match tokio::task::spawn_blocking(move || job.kill_and_confirm(TREE_EXIT_BOUND)).await {
        Ok(Ok(0)) => TreeTermination::Confirmed,
        Ok(Ok(_)) => TreeTermination::StillPresent,
        _ => TreeTermination::CheckFailed,
    }
}

async fn cleanup_child(child: &mut Box<dyn ChildWrapper>) -> CleanupOutcome {
    match child.try_wait() {
        Ok(Some(_)) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            CleanupOutcome::AlreadyExited
        }
        Ok(None) => cleanup_after_inspection(child, false).await,
        Err(_) => cleanup_after_inspection(child, true).await,
    }
}

async fn cleanup_after_inspection(
    child: &mut Box<dyn ChildWrapper>,
    inspection_failed: bool,
) -> CleanupOutcome {
    let termination = if child.start_kill().is_ok() {
        TerminationRequest::Accepted
    } else {
        TerminationRequest::Failed
    };
    let reap = if child.wait().await.is_ok() {
        ChildReap::Succeeded
    } else {
        ChildReap::Failed
    };

    if inspection_failed {
        CleanupOutcome::InspectionFailed { termination, reap }
    } else {
        match termination {
            TerminationRequest::Accepted => CleanupOutcome::TerminationRequestAccepted { reap },
            TerminationRequest::Failed => CleanupOutcome::TerminationRequestFailed { reap },
        }
    }
}

async fn read_stream<R>(stream: Option<R>) -> io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let Some(mut stream) = stream else {
        return Ok(Vec::new());
    };
    let mut output = Vec::new();
    stream.read_to_end(&mut output).await?;
    Ok(output)
}

#[cfg(test)]
#[path = "compilation_gate_tests.rs"]
mod compilation_gate_tests;

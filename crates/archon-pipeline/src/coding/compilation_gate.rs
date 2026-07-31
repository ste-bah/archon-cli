//! Async compilation-gate command execution.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::Duration;

#[cfg(windows)]
use process_wrap::tokio::JobObject;
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
    TimedOut(CleanupOutcome),
}

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
    let mut command = CommandWrap::from(command);
    command.wrap(KillOnDrop);
    #[cfg(unix)]
    command.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    command.wrap(JobObject);
    let mut child = command.spawn()?;
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
        Err(_) => Ok(CommandExecution::TimedOut(cleanup_child(&mut child).await)),
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

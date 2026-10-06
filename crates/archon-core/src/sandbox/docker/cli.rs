//! Docker CLI calls that cannot outlive a daemon that stopped answering.
//!
//! The pool and the reaper talk to the daemon through short CLI calls: `run
//! --detach`, `inspect`, `rm --force`, `ps`. Each was awaited with no bound, so
//! a daemon that accepted the connection and then never answered held a turn
//! boundary, a teardown or a reap for ever, with nothing reported.
//!
//! The bound is a no-progress bound, never a total: the deadline restarts every
//! time the call writes a byte to stdout or stderr. None of these calls stream,
//! so in practice it reads as "no answer from the daemon within
//! [`DAEMON_NO_ANSWER_BOUND`]". On expiry the call's whole process group is
//! killed and reaped, and the caller gets [`DockerCliError::NoAnswer`] naming the
//! call and what the daemon did (bytes seen, time spent, last stderr) as
//! evidence. Every caller reacts to it explicitly; none treats it as success.

use std::fmt;
use std::process::{Output, Stdio};
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;
use tokio::process::Child;

/// How long a docker CLI call may go with no output and no exit before it is
/// treated as a daemon that is not answering.
///
/// Generous on purpose: a busy daemon can take seconds to answer `run
/// --detach`, and killing a call that would have answered turns a slow machine
/// into a broken one. `--pull never` on the held container's `run` means no
/// call here waits on a registry.
pub(super) const DAEMON_NO_ANSWER_BOUND: Duration = Duration::from_secs(60);

/// How often the blocking variant checks whether its call has exited.
const BLOCKING_POLL: Duration = Duration::from_millis(20);

/// Why a docker CLI call gave no usable answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DockerCliError {
    /// The binary could not be started at all.
    Spawn { call: String, error: String },
    /// No output and no exit within the bound. The call was killed and reaped.
    NoAnswer {
        call: String,
        bound: Duration,
        evidence: String,
    },
    /// The daemon answered, and the answer was a failure.
    Failed {
        call: String,
        status: String,
        stderr: String,
    },
}

impl DockerCliError {
    pub(super) fn is_no_answer(&self) -> bool {
        matches!(self, Self::NoAnswer { .. })
    }
}

impl fmt::Display for DockerCliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { call, error } => write!(f, "could not run `{call}`: {error}"),
            Self::NoAnswer {
                call,
                bound,
                evidence,
            } => write!(
                f,
                "`{call}` got no answer from the Docker daemon within {}s and was killed; \
                 daemon state: not answering ({evidence})",
                bound.as_secs_f64()
            ),
            Self::Failed {
                call,
                status,
                stderr,
            } => write!(f, "`{call}` failed ({status}): {stderr}"),
        }
    }
}

/// Run one docker CLI call under the no-progress bound.
///
/// `call` names the call in any error, and is supplied by the caller rather
/// than built from `args` so a long argument list (mounts, labels) does not
/// bury the part that matters. A non-zero exit is [`DockerCliError::Failed`].
pub(super) async fn run(
    binary: &str,
    args: &[String],
    call: &str,
    bound: Duration,
) -> Result<Output, DockerCliError> {
    let mut command = archon_shell::spawn::tokio_command(binary);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().map_err(|error| DockerCliError::Spawn {
        call: call.to_string(),
        error: error.to_string(),
    })?;
    let started = Instant::now();
    let (mut stdout_pipe, mut stderr_pipe) = (child.stdout.take(), child.stderr.take());
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let (mut out_chunk, mut err_chunk) = ([0u8; 4096], [0u8; 4096]);
    while stdout_pipe.is_some() || stderr_pipe.is_some() {
        // A fresh sleep per iteration: any byte restarts the deadline.
        tokio::select! {
            read = read_some(&mut stdout_pipe, &mut out_chunk) => {
                absorb(read, &mut stdout_pipe, &out_chunk, &mut stdout);
            }
            read = read_some(&mut stderr_pipe, &mut err_chunk) => {
                absorb(read, &mut stderr_pipe, &err_chunk, &mut stderr);
            }
            () = tokio::time::sleep(bound) => {
                kill_and_reap(&mut child).await;
                return Err(no_answer(call, bound, progress(started, bound, &stdout, &stderr)));
            }
        }
    }
    // Both pipes closed; the exit itself is held to the same bound.
    let status = match tokio::time::timeout(bound, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            return Err(DockerCliError::Spawn {
                call: call.to_string(),
                error: format!("could not wait for the call to exit: {error}"),
            });
        }
        Err(_) => {
            kill_and_reap(&mut child).await;
            return Err(no_answer(
                call,
                bound,
                progress(started, bound, &stdout, &stderr),
            ));
        }
    };
    if !status.success() {
        return Err(DockerCliError::Failed {
            call: call.to_string(),
            status: status.to_string(),
            stderr: String::from_utf8_lossy(&stderr).trim().to_string(),
        });
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// The same bound for a caller that cannot await: the pool's `Drop`.
///
/// Output is discarded, so there is no progress to watch and the bound is per
/// call — still "no answer from the daemon within `bound`".
pub(super) fn run_blocking(
    binary: &str,
    args: &[String],
    call: &str,
    bound: Duration,
) -> Result<(), DockerCliError> {
    let mut command = archon_shell::spawn::command(binary);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().map_err(|error| DockerCliError::Spawn {
        call: call.to_string(),
        error: error.to_string(),
    })?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(DockerCliError::Failed {
                    call: call.to_string(),
                    status: status.to_string(),
                    stderr: String::new(),
                });
            }
            Ok(None) if started.elapsed() < bound => std::thread::sleep(BLOCKING_POLL),
            Ok(None) => {
                kill_group(child.id());
                let _ = child.kill();
                let _ = child.wait();
                let evidence = format!(
                    "no exit in {:.1}s; output is not captured on this path",
                    started.elapsed().as_secs_f64()
                );
                return Err(no_answer(call, bound, evidence));
            }
            Err(error) => {
                kill_group(child.id());
                let _ = child.kill();
                let _ = child.wait();
                return Err(DockerCliError::Spawn {
                    call: call.to_string(),
                    error: format!("could not wait for the call to exit: {error}"),
                });
            }
        }
    }
}

/// Read from a pipe that may already be closed. A closed pipe never resolves,
/// so `select!` keeps waiting on the other one and on the deadline.
async fn read_some<R: tokio::io::AsyncRead + Unpin>(
    pipe: &mut Option<R>,
    chunk: &mut [u8],
) -> std::io::Result<usize> {
    match pipe {
        Some(pipe) => pipe.read(chunk).await,
        None => std::future::pending().await,
    }
}

fn absorb<R>(read: std::io::Result<usize>, pipe: &mut Option<R>, chunk: &[u8], into: &mut Vec<u8>) {
    match read {
        Ok(0) | Err(_) => *pipe = None,
        Ok(n) => into.extend_from_slice(&chunk[..n]),
    }
}

/// Kill the call and every process it started, then reap it.
async fn kill_and_reap(child: &mut Child) {
    if let Some(pid) = child.id() {
        kill_group(pid);
    }
    // `kill` also waits, so the child is reaped and leaves no zombie.
    if let Err(error) = child.kill().await {
        tracing::warn!(%error, "sandbox: could not reap a docker call that stopped answering");
    }
}

/// The call runs as the leader of its own process group, so the group id is
/// its pid.
fn kill_group(pid: u32) {
    #[cfg(unix)]
    {
        // SAFETY: `kill` takes no pointers; a negative pid names the process
        // group that `process_group(0)` made this child the leader of.
        let _ = unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGKILL) };
    }
    #[cfg(not(unix))]
    let _ = pid;
}

fn no_answer(call: &str, bound: Duration, evidence: String) -> DockerCliError {
    DockerCliError::NoAnswer {
        call: call.to_string(),
        bound,
        evidence,
    }
}

/// What the call did before it went quiet: the daemon state as evidence.
fn progress(started: Instant, bound: Duration, stdout: &[u8], stderr: &[u8]) -> String {
    let mut evidence = format!(
        "{} bytes of stdout and {} bytes of stderr in {:.1}s, none in the last {}s",
        stdout.len(),
        stderr.len(),
        started.elapsed().as_secs_f64(),
        bound.as_secs_f64()
    );
    let tail = String::from_utf8_lossy(stderr);
    let tail = tail.trim();
    if !tail.is_empty() {
        let start = tail
            .char_indices()
            .rev()
            .nth(199)
            .map_or(0, |(index, _)| index);
        evidence.push_str(&format!("; last stderr: {}", &tail[start..]));
    }
    evidence
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;

//! Running Gradle or Maven as a child process.
//!
//! Both are invoked directly rather than through a shell: the arguments are
//! constructed here, never interpolated from model output, so there is nothing
//! for a shell to reinterpret.

use std::process::Stdio;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::project::{JavaProject, Launcher, Stage, stage_args};

/// A completed build-tool run.
pub struct RunOutcome {
    pub exit_code: i32,
    /// stdout and stderr interleaved as the tool wrote them, truncated.
    pub output: String,
    /// Set when the run did not complete: timeout or cancellation.
    pub aborted: Option<String>,
}

impl RunOutcome {
    pub fn succeeded(&self) -> bool {
        self.aborted.is_none() && self.exit_code == 0
    }
}

/// Console output kept per stage.
///
/// Generous, because a Gradle failure often explains itself hundreds of lines
/// after the task that failed, but bounded: a dependency-resolution storm can
/// otherwise produce tens of megabytes.
const MAX_OUTPUT_BYTES: usize = 256 * 1024;

/// How long to keep reading the pipes after the build process has exited.
///
/// Gradle forks a daemon that inherits the build's stdout and stderr and then
/// outlives it, so the write end of those pipes stays open after the build is
/// over. Waiting for end-of-stream would therefore never return on the first
/// Gradle run in a project — the build completes and the caller hangs.
const PIPE_DRAIN_GRACE: Duration = Duration::from_secs(2);

/// Run one stage of `project` and return what the tool printed and exited with.
pub async fn run_stage(
    project: &JavaProject,
    stage: Stage,
    timeout: Duration,
    cancel: Option<CancellationToken>,
) -> RunOutcome {
    let program = match &project.launcher {
        Launcher::Wrapper(path) => path.clone(),
        Launcher::OnPath(name) => std::path::PathBuf::from(name),
    };

    let mut command = tokio::process::Command::new(&program);
    command
        .args(stage_args(project.build_system, stage))
        .current_dir(&project.root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            return RunOutcome {
                exit_code: -1,
                output: String::new(),
                aborted: Some(format!("could not run {}: {e}", program.display())),
            };
        }
    };

    // Drained concurrently with the wait. A build that fills its stdout pipe
    // while nothing reads it blocks forever, and would then be reported as a
    // timeout rather than as the deadlock it is.
    let stdout = child.stdout.take().map(drain);
    let stderr = child.stderr.take().map(drain);
    let readers: Vec<_> = [stdout, stderr].into_iter().flatten().collect();

    let cancel = cancel.unwrap_or_default();
    let status = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err("cancelled".to_string()),
        result = tokio::time::timeout(timeout, child.wait()) => match result {
            Ok(Ok(status)) => Ok(status),
            Ok(Err(e)) => Err(format!("could not wait for {}: {e}", program.display())),
            Err(_) => Err(format!("timed out after {}s", timeout.as_secs())),
        },
    };

    let mut output = String::new();
    if let Err(reason) = &status {
        // The build is still running: neither a cancellation nor a timeout
        // stops it on its own, and leaving a Gradle or Maven process holding
        // the project's build directory breaks every later stage.
        if let Err(e) = child.start_kill() {
            tracing::warn!("could not terminate the {} process: {e}", program.display());
        }
        let _ = child.wait().await;
        tracing::info!(reason, "java: terminated the build process");
    }

    for (mut task, buffer) in readers {
        // Bounded, and this is the reason it has to be: Gradle's launcher hands
        // its inherited stdout and stderr to the daemon it forks, and that
        // daemon outlives the build. The write end of the pipe therefore never
        // closes, so reading to end-of-stream never returns — the build
        // finishes and the caller waits forever on output that will not arrive.
        //
        // Everything the build itself wrote is in the buffer by the time it
        // exits, so the grace period costs nothing in the normal case.
        if tokio::time::timeout(PIPE_DRAIN_GRACE, &mut task)
            .await
            .is_err()
        {
            tracing::debug!(
                "build output pipe still open after the process exited; a forked \
                 daemon is holding it. Using what was read."
            );
            task.abort();
        }
        // Read from the shared buffer rather than the task's return value, so
        // output already received survives an aborted read.
        if let Ok(bytes) = buffer.lock() {
            output.push_str(&String::from_utf8_lossy(&bytes));
        }
    }

    match status {
        Ok(status) => RunOutcome {
            exit_code: status.code().unwrap_or(-1),
            output: truncate(output),
            aborted: None,
        },
        Err(reason) => RunOutcome {
            exit_code: -1,
            output: truncate(output),
            aborted: Some(reason),
        },
    }
}

/// Shared buffer a reader task appends into.
type SharedOutput = std::sync::Arc<std::sync::Mutex<Vec<u8>>>;

/// Read a pipe on its own task, accumulating into a buffer the caller can read
/// even if the task has to be abandoned.
///
/// Deliberately not `read_to_end` into a returned `String`: that value only
/// materialises when the read completes, so a pipe held open by a surviving
/// daemon would mean losing every byte the build actually wrote. Appending to
/// shared storage as it arrives makes the output recoverable at any point.
fn drain<R>(mut pipe: R) -> (tokio::task::JoinHandle<()>, SharedOutput)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let buffer: SharedOutput = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = std::sync::Arc::clone(&buffer);
    let handle = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut chunk = [0u8; 8192];
        loop {
            match pipe.read(&mut chunk).await {
                Ok(0) => break,
                // The lock is taken and released around the copy, never held
                // across the await above.
                Ok(read) => match sink.lock() {
                    Ok(mut bytes) => bytes.extend_from_slice(&chunk[..read]),
                    Err(_) => break,
                },
                Err(e) => {
                    tracing::warn!("could not read build output: {e}");
                    break;
                }
            }
        }
    });
    (handle, buffer)
}

fn truncate(mut text: String) -> String {
    if text.len() <= MAX_OUTPUT_BYTES {
        return text;
    }
    // Keep the tail: a build tool puts its failure summary at the end.
    let cut = text.len() - MAX_OUTPUT_BYTES;
    let cut = (cut..text.len())
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(text.len());
    text.replace_range(..cut, "[…output truncated…]\n");
    text
}

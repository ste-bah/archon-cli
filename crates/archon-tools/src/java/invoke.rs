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

    for task in [stdout, stderr].into_iter().flatten() {
        if let Ok(text) = task.await {
            output.push_str(&text);
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

/// Read a pipe to end-of-stream on its own task.
fn drain<R>(mut pipe: R) -> tokio::task::JoinHandle<String>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buffer = Vec::new();
        if let Err(e) = pipe.read_to_end(&mut buffer).await {
            tracing::warn!("could not read build output: {e}");
        }
        String::from_utf8_lossy(&buffer).into_owned()
    })
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

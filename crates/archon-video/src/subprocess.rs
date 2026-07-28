use std::path::Path;
use std::process::{Output, Stdio};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

use crate::errors::VideoError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum CommandError {
    #[error("subprocess spawn failed: {0}")]
    Spawn(std::io::Error),
    #[error("subprocess I/O failed: {0}")]
    Io(std::io::Error),
    #[error("subprocess timed out after {timeout_secs}s")]
    Timeout { timeout_secs: u64 },
}

pub(crate) enum CommandOperation {
    Metadata,
    AsrProvider,
}

impl CommandError {
    pub(crate) fn into_video_error(
        self,
        binary_name: &str,
        binary_path: &str,
        operation: &str,
        operation_kind: CommandOperation,
    ) -> VideoError {
        match self {
            Self::Spawn(_) => VideoError::BinaryNotFound {
                name: binary_name.into(),
                path: binary_path.into(),
            },
            Self::Io(error) => operation_kind.error(format!("{operation} I/O failed: {error}")),
            Self::Timeout { timeout_secs } => {
                operation_kind.error(format!("{operation} timed out after {timeout_secs}s"))
            }
        }
    }
}

impl CommandOperation {
    fn error(self, message: String) -> VideoError {
        match self {
            Self::Metadata => VideoError::MetadataFailed { message },
            Self::AsrProvider => VideoError::AsrProviderUnavailable { message },
        }
    }
}

pub(crate) async fn run_ffmpeg_audio_extraction(
    bin: &str,
    video_path: &Path,
    output_path: &Path,
) -> Result<Output, VideoError> {
    let mut command = Command::new(bin);
    command
        .args(["-hide_banner", "-nostdin", "-y"])
        .arg("-i")
        .arg(video_path)
        .args(["-vn", "-ar", "16000", "-ac", "1", "-f", "wav"])
        .arg(output_path);
    output_with_timeout(&mut command, ASR_COMMAND_TIMEOUT)
        .await
        .map_err(|error| {
            error.into_video_error(
                "ffmpeg",
                bin,
                "ffmpeg audio extraction",
                CommandOperation::Metadata,
            )
        })
}

pub(crate) async fn run_whisper_cpp(
    bin: &str,
    model: &str,
    output_prefix: &Path,
    input_path: &Path,
) -> Result<Output, VideoError> {
    let mut command = Command::new(bin);
    command
        .args(["--model", model, "--output-json", "--output-file"])
        .arg(output_prefix)
        .arg("--file")
        .arg(input_path);
    output_with_timeout(&mut command, ASR_COMMAND_TIMEOUT)
        .await
        .map_err(|error| {
            error.into_video_error(
                "whisper-cli",
                bin,
                "whisper-cpp",
                CommandOperation::AsrProvider,
            )
        })
}

const ASR_COMMAND_TIMEOUT: Duration = Duration::from_secs(10 * 60);

pub(crate) async fn output_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> Result<Output, CommandError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(CommandError::Spawn)?;
    let mut stdout = spawn_reader(child.stdout.take());
    let mut stderr = spawn_reader(child.stderr.take());

    let result = tokio::select! {
        biased;
        result = collect_output(&mut child, &mut stdout, &mut stderr) => result,
        _ = tokio::time::sleep(timeout) => {
            match collect_completed_output(&mut child, &mut stdout, &mut stderr).await {
                Ok(Some(output)) => Ok(output),
                Ok(None) => Err(CommandError::Timeout {
                    timeout_secs: timeout.as_secs(),
                }),
                Err(error) => Err(error),
            }
        },
    };
    if result.is_err() {
        kill_and_reap(&mut child).await;
        abort_readers(stdout, stderr).await;
    }
    result
}

async fn collect_output(
    child: &mut Child,
    stdout: &mut tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
    stderr: &mut tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
) -> Result<Output, CommandError> {
    let status = child.wait().await.map_err(CommandError::Io)?;
    let (stdout, stderr) = tokio::join!(stdout, stderr);
    Ok(Output {
        status,
        stdout: join_reader(stdout)?,
        stderr: join_reader(stderr)?,
    })
}

async fn collect_completed_output(
    child: &mut Child,
    stdout: &mut tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
    stderr: &mut tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
) -> Result<Option<Output>, CommandError> {
    if !stdout.is_finished() || !stderr.is_finished() {
        return Ok(None);
    }
    let Some(status) = child.try_wait().map_err(CommandError::Io)? else {
        return Ok(None);
    };
    let (stdout, stderr) = tokio::join!(stdout, stderr);
    Ok(Some(Output {
        status,
        stdout: join_reader(stdout)?,
        stderr: join_reader(stderr)?,
    }))
}

async fn kill_and_reap(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

async fn abort_readers(
    stdout: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
    stderr: tokio::task::JoinHandle<std::io::Result<Vec<u8>>>,
) {
    stdout.abort();
    stderr.abort();
    let _ = tokio::join!(stdout, stderr);
}

fn spawn_reader<T>(pipe: Option<T>) -> tokio::task::JoinHandle<std::io::Result<Vec<u8>>>
where
    T: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            pipe.read_to_end(&mut bytes).await?;
        }
        Ok(bytes)
    })
}

fn join_reader(
    result: Result<std::io::Result<Vec<u8>>, tokio::task::JoinError>,
) -> Result<Vec<u8>, CommandError> {
    result
        .map_err(|error| CommandError::Io(std::io::Error::other(error)))?
        .map_err(CommandError::Io)
}

#[cfg(test)]
mod tests {
    use std::process::Command as StdCommand;
    use std::time::Instant;

    use tempfile::NamedTempFile;

    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_and_reaps_child() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("hang.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf '%s' $$ > \"$1\"\nexec sleep 30\n",
        )
        .unwrap();
        let permissions = std::os::unix::fs::PermissionsExt::from_mode(0o755);
        std::fs::set_permissions(&script, permissions).unwrap();
        let pid_file = NamedTempFile::new().unwrap();
        let mut command = Command::new(&script);
        command.arg(pid_file.path());
        let started = Instant::now();

        let error = output_with_timeout(&mut command, Duration::from_millis(100))
            .await
            .unwrap_err();

        assert!(
            matches!(error, CommandError::Timeout { .. }),
            "unexpected subprocess error: {error:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        let pid = std::fs::read_to_string(pid_file.path()).unwrap();
        let status = StdCommand::new("kill")
            .args(["-0", pid.trim()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(!status.success(), "timed-out child process still exists");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn completed_output_wins_timeout_boundary() {
        let mut command = Command::new("sh");
        command
            .args(["-c", "printf stdout; printf stderr >&2"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        let mut stdout = spawn_reader(child.stdout.take());
        let mut stderr = spawn_reader(child.stderr.take());
        child.wait().await.unwrap();
        while !stdout.is_finished() || !stderr.is_finished() {
            tokio::task::yield_now().await;
        }

        let output = collect_completed_output(&mut child, &mut stdout, &mut stderr)
            .await
            .unwrap()
            .expect("completed subprocess was reported as timed out");

        assert!(output.status.success());
        assert_eq!(output.stdout, b"stdout");
        assert_eq!(output.stderr, b"stderr");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_does_not_wait_for_inherited_pipe_handles() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("inherited-pipes.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf '%s' $$ > \"$1\"\nsleep 30 &\nprintf '%s' $! > \"$2\"\nwait\n",
        )
        .unwrap();
        let permissions = std::os::unix::fs::PermissionsExt::from_mode(0o755);
        std::fs::set_permissions(&script, permissions).unwrap();
        let child_pid_file = NamedTempFile::new().unwrap();
        let descendant_pid_file = NamedTempFile::new().unwrap();
        let mut command = Command::new(&script);
        command
            .arg(child_pid_file.path())
            .arg(descendant_pid_file.path());
        let started = Instant::now();

        let outcome = tokio::time::timeout(
            Duration::from_secs(2),
            output_with_timeout(&mut command, Duration::from_millis(500)),
        )
        .await;

        for pid_file in [&child_pid_file, &descendant_pid_file] {
            let pid = std::fs::read_to_string(pid_file.path()).unwrap();
            let _ = StdCommand::new("kill")
                .args(["-KILL", pid.trim()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }

        let error = outcome
            .expect("timeout cleanup waited for inherited pipe handles")
            .unwrap_err();
        assert!(
            matches!(error, CommandError::Timeout { .. }),
            "unexpected subprocess error: {error:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}

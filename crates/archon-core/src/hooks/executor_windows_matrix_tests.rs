use process_wrap::tokio::{ChildWrapper, CommandWrap, JobObject, KillOnDrop};
use tokio::io::AsyncReadExt;

use super::{windows_file_command, write_windows_output_fixture};

#[derive(Clone, Copy, Debug)]
enum ProbeEnvironment {
    Inherited,
    Empty,
    Sanitized,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_raw_child_drains_large_output() {
    run_probe(false, false, ProbeEnvironment::Inherited).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_empty_env_raw_child_drains_large_output() {
    run_probe(false, false, ProbeEnvironment::Empty).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_sanitized_raw_child_drains_large_output() {
    run_probe(false, false, ProbeEnvironment::Sanitized).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_job_object_child_drains_large_output() {
    run_probe(true, false, ProbeEnvironment::Inherited).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_empty_env_job_object_child_drains_large_output() {
    run_probe(true, false, ProbeEnvironment::Empty).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_sanitized_job_object_child_drains_large_output() {
    run_probe(true, false, ProbeEnvironment::Sanitized).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_job_object_shell_child_drains_large_output() {
    run_probe(true, true, ProbeEnvironment::Inherited).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_empty_env_job_object_shell_child_drains_large_output() {
    run_probe(true, true, ProbeEnvironment::Empty).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_sanitized_job_object_shell_child_drains_large_output() {
    run_probe(true, true, ProbeEnvironment::Sanitized).await;
}

async fn run_probe(use_job_object: bool, use_hook_shell: bool, environment: ProbeEnvironment) {
    let dir = tempfile::tempdir().unwrap();
    let phase_file = dir.path().join("direct-output.phase");
    let fixture = write_windows_output_fixture(dir.path(), &phase_file);
    let mut command = output_command(&fixture, use_hook_shell, environment);
    let mut child: Box<dyn ChildWrapper> = if use_job_object {
        let mut wrapped = CommandWrap::from(command);
        wrapped.wrap(KillOnDrop).wrap(JobObject);
        wrapped.spawn().unwrap()
    } else {
        Box::new(command.spawn().unwrap())
    };
    let mut stdout = child.stdout().take().unwrap();
    let mut stderr = child.stderr().take().unwrap();
    let mut stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await.map(|_| bytes.len())
    });
    let mut stderr_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).await.map(|_| bytes.len())
    });
    let deadline = std::time::Duration::from_secs(15);
    let result = tokio::time::timeout(deadline, async {
        let status = child.wait().await.unwrap();
        (
            status,
            (&mut stdout_task).await.unwrap(),
            (&mut stderr_task).await.unwrap(),
        )
    })
    .await;
    if result.is_err() {
        let _ = child.start_kill();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), child.wait()).await;
        stdout_task.abort();
        stderr_task.abort();
    }
    let phase = std::fs::read_to_string(&phase_file).unwrap_or_else(|_| "not-started".into());
    let (status, stdout, stderr) = result.unwrap_or_else(|_| {
        panic!(
            "direct child timed out: job={use_job_object}, shell={use_hook_shell}, env={environment:?}, phase={phase:?}"
        )
    });
    assert_eq!(status.code(), Some(2));
    assert_eq!(stdout.unwrap(), 131072);
    assert_eq!(stderr.unwrap(), 131072);
}

fn output_command(
    fixture: &std::path::Path,
    use_hook_shell: bool,
    environment: ProbeEnvironment,
) -> tokio::process::Command {
    let mut command = if use_hook_shell {
        let shell = crate::hooks::shell::resolve_hook_shell();
        let mut command = tokio::process::Command::new(&shell.program);
        command
            .arg(shell.command_arg)
            .arg(windows_file_command(fixture));
        command
    } else {
        let mut command = tokio::process::Command::new("powershell");
        command.args(["-NoProfile", "-File"]).arg(fixture);
        command
    };
    command
        .current_dir(fixture.parent().unwrap())
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    match environment {
        ProbeEnvironment::Inherited => {}
        ProbeEnvironment::Empty => {
            command.env_clear();
        }
        ProbeEnvironment::Sanitized => {
            command
                .env_clear()
                .envs(archon_tools::bash::sanitized_env());
        }
    }
    command
}

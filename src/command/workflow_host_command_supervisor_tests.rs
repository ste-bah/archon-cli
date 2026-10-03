use std::collections::BTreeSet;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use archon_workflow::RemediationScope;

use super::workflow_host_command_catalog::ResolvedHostCommand;
use super::workflow_host_command_supervisor::{
    HostCommandControl, HostCommandSignal, supervise_process_group,
};

fn executable(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\nset -eu\n{body}\n")).unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&path, permissions).unwrap();
    path
}

fn command(program: PathBuf) -> ResolvedHostCommand {
    ResolvedHostCommand {
        command_id: "test-fixture".into(),
        program,
        args: Vec::new(),
        cwd: std::env::temp_dir(),
        // A resolved host request carries process essentials even under None.
        environment: ["PATH", "HOME"]
            .into_iter()
            .filter_map(|name| std::env::var_os(name).map(|value| (name.to_string(), value)))
            .collect(),
        stdin: None,
        timeout_secs: 5,
        max_stdout_bytes: 1024 * 1024,
        max_stderr_bytes: 1024 * 1024,
        declared_write_set: Vec::new(),
        remediation_scopes: BTreeSet::from([RemediationScope::Operational]),
    }
}

#[tokio::test]
async fn supervisor_waits_for_exit_when_the_child_closes_its_pipes_early() {
    // The drain tasks hold the only supervisor-event senders. A child that
    // closes stdout and stderr before exiting ends both tasks, closing the
    // channel while the process is still alive: ordinary end of output, not
    // a supervision failure.
    let temp = tempfile::tempdir().unwrap();
    let program = executable(temp.path(), "early-eof", "exec 1>&- 2>&-\nsleep 1");
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(command(program), control)
        .await
        .expect("closing the pipes before exit is not a failure");
    assert_eq!(output.exit_code, Some(0));
}

#[tokio::test]
async fn supervisor_drains_large_stdout_and_stderr_without_deadlock() {
    let temp = tempfile::tempdir().unwrap();
    let program = executable(
        temp.path(),
        "both-streams",
        "i=0; while [ $i -lt 1500 ]; do printf 'stdout-%04d-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\\n' \"$i\"; printf 'stderr-%04d-yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy\\n' \"$i\" >&2; i=$((i+1)); done",
    );
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(command(program), control)
        .await
        .unwrap();

    assert_eq!(output.exit_code, Some(0));
    assert!(output.stdout.len() > 50_000);
    assert!(output.stderr.len() > 50_000);
    assert_eq!(output.stdout_bytes as usize, output.stdout.len());
    assert_eq!(output.stderr_bytes as usize, output.stderr.len());
}

#[tokio::test]
async fn stdout_overflow_terminates_group_and_prevents_late_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let sentinel = temp.path().join("late");
    let program = executable(
        temp.path(),
        "overflow",
        &format!(
            "(sleep 0.4; printf late > '{}') & while :; do printf '0123456789abcdef'; done",
            sentinel.display()
        ),
    );
    let mut request = command(program);
    request.max_stdout_bytes = 256;
    let (control, _handle) = HostCommandControl::new();
    let error = supervise_process_group(request, control)
        .await
        .expect_err("overflow must fail operationally");

    assert!(
        error.to_string().contains("stdout output exceeded"),
        "{error}"
    );
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(!sentinel.exists(), "descendant survived output overflow");
}

#[tokio::test]
async fn timeout_terminates_background_process_group() {
    let temp = tempfile::tempdir().unwrap();
    let sentinel = temp.path().join("late");
    let program = executable(
        temp.path(),
        "timeout",
        &format!(
            "(sleep 0.4; printf late > '{}') & sleep 30",
            sentinel.display()
        ),
    );
    let mut request = command(program);
    request.timeout_secs = 0;
    let (control, _handle) = HostCommandControl::new();
    // Issue #255: an operational outcome for the executor, not an error.
    let output = supervise_process_group(request, control).await.unwrap();

    assert!(output.timed_out && output.exit_code.is_none());
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(!sentinel.exists(), "descendant survived timeout");
}

#[tokio::test]
async fn pause_and_cancel_interrupt_and_reap_direct_child() {
    for signal in [HostCommandSignal::Paused, HostCommandSignal::Cancelled] {
        let temp = tempfile::tempdir().unwrap();
        let sentinel = temp.path().join("late");
        let program = executable(
            temp.path(),
            "control",
            &format!("sleep 0.4; printf late > '{}'", sentinel.display()),
        );
        let (control, handle) = HostCommandControl::new();
        let task = tokio::spawn(supervise_process_group(command(program), control));
        tokio::time::sleep(Duration::from_millis(30)).await;
        handle.signal(signal).unwrap();
        let error = task.await.unwrap().expect_err("control must interrupt");
        assert!(error.to_string().contains(signal.as_str()), "{error}");
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(!sentinel.exists(), "child survived {}", signal.as_str());
    }
}

#[test]
fn supervisor_clears_environment_and_delivers_exact_stdin() {
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            "command::workflow_host_command_supervisor_tests::stdin_environment_child",
            "--nocapture",
        ])
        .env("ARCHON_R2A_AMBIENT_SENTINEL", "must-not-leak")
        .status()
        .unwrap();
    assert!(status.success());
}

#[tokio::test]
#[ignore = "isolated process environment"]
async fn stdin_environment_child() {
    let temp = tempfile::tempdir().unwrap();
    let program = executable(
        temp.path(),
        "stdin-env",
        "printf 'declared=%s\\n' \"${DECLARED:-missing}\"; printf 'ambient=%s\\n' \"${ARCHON_R2A_AMBIENT_SENTINEL:-absent}\"; cat",
    );
    let mut request = command(program);
    request
        .environment
        .insert("DECLARED".into(), "allowed".into());
    request.stdin = Some(b"opaque;$(printf not-executed)".to_vec());
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(request, control).await.unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(stdout.contains("declared=allowed"));
    assert!(stdout.contains("ambient=absent"));
    assert!(stdout.ends_with("opaque;$(printf not-executed)"));
    assert!(!stdout.contains("must-not-leak"));
}

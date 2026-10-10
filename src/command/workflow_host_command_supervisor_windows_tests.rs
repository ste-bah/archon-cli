//! Issue 273 (Windows): the host command and every process it starts are
//! ended, and confirmed ended, on timeout, on completion and when the
//! supervisor future is dropped.
//!
//! The fixture is PowerShell: `Start-Process` starts a detached `ping` (a
//! descendant a direct-child kill never reaches) and reports its pid.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use archon_workflow::RemediationScope;

use super::workflow_host_command_catalog::ResolvedHostCommand;
use super::workflow_host_command_supervisor::{HostCommandControl, supervise_process_group};

fn powershell() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    Path::new(&root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
}

/// A command that starts a detached `ping`, writes its pid to `pid_file`,
/// then runs `rest`.
fn command(pid_file: &Path, rest: &str, timeout_secs: u64) -> ResolvedHostCommand {
    let script = format!(
        "$p = Start-Process -FilePath ping -ArgumentList '-n','120','127.0.0.1' -WindowStyle Hidden -PassThru; Set-Content -Path '{}' -Value $p.Id; {rest}",
        pid_file.display()
    );
    ResolvedHostCommand {
        command_id: "test-fixture".into(),
        program: powershell(),
        args: vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            script,
        ],
        cwd: std::env::temp_dir(),
        // What a real host command gets, so the fixture's PowerShell starts
        // as one would (Issue 273).
        environment: super::workflow_host_environment::process_environment(),
        stdin: None,
        timeout_secs,
        max_stdout_bytes: 1024 * 1024,
        max_stderr_bytes: 1024 * 1024,
        declared_write_set: Vec::new(),
        remediation_scopes: BTreeSet::from([RemediationScope::Operational]),
        spill_dir: None,
    }
}

fn read_pid(path: &Path) -> u32 {
    let start = Instant::now();
    loop {
        if let Some(pid) = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
        {
            return pid;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "descendant never started"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn alive(pid: u32) -> bool {
    let output = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""))
}

/// Supervision confirms the job empty before it returns, so the descendant
/// must already be gone: no grace period.
fn assert_gone(pid: u32, cause: &str) {
    if alive(pid) {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .output();
        panic!("detached descendant {pid} outlived {cause}");
    }
}

#[tokio::test]
async fn timeout_ends_a_detached_descendant() {
    // Teardown confirms the job empty before supervision returns.
    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("pid");
    let request = command(&pid_file, "Start-Sleep -Seconds 120", 10);
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(request, control, None)
        .await
        .unwrap();
    assert!(output.timed_out);
    assert_gone(read_pid(&pid_file), "the timeout");
}

#[tokio::test]
async fn completion_ends_a_detached_descendant() {
    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("pid");
    let request = command(&pid_file, "exit 0", 60);
    let (control, _handle) = HostCommandControl::new();
    let output = supervise_process_group(request, control, None)
        .await
        .unwrap();
    assert_eq!(output.exit_code, Some(0));
    assert_gone(read_pid(&pid_file), "completion");
}

#[tokio::test]
async fn a_dropped_supervisor_ends_a_detached_descendant() {
    let temp = tempfile::tempdir().unwrap();
    let pid_file = temp.path().join("pid");
    let request = command(&pid_file, "Start-Sleep -Seconds 120", 120);
    let (control, _handle) = HostCommandControl::new();
    let task = tokio::spawn(async move { supervise_process_group(request, control, None).await });
    let pid = tokio::task::spawn_blocking({
        let pid_file = pid_file.clone();
        move || read_pid(&pid_file)
    })
    .await
    .unwrap();
    task.abort();
    // The task's future is dropped before its handle resolves, and the
    // dropped supervisor confirms the job empty before it lets go: the
    // descendant is gone now, with no grace period.
    let _ = task.await;
    assert!(
        !alive(pid),
        "detached descendant {pid} outlived the dropped supervisor's confirmation"
    );
}

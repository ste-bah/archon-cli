use std::time::Duration;

use serde_json::json;

use super::bash_output::CapturedOutput;

use super::bash_process::bash_result_from_pipes;
use super::*;
use crate::tool::ToolContext;

#[tokio::test]
#[cfg(unix)]
async fn parent_exit_with_descendant_held_pipes_hits_overall_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("held-pipes.pid");
    let tool = BashTool {
        timeout_secs: 1,
        max_output_bytes: 1024,
        ..Default::default()
    };
    let command = descendant_holding_pipes_command(&pid_file);
    let started = std::time::Instant::now();
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        tool.execute(
            json!({"command": command, "timeout": 1_000}),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        ),
    )
    .await
    .expect("Bash invocation exceeded its strict outer deadline");

    assert!(
        result.is_error,
        "descendant-held pipes must time out: {}",
        result.content
    );
    assert!(result.content.contains("timed out"), "{}", result.content);
    assert!(started.elapsed() < Duration::from_secs(3));
    wait_until_process_is_absent(&std::fs::read_to_string(&pid_file).expect("descendant pid file"))
        .await;
}

#[cfg(unix)]
fn descendant_holding_pipes_command(pid_file: &std::path::Path) -> String {
    let path = shell_quote(pid_file);
    format!(
        "sh -c \"trap '' HUP; echo \\$\\$ > {path}; exec sleep 30\" & while [ ! -s {path} ]; do sleep 0.01; done"
    )
}

#[cfg(unix)]
fn shell_quote(path: &std::path::Path) -> String {
    format!(
        "'{}'",
        path.display().to_string().replace('\'', "'\\\"'\\\"'")
    )
}

#[tokio::test]
#[cfg(unix)]
async fn timeout_kills_background_process_group() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("child.pid");
    let tool = BashTool {
        timeout_secs: 1,
        max_output_bytes: 1024,
        ..Default::default()
    };
    let result = tool
        .execute(
            json!({
                "command": format!(
                    "sh -c 'trap \"\" TERM; while :; do sleep 1; done' & echo $! > {}; wait",
                    pid_file.display()
                ),
                "timeout": 100
            }),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        )
        .await;

    assert!(result.is_error, "command should time out");
    let pid = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .to_string();
    for _ in 0..20 {
        if !process_exists(&pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let _ = std::process::Command::new("kill")
        .arg("-9")
        .arg(&pid)
        .status();
    panic!("background sleep process survived Bash timeout: pid={pid}");
}

#[tokio::test]
#[cfg(windows)]
async fn windows_bash_waits_for_descendant_after_parent_exits() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("detached-child.pid");
    let fixture = write_windows_detached_descendant_fixture(&pid_file);
    let tool = BashTool {
        timeout_secs: 1,
        max_output_bytes: 1024,
        ..Default::default()
    };
    let result = tokio::time::timeout(
        Duration::from_secs(4),
        tool.execute(
            json!({
                "command": windows_powershell_file_command(&fixture),
                "timeout": 1_000
            }),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        ),
    )
    .await
    .expect("Bash invocation exceeded its strict outer deadline");

    let pid = std::fs::read_to_string(&pid_file).expect("detached descendant pid file");
    assert!(result.is_error, "complete Windows job must time out");
    assert!(result.content.contains("timed out"), "{}", result.content);
    wait_until_windows_process_is_absent(pid.trim()).await;
}

#[tokio::test]
#[cfg(windows)]
async fn windows_timeout_kills_background_descendant() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("child.pid");
    let fixture = write_windows_descendant_fixture(&pid_file);
    let working_dir = dir.path().to_path_buf();
    let tool = BashTool {
        // 15s, not 3s: the test must observe the descendant ALIVE before the
        // timeout fires, and the pid-file wait below can take seconds under
        // load. Three seconds left a one-second probe window. This still proves
        // the descendant is killed at the timeout; it just no longer races the
        // machine. Mirrors `windows_hook_timeout_kills_descendant_process` in
        // archon-core, which was widened for exactly this reason.
        timeout_secs: 15,
        max_output_bytes: 1024,
        ..Default::default()
    };
    let command = windows_powershell_file_command(&fixture);
    let invocation = tokio::spawn(async move {
        tool.execute(
            json!({"command": command, "timeout": 15_000}),
            &ToolContext {
                working_dir,
                ..ToolContext::default()
            },
        )
        .await
    });

    let pid = wait_for_windows_pid_file(&pid_file).await;
    assert!(
        windows_process_exists(&pid),
        "descendant must be live before timeout"
    );
    assert!(
        !invocation.is_finished(),
        "invocation ended before timeout probe"
    );
    // Outer bound is a hang guard: the tool's own 15s timeout has already fired
    // by the time this is awaited, so this only has to exceed it plus teardown.
    let result = tokio::time::timeout(Duration::from_secs(30), invocation)
        .await
        .expect("Bash invocation exceeded outer deadline")
        .expect("Bash invocation task panicked");

    assert!(
        result.is_error,
        "command should time out: {}",
        result.content
    );
    wait_until_windows_process_is_absent(&pid).await;
}

#[cfg(windows)]
fn write_windows_detached_descendant_fixture(pid_file: &std::path::Path) -> std::path::PathBuf {
    let fixture = pid_file.with_extension("ps1");
    std::fs::write(
        &fixture,
        "\
$child = Start-Process powershell -ArgumentList @('-NoProfile', '-Command', 'Start-Sleep -Seconds 30') -PassThru -RedirectStandardOutput (Join-Path $PSScriptRoot 'detached.stdout') -RedirectStandardError (Join-Path $PSScriptRoot 'detached.stderr')
[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'detached-child.pid'), [string]$child.Id)
",
    )
    .unwrap();
    fixture
}

#[cfg(windows)]
fn write_windows_descendant_fixture(pid_file: &std::path::Path) -> std::path::PathBuf {
    let fixture = pid_file.with_extension("ps1");
    std::fs::write(
        &fixture,
        "\
$child = Start-Process powershell -ArgumentList @('-NoProfile', '-Command', 'Start-Sleep -Seconds 30') -PassThru
[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'child.pid'), [string]$child.Id)
Wait-Process -Id $child.Id
",
    )
    .unwrap();
    fixture
}

#[cfg(windows)]
fn windows_powershell_file_command(fixture: &std::path::Path) -> String {
    let path = fixture.display().to_string().replace('\'', "'\\''");
    format!("powershell -NoProfile -File '{path}'")
}

#[cfg(windows)]
async fn wait_for_windows_pid_file(pid_file: &std::path::Path) -> String {
    // Hang guard, not a speed assertion. It must stay comfortably UNDER the tool
    // timeout above: that test probes `!invocation.is_finished()` after this
    // returns, so the gap between this bound and the timeout IS the probe
    // window. At 2s against a 3s timeout the window was one second, and under
    // load this wait routinely outlasted the timeout it was supposed to precede.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(pid) = std::fs::read_to_string(pid_file) {
            let pid = pid.trim();
            if !pid.is_empty() {
                return pid.to_owned();
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            tokio::time::Instant::now() < deadline,
            "descendant pid file was not written: {}",
            pid_file.display()
        );
    }
}

#[cfg(windows)]
async fn wait_until_windows_process_is_absent(pid: &str) {
    // 20s, not 2s. Killing a process tree and having Windows reap it is not
    // bounded by anything this test controls, and this is a hang guard.
    for _ in 0..400 {
        if !windows_process_exists(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("background process survived Bash timeout: pid={pid}");
}

#[cfg(windows)]
fn windows_process_exists(pid: &str) -> bool {
    let script = format!(
        "if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ exit 0 }} else {{ exit 1 }}"
    );
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
#[cfg(windows)]
fn windows_bash_process_exists_reports_existing_and_absent_processes() {
    assert!(windows_process_exists(&std::process::id().to_string()));
    assert!(!windows_process_exists("4294967295"));
}

#[cfg(unix)]
async fn wait_until_process_is_absent(pid: &str) {
    let pid = pid.trim();
    for _ in 0..40 {
        if !process_exists(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("background process survived Bash timeout: pid={pid}");
}

#[cfg(unix)]
fn process_exists(pid: &str) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[tokio::test]
async fn final_content_bound_truncates_valid_utf8_output() {
    let result =
        execute_with_output_limit("printf 'abcdefghijklmnopqrstuvwxyz%.0s' $(seq 1 3)", 40).await;

    assert_within_output_limit(&result, 40);
    assert!(
        result.content.contains("Output truncated"),
        "{}",
        result.content
    );
}

#[tokio::test]
async fn final_content_bound_handles_invalid_utf8_expansion() {
    let result = execute_with_output_limit("printf '\\377%.0s' $(seq 1 64)", 40).await;

    assert_within_output_limit(&result, 40);
    assert!(
        result.content.contains("Output truncated"),
        "{}",
        result.content
    );
    assert!(result.content.is_char_boundary(result.content.len()));
}

#[tokio::test]
async fn final_content_bound_is_shared_by_stdout_and_stderr() {
    let result = execute_with_output_limit(
        "printf 'abcdefghijklmnopqrstuvwxyz'; printf 'abcdefghijklmnopqrstuvwxyz' >&2",
        48,
    )
    .await;

    assert_within_output_limit(&result, 48);
    assert!(
        result.content.contains("Output truncated"),
        "{}",
        result.content
    );
}

#[tokio::test]
async fn nonzero_exit_keeps_exit_code_prefix_within_output_bound() {
    let result = execute_with_output_limit(
        "printf 'abcdefghijklmnopqrstuvwxyz%.0s' $(seq 1 3); exit 7",
        64,
    )
    .await;

    assert!(result.is_error);
    assert!(
        result.content.starts_with("Exit code 7\n"),
        "{}",
        result.content
    );
    assert!(
        result.content.contains("Output truncated"),
        "{}",
        result.content
    );
    assert_within_output_limit(&result, 64);
}

#[tokio::test]
async fn zero_output_limit_returns_empty_content_without_panicking() {
    let result = execute_with_output_limit("printf output", 0).await;

    assert_eq!(result.content, "");
    assert_within_output_limit(&result, 0);
}

#[tokio::test]
async fn output_limit_smaller_than_marker_uses_a_bounded_indicator() {
    let result = execute_with_output_limit("printf abcdef", 5).await;

    assert_within_output_limit(&result, 5);
    assert!(result.content.ends_with("..."), "{}", result.content);
}

#[tokio::test]
async fn execution_deadline_reports_less_budget_after_elapsed_time() {
    let deadline = crate::execution_deadline::ExecutionDeadline::new(Duration::from_millis(50));
    tokio::time::sleep(Duration::from_millis(10)).await;

    assert!(deadline.remaining() < Duration::from_millis(50));
}

async fn execute_with_output_limit(command: &str, max_output_bytes: usize) -> ToolResult {
    let bash_path = which::which("bash").expect("test host must provide bash");
    let result = tokio::process::Command::new(bash_path)
        .arg("-c")
        .arg(command)
        .output()
        .await
        .expect("test command must run");
    let captured_limit = max_output_bytes.min(16);
    let stdout = CapturedOutput {
        truncated: result.stdout.len() > captured_limit,
        bytes: result.stdout.into_iter().take(captured_limit).collect(),
    };
    let stderr = CapturedOutput {
        truncated: result.stderr.len() > captured_limit,
        bytes: result.stderr.into_iter().take(captured_limit).collect(),
    };
    bash_result_from_pipes(
        max_output_bytes,
        stdout,
        stderr,
        result.status.code().unwrap_or(-1),
    )
}

fn assert_within_output_limit(result: &ToolResult, max_output_bytes: usize) {
    assert!(
        result.content.len() <= max_output_bytes,
        "{} bytes exceeded {max_output_bytes}: {:?}",
        result.content.len(),
        result.content
    );
}

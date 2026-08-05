//! Windows process-lifecycle tests for the Bash tool.
//!
//! Split from `bash_process_tests.rs` at the 500-line gate. The seam is the
//! platform: these spawn PowerShell, write `.ps1` fixtures, and reason about
//! Win32 process trees, which is a different problem from the unix
//! process-group tests they used to sit beside -- and their two failure modes
//! (a probe window narrower than the timeout it precedes, and PowerShell
//! resolved off PATH) are Windows-specific in a way none of the unix tests are.

use super::*;

#[tokio::test]
#[cfg(windows)]
async fn windows_bash_waits_for_descendant_after_parent_exits() {
    let dir = tempfile::tempdir().unwrap();
    let completion_file = dir.path().join("detached-child.complete");
    let fixture = write_windows_detached_descendant_fixture(&completion_file);
    let tool = BashTool {
        timeout_secs: 15,
        max_output_bytes: 1024,
        ..Default::default()
    };
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        tool.execute(
            json!({
                "command": windows_powershell_file_command(&fixture),
                "timeout": 15_000
            }),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        ),
    )
    .await
    .expect("Bash invocation exceeded its strict outer deadline");

    assert!(!result.is_error, "{}", result.content);
    let completion = std::fs::read_to_string(&completion_file)
        .expect("Bash returned before its Windows descendant completed");
    assert_eq!(completion, "complete");
}

#[tokio::test]
#[cfg(windows)]
async fn windows_timeout_kills_background_descendant() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("child.pid");
    let fixture = write_windows_descendant_fixture(&pid_file);
    let working_dir = dir.path().to_path_buf();
    let tool = BashTool {
        // 15s, not 3s. This test probes `!invocation.is_finished()` AFTER
        // waiting for the descendant's pid file, so the gap between that wait
        // and this timeout IS the probe window. At a 2s wait against a 3s
        // timeout the window was one second, and on a loaded runner the wait
        // routinely outlasted the timeout it was meant to precede: the
        // invocation had already ended, and the test failed claiming the
        // descendant machinery was broken when it had simply been raced.
        //
        // The identical defect in `archon-core`'s hook version was fixed after
        // it failed 4 runs in 6; the numbers here now match it. What the test
        // proves is unchanged -- the descendant is live before the timeout and
        // dead after it.
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
    // Outer deadline must clear the 15s tool timeout with room for teardown.
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
fn write_windows_detached_descendant_fixture(
    completion_file: &std::path::Path,
) -> std::path::PathBuf {
    let fixture = completion_file.with_extension("ps1");
    let shell = powershell_exe();
    std::fs::write(
        &fixture,
        format!(
            "\
$command = \"Start-Sleep -Milliseconds 3000; [IO.File]::WriteAllText((Join-Path '$PSScriptRoot' 'detached-child.complete'), 'complete')\"
Start-Process '{shell}' -ArgumentList @('-NoProfile', '-Command', $command) -RedirectStandardOutput (Join-Path $PSScriptRoot 'detached.stdout') -RedirectStandardError (Join-Path $PSScriptRoot 'detached.stderr')
"
        ),
    )
    .unwrap();
    fixture
}

#[cfg(windows)]
fn write_windows_descendant_fixture(pid_file: &std::path::Path) -> std::path::PathBuf {
    let fixture = pid_file.with_extension("ps1");
    let shell = powershell_exe();
    std::fs::write(
        &fixture,
        format!(
            "\
$child = Start-Process '{shell}' -ArgumentList @('-NoProfile', '-Command', 'Start-Sleep -Seconds 30') -PassThru
[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'child.pid'), [string]$child.Id)
Wait-Process -Id $child.Id
"
        ),
    )
    .unwrap();
    fixture
}

#[cfg(windows)]
fn windows_powershell_file_command(fixture: &std::path::Path) -> String {
    let path = fixture.display().to_string().replace('\'', "'\\''");
    let shell = powershell_exe().replace('\'', "'\\''");
    format!("'{shell}' -NoProfile -File '{path}'")
}

#[cfg(windows)]
async fn wait_for_windows_pid_file(pid_file: &std::path::Path) -> String {
    // Hang guard, not a speed assertion, and it must stay comfortably UNDER the
    // timeout of the test it precedes -- see
    // `windows_timeout_kills_background_descendant`.
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
    for _ in 0..40 {
        if !windows_process_exists(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("background process survived Bash timeout: pid={pid}");
}

#[cfg(windows)]
/// Absolute path to Windows PowerShell, or the bare name if it cannot be found.
///
/// Spawning `powershell` by name resolves in a normal Windows shell but not in
/// every environment this suite runs in: a Git Bash session carrying
/// System32 on PATH without the WindowsPowerShell subdirectory fails to resolve
/// it, `Command::status()` returns `Err`, and the poll below then reports a
/// process as gone that was never actually queried.
fn powershell_exe() -> String {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
    let absolute = std::path::Path::new(&system_root)
        .join("System32\\WindowsPowerShell\\v1.0\\powershell.exe");
    if absolute.is_file() {
        return absolute.display().to_string();
    }
    "powershell".to_string()
}

#[cfg(windows)]
fn windows_process_exists(pid: &str) -> bool {
    let script = format!(
        "if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ exit 0 }} else {{ exit 1 }}"
    );
    std::process::Command::new(powershell_exe())
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

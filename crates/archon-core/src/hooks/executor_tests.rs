use super::{HookConfig, HookOutcome, execute_hook};
use crate::hooks::{HookCommandType, HookFailurePolicy};

fn config(command: &str, timeout: u32) -> HookConfig {
    HookConfig {
        hook_type: HookCommandType::Command,
        command: command.to_owned(),
        if_condition: None,
        timeout: Some(timeout),
        once: None,
        r#async: None,
        async_rewake: None,
        status_message: None,
        headers: Default::default(),
        allowed_env_vars: Vec::new(),
        on_failure: Some(HookFailurePolicy::Block),
        enabled: true,
    }
}

#[tokio::test]
#[cfg(unix)]
async fn command_drains_pipes_before_writing_a_large_payload() {
    let dir = tempfile::tempdir().unwrap();
    let payload = vec![b'x'; 512 * 1024];
    let consumed = dir.path().join("consumed-payload");
    let output = super::executor_process::run_command(
        &unix_output_before_stdin_command(&consumed),
        &payload,
        dir.path(),
        "issue92-session",
        "PreToolUse",
        2,
    )
    .await
    .expect("output must drain while the payload is written");

    assert_eq!(std::fs::read(consumed).unwrap(), payload);
    assert!(output.stdout.len() + output.stderr.len() <= 64 * 1024);
}

#[cfg(unix)]
fn unix_output_before_stdin_command(consumed: &std::path::Path) -> String {
    format!(
        "head -c 131072 /dev/zero; head -c 131072 /dev/zero >&2; cat > {}",
        unix_shell_quote(consumed)
    )
}

#[tokio::test]
#[cfg(unix)]
async fn hook_environment_is_allowlisted_and_includes_explicit_context() {
    let output = execute_hook(
        &config(
            "printf '{\"outcome\":\"success\",\"additional_context\":\"%s|%s|%s|%s\"}' \"${ARCHON_SESSION_ID}\" \"${ARCHON_CWD}\" \"${ARCHON_HOOK_EVENT}\" \"${CARGO_MANIFEST_DIR:-missing}\"",
            2,
        ),
        &serde_json::json!({}),
        std::path::Path::new("/tmp"),
        "issue92-session",
        "PreToolUse",
    )
    .await;

    assert_eq!(output.outcome, HookOutcome::Success);
    assert_eq!(
        output.additional_context.as_deref(),
        Some("issue92-session|/tmp|PreToolUse|missing")
    );
}

#[tokio::test]
#[cfg(unix)]
async fn hook_parent_exit_with_descendant_held_pipes_hits_overall_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("held-pipes.pid");
    let started = std::time::Instant::now();
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(4),
        super::executor_process::run_command(
            &unix_descendant_holding_pipes_command(&pid_file),
            b"{}",
            dir.path(),
            "issue92-session",
            "PreToolUse",
            1,
        ),
    )
    .await
    .expect("hook invocation exceeded its strict outer deadline");

    assert!(matches!(output, Err(super::RunError::Timeout(_))));
    assert!(started.elapsed() < std::time::Duration::from_secs(4));
    wait_until_unix_process_is_absent(
        &std::fs::read_to_string(&pid_file).expect("descendant pid file"),
    )
    .await;
}

#[cfg(unix)]
fn unix_descendant_holding_pipes_command(pid_file: &std::path::Path) -> String {
    let path = unix_shell_quote(pid_file);
    format!(
        "sh -c \"trap '' HUP; echo \\$\\$ > {path}; exec sleep 30\" & while [ ! -s {path} ]; do sleep 0.01; done"
    )
}

#[cfg(unix)]
fn unix_shell_quote(path: &std::path::Path) -> String {
    format!(
        "'{}'",
        path.display().to_string().replace('\'', "'\\\"'\\\"'")
    )
}

#[cfg(unix)]
async fn wait_until_unix_process_is_absent(pid: &str) {
    let pid = pid.trim();
    for _ in 0..40 {
        if !unix_process_exists(pid) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("hook descendant survived timeout: pid={pid}");
}

#[cfg(unix)]
fn unix_process_exists(pid: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-0", pid])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[tokio::test]
#[cfg(unix)]
async fn hook_output_has_a_shared_bound_and_reports_truncation() {
    let output = execute_hook(
        &config(
            "yes stdout | head -c 131072; yes stderr | head -c 131072 >&2; exit 2",
            2,
        ),
        &serde_json::json!({}),
        std::path::Path::new("/tmp"),
        "issue92-session",
        "PreToolUse",
    )
    .await;

    assert_eq!(output.outcome, HookOutcome::Blocking);
    assert!(
        output
            .reason
            .unwrap_or_default()
            .contains("output truncated"),
        "a bounded hook result must identify truncation"
    );
}

#[tokio::test]
#[cfg(windows)]
async fn windows_hook_output_has_a_shared_bound_and_reports_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = write_windows_output_fixture(dir.path());
    // Hang guard, not a speed assertion. What this test proves is capture
    // semantics: exit code, the shared 64 KiB bound, and exactly one truncation
    // marker across both pipes. Process-tree cleanup under a *short* timeout has
    // its own dedicated tests, so nothing here depends on this number being
    // tight. It only has to be larger than a cold PowerShell start plus 256 KiB
    // pushed through two drained pipes. That was 2s, then 15s, and 15s still
    // lost on a loaded GitHub Windows runner — the failure is startup latency,
    // not a stall: stdin is closed by `write_payload`, and `drain_pipe` keeps
    // reading past budget exhaustion, so the child can never block on a pipe.
    let output = super::executor_process::run_command(
        &windows_file_command(&fixture),
        b"{}",
        dir.path(),
        "issue92-session",
        "PreToolUse",
        60,
    )
    .await
    .expect("Windows hook fixture should run");

    assert_eq!(output.exit_code, 2);
    assert!(output.stdout.len() + output.stderr.len() <= 64 * 1024);
    assert_eq!(
        output.stdout.matches("[hook output truncated").count()
            + output.stderr.matches("[hook output truncated").count(),
        1
    );
    assert!(
        output.stdout.contains("[hook output truncated")
            || output.stderr.contains("[hook output truncated")
    );
}

#[tokio::test]
#[cfg(windows)]
async fn windows_hook_uses_isolated_environment_and_explicit_context() {
    unsafe { std::env::set_var("ISSUE92_FORBIDDEN", "must-not-reach-hooks") };
    let dir = tempfile::tempdir().unwrap();
    let mut hook = config(&windows_environment_command(), 2);
    hook.hook_type = HookCommandType::Prompt;
    let output = execute_hook(
        &hook,
        &serde_json::json!({}),
        dir.path(),
        "issue92-session",
        "PreToolUse",
    )
    .await;
    unsafe { std::env::remove_var("ISSUE92_FORBIDDEN") };

    assert_eq!(output.outcome, HookOutcome::Success);
    let context = output.additional_context.unwrap();
    let parts: Vec<_> = context.split('|').collect();
    assert_eq!(parts[0], "issue92-session");
    assert_eq!(parts[2], "PreToolUse");
    assert_eq!(parts[3], "missing");
    assert!(parts[1].ends_with(dir.path().file_name().unwrap().to_str().unwrap()));
    assert!(parts[4..].iter().all(|value| !value.is_empty()));
}

#[tokio::test]
#[cfg(windows)]
async fn windows_hook_timeout_kills_descendant_process() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("descendant.pid");
    let command = windows_descendant_command(&pid_file);
    let working_dir = dir.path().to_path_buf();
    let invocation = tokio::spawn(async move {
        execute_hook(
            // 15s, not 3s: the test must observe the descendant ALIVE before the
            // timeout fires, and the pid-file wait above can take seconds under
            // load. Three seconds left a one-second probe window and this test
            // failed 4 runs in 6. The test still proves the descendant is killed
            // at the timeout; it just no longer races the machine.
            &config(&command, 15),
            &serde_json::json!({}),
            &working_dir,
            "issue92-session",
            "PreToolUse",
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
        "hook invocation ended before timeout probe"
    );
    // Outer bound is a hang guard: the hook's own timeout has already fired by
    // now, and this exists only so a hang fails rather than blocks the suite.
    let output = tokio::time::timeout(std::time::Duration::from_secs(60), invocation)
        .await
        .expect("hook invocation exceeded its outer deadline")
        .expect("hook invocation task panicked");

    assert_eq!(output.outcome, HookOutcome::Blocking);
    wait_until_process_is_absent(&pid).await;
}

#[cfg(windows)]
fn windows_environment_command() -> String {
    if archon_shell::resolve_shell().command_arg == "-c" {
        return "printf '%s|%s|%s|%s|%s|%s|%s|%s' \"$ARCHON_SESSION_ID\" \"$ARCHON_CWD\" \"$ARCHON_HOOK_EVENT\" \"${ISSUE92_FORBIDDEN:-missing}\" \"$SYSTEMROOT\" \"$COMSPEC\" \"$PATHEXT\" \"$USERPROFILE\"".to_owned();
    }
    "set \"forbidden=missing\" & if defined ISSUE92_FORBIDDEN set \"forbidden=%ISSUE92_FORBIDDEN%\" & call echo %ARCHON_SESSION_ID%^|%ARCHON_CWD%^|%ARCHON_HOOK_EVENT%^|%%forbidden%%^|%SYSTEMROOT%^|%COMSPEC%^|%PATHEXT%^|%USERPROFILE%".to_owned()
}

#[cfg(windows)]
fn windows_descendant_command(pid_file: &std::path::Path) -> String {
    let fixture = write_windows_descendant_fixture(pid_file);
    windows_file_command(&fixture)
}

#[cfg(windows)]
fn write_windows_descendant_fixture(pid_file: &std::path::Path) -> std::path::PathBuf {
    let fixture = pid_file.with_extension("ps1");
    std::fs::write(
        &fixture,
        "\
$child = Start-Process powershell -ArgumentList @('-NoProfile', '-Command', 'Start-Sleep -Seconds 30') -PassThru
[IO.File]::WriteAllText((Join-Path $PSScriptRoot 'descendant.pid'), [string]$child.Id)
Wait-Process -Id $child.Id
",
    )
    .unwrap();
    fixture
}

#[cfg(windows)]
fn write_windows_output_fixture(dir: &std::path::Path) -> std::path::PathBuf {
    let fixture = dir.join("large-output.ps1");
    std::fs::write(
        &fixture,
        "\
$null = [Console]::In.ReadToEnd()
$bytes = New-Object byte[] 131072
[Console]::OpenStandardOutput().Write($bytes, 0, $bytes.Length)
[Console]::OpenStandardError().Write($bytes, 0, $bytes.Length)
exit 2
",
    )
    .unwrap();
    fixture
}

#[cfg(windows)]
fn windows_file_command(fixture: &std::path::Path) -> String {
    let path = fixture.display().to_string();
    if archon_shell::resolve_shell().command_arg == "-c" {
        return format!(
            "powershell -NoProfile -File '{}'",
            path.replace('\'', "'\\''")
        );
    }
    format!("powershell -NoProfile -File \"{path}\"")
}

#[cfg(windows)]
async fn wait_for_windows_pid_file(pid_file: &std::path::Path) -> String {
    // Hang guard, not a speed assertion. It must stay comfortably UNDER the
    // hook timeout in `windows_hook_timeout_kills_descendant_process`: that test
    // probes `!invocation.is_finished()` after this returns, so the gap between
    // this bound and the hook timeout IS the probe window. At 2s against a 3s
    // timeout the window was one second, and under full-suite load the wait
    // routinely outlasted the timeout it was supposed to precede.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Ok(pid) = std::fs::read_to_string(pid_file) {
            let pid = pid.trim();
            if !pid.is_empty() {
                return pid.to_owned();
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            tokio::time::Instant::now() < deadline,
            "descendant pid file was not written: {}",
            pid_file.display()
        );
    }
}

#[cfg(windows)]
async fn wait_until_process_is_absent(pid: &str) {
    // 20s, not 2s. Killing a process tree and having Windows reap it is not
    // bounded by anything this test controls, and this is a hang guard.
    for _ in 0..400 {
        if !windows_process_exists(pid) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("hook descendant survived timeout: pid={pid}");
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
fn windows_hook_process_exists_reports_existing_and_absent_processes() {
    assert!(windows_process_exists(&std::process::id().to_string()));
    assert!(!windows_process_exists("4294967295"));
}

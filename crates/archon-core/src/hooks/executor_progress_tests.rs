use super::executor_process::run_command;
use super::{HookConfig, HookOutcome, RunError, execute_hook};

fn config(command: &str, allow: bool) -> HookConfig {
    serde_json::from_value(serde_json::json!({
        "type": "command", "command": command, "timeout": 1,
        "on_failure": if allow { "allow" } else { "block" }
    }))
    .unwrap()
}

async fn run(command: &str, payload: &[u8]) -> super::CommandOutput {
    let dir = tempfile::tempdir().unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        run_command(command, payload, dir.path(), "progress", "PreToolUse", 5),
    )
    .await
    .expect("hang guard")
    .expect("output must renew the no-progress window")
}

#[tokio::test]
async fn stdout_progress_outlives_the_original_window() {
    let output = run(
        "for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do printf x; sleep 0.3; done",
        b"{}",
    )
    .await;
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stdout, "x".repeat(20));
}

#[tokio::test]
async fn discarded_stderr_still_counts_as_progress() {
    let output = run(
        "head -c 131072 /dev/zero >&2; for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do printf x >&2; sleep 0.3; done",
        b"{}",
    )
    .await;
    assert_eq!(output.exit_code, 0);
    assert!(output.stderr.contains("truncated"));
    assert!(output.stdout.len() + output.stderr.len() <= 65536);
}

#[tokio::test]
async fn descendant_output_renews_window_after_parent_exit() {
    let output = run(
        "sh -c 'for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do printf x; sleep 0.3; done' &",
        b"{}",
    )
    .await;
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stdout, "x".repeat(20));
}

#[tokio::test]
async fn output_renews_window_while_stdin_is_blocked() {
    let output = run(
        "for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do printf x; sleep 0.3; done; cat >/dev/null",
        &vec![b'x'; 512 * 1024],
    )
    .await;
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stdout, "x".repeat(20));
}

#[tokio::test]
async fn stalled_hook_reports_no_progress_under_both_failure_policies() {
    let dir = tempfile::tempdir().unwrap();
    for allow in [false, true] {
        let result = execute_hook(
            &config("printf x; sleep 3", allow),
            &serde_json::json!({}),
            dir.path(),
            "progress",
            "PreToolUse",
        )
        .await;
        assert_eq!(
            result.outcome,
            if allow {
                HookOutcome::NonBlockingError
            } else {
                HookOutcome::Blocking
            }
        );
        assert!(
            result
                .reason
                .as_deref()
                .unwrap_or_default()
                .contains("no progress"),
            "{result:?}"
        );
        if !allow {
            assert!(result.reason.as_deref().unwrap().contains("stopped"));
        }
    }
}

#[tokio::test]
#[cfg(unix)]
async fn blocked_stalls_never_expose_command_arguments() {
    let dir = tempfile::tempdir().unwrap();
    for command in [
        "curl -H 'Authorization: Bearer secret-one' https://example.invalid",
        "TOKEN=secret-two curl https://example.invalid",
        "'secret-three' command",
    ] {
        let result = execute_hook(
            &config(&format!("printf x; sleep 3 # {command}"), false),
            &serde_json::json!({}),
            dir.path(),
            "progress",
            "PreToolUse",
        )
        .await;
        let reason = result.reason.unwrap_or_default();
        assert!(reason.contains("no progress"), "{reason}");
        assert!(!reason.contains("secret-"), "secret leaked: {reason}");
    }
}

#[tokio::test]
async fn exit_two_fallback_reasons_redact_command_arguments() {
    let dir = tempfile::tempdir().unwrap();
    for command in [
        // Each reads its input first: an unsuccessful hook that exits before
        // the payload is written keeps its broken pipe (hooks_tests.rs
        // `unsuccessful_hook_cannot_hide_stdin_broken_pipe`).
        "cat >/dev/null; exit 2 # curl -H 'Authorization: Bearer secret-one'",
        "cat >/dev/null; exit 2 # TOKEN=secret-two command",
        "cat >/dev/null; exit 2 # 'secret-three' command",
    ] {
        let result = execute_hook(
            &config(command, false),
            &serde_json::json!({}),
            dir.path(),
            "redaction",
            "PreToolUse",
        )
        .await;
        assert_eq!(result.outcome, HookOutcome::Blocking);
        let reason = result.reason.unwrap_or_default();
        assert!(reason.contains("blocked tool execution"), "{reason}");
        assert!(!reason.contains("secret-"), "secret leaked: {reason}");
    }
}

#[tokio::test]
async fn silent_descendant_is_killed_with_no_progress_reason() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("child.pid");
    let command = format!("sleep 30 & echo $! > '{}'", pid_file.display());
    let result = run_command(&command, b"{}", dir.path(), "progress", "PreToolUse", 5).await;
    let pid: i32 = std::fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // The executor must clean up even after the shell has already exited.
    for _ in 0..40 {
        if unsafe { libc::kill(pid, 0) } != 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "descendant survived");
    assert!(matches!(result, Err(RunError::Timeout(_))));
    assert!(result.unwrap_err().to_string().contains("no progress"));
}

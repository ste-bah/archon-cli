use std::time::Duration;

use serde_json::json;

use super::bash_output::CapturedOutput;

use super::bash_process::bash_result_from_pipes;
use super::*;
use crate::tool::ToolContext;

#[tokio::test]
#[cfg(unix)]
async fn parent_exit_with_descendant_held_pipes_is_cleaned_before_result() {
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
        !result.is_error,
        "completed shell with cleaned descendants should succeed: {}",
        result.content
    );
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
async fn normal_completion_kills_delayed_background_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let delayed_file = dir.path().join("delayed-write");
    let tool = BashTool {
        max_output_bytes: 1024,
        ..Default::default()
    };

    let result = tool
        .execute(
            json!({
                "command": format!(
                    "(sleep 0.2; printf delayed > {}) &",
                    shell_quote(&delayed_file)
                )
            }),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !delayed_file.exists(),
        "a detached mutation escaped Bash completion"
    );
}

#[tokio::test]
async fn stderr_redirection_and_heredoc_are_not_background_commands() {
    let dir = tempfile::tempdir().unwrap();
    let tool = BashTool::default();
    let result = tool
        .execute(
            json!({"command": "cat <<'EOF' >&2\nheredoc stderr\nEOF"}),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("heredoc stderr"));
}

#[tokio::test]
#[cfg(unix)]
async fn parent_completion_returns_after_descendant_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("cleanup-before-return.pid");
    let tool = BashTool {
        timeout_secs: 1,
        max_output_bytes: 1024,
        ..Default::default()
    };
    let result = tool
        .execute(
            json!({"command": descendant_holding_pipes_command(&pid_file), "timeout": 1_000}),
            &ToolContext {
                working_dir: dir.path().to_path_buf(),
                ..ToolContext::default()
            },
        )
        .await;

    assert!(!result.is_error, "{}", result.content);
    let pid = std::fs::read_to_string(pid_file).unwrap();
    // Bounded poll rather than an immediate check.
    //
    // `terminate_child` SIGKILLs the process GROUP and then waits for the direct
    // child. The descendant is signalled but reaped asynchronously by the
    // kernel, so it can still be visible for a few milliseconds after `execute`
    // returns. That window is invisible on an idle machine and reliably lost
    // under CI load, which is what made this test flaky on ubuntu.
    //
    // The guarantee worth asserting is that the descendant does not SURVIVE --
    // SIGKILL cannot be trapped, so anything still present is mid-teardown, not
    // leaked. This still fails if cleanup never happens, which is the regression
    // the test exists to catch; it only tolerates teardown latency.
    wait_until_process_is_absent(&pid).await;
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
    if std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            stat.rsplit_once(") ")
                .map(|(_, fields)| fields.starts_with('Z'))
        })
        .unwrap_or(false)
    {
        return false;
    }
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

// Both tests below repeat a format string to overflow the output limit. They
// used `$(seq 1 N)` to generate the repetitions, which made them depend on
// coreutils being on the shell's PATH. The Bash tool runs with a sanitized
// environment, so on Windows — where `seq` lives in Git's `usr\bin` and is not
// on the Windows PATH the tool passes through — the command exited 127 and
// produced too little output to truncate. The failure read as "truncation
// marker missing" rather than "seq not found".
//
// Brace expansion is a bash builtin, so the repetition count no longer depends
// on anything outside the shell. Same counts, same expected output.

#[tokio::test]
async fn final_content_bound_truncates_valid_utf8_output() {
    let result =
        execute_with_output_limit("printf 'abcdefghijklmnopqrstuvwxyz%.0s' {1..3}", 40).await;

    assert_within_output_limit(&result, 40);
    assert!(
        result.content.contains("Output truncated"),
        "{}",
        result.content
    );
}

#[tokio::test]
async fn final_content_bound_handles_invalid_utf8_expansion() {
    let result = execute_with_output_limit("printf '\\377%.0s' {1..64}", 40).await;

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
        read_error: None,
    };
    let stderr = CapturedOutput {
        truncated: result.stderr.len() > captured_limit,
        bytes: result.stderr.into_iter().take(captured_limit).collect(),
        read_error: None,
    };
    bash_result_from_pipes(
        max_output_bytes,
        &ToolContext::default(),
        command,
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

#[cfg(windows)]
#[path = "bash_process_windows_tests.rs"]
mod windows;

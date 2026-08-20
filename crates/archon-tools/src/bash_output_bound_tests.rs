//! Output-bound behaviour of a bash tool result.
//!
//! Split from `bash_process_tests.rs` for the 500-line ceiling. These assert
//! what a result may CONTAIN — truncation markers, the shared stdout/stderr
//! budget, exit-code prefixes — as opposed to how the child process is
//! contained, which stays with the containment tests.

use std::time::Duration;

use super::bash_output::CapturedOutput;
use super::bash_process::bash_result_from_pipes;
use super::*;
use crate::tool::ToolContext;

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

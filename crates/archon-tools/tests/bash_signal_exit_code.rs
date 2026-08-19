//! A process killed by a signal must not report the same code as one that
//! exited with -1 (#193).
//!
//! `ExitStatus::code()` is `None` on Unix for a signalled process, and
//! `unwrap_or(-1)` collapsed every signal into one number: an OOM kill and a
//! segfault came back identical, and neither was distinguishable from a command
//! that genuinely returned -1.
//!
//! Unix only. Windows has no signals, `code()` is always `Some`, and the
//! behaviour there is unchanged.

#![cfg(unix)]

use archon_tools::tool::{Tool, ToolContext};

/// `128 + signal` is what every shell reports in `$?`, so the number the model
/// sees is the number the same command would have shown at a prompt.
#[tokio::test]
async fn a_process_killed_by_a_signal_reports_128_plus_the_signal() {
    let tool = archon_tools::bash::BashTool::default();
    let ctx = ToolContext {
        working_dir: std::env::temp_dir(),
        ..Default::default()
    };

    // SIGKILL is 9, so the shell convention is 137. The subshell kills itself
    // rather than relying on an outside signaller, which would race.
    let result = tool
        .execute(
            serde_json::json!({ "command": "kill -9 $$" }),
            &ctx,
        )
        .await;

    assert!(
        result.content.contains("137"),
        "a SIGKILLed process must report 137, not a collapsed -1: {}",
        result.content
    );
    assert!(
        !result.content.contains("-1"),
        "the signal was collapsed into -1: {}",
        result.content
    );
}

/// The ordinary path is untouched: a real non-zero exit still reports itself.
#[tokio::test]
async fn an_ordinary_failure_still_reports_its_own_code() {
    let tool = archon_tools::bash::BashTool::default();
    let ctx = ToolContext {
        working_dir: std::env::temp_dir(),
        ..Default::default()
    };

    let result = tool
        .execute(serde_json::json!({ "command": "exit 3" }), &ctx)
        .await;

    assert!(
        result.content.contains('3'),
        "exit 3 must report 3: {}",
        result.content
    );
}

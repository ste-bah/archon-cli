//! The run id must reach a real command, not merely the helper that builds the
//! environment.
//!
//! A task can declare the run's own identifier in `required_env_keys`, and a
//! verification branch fails outright when a declared key is absent. Nothing
//! set it, so a run reached its terminal task and failed on a value it was
//! itself holding. These tests execute the Bash tool for real and read the
//! variable back, because a unit test over the env-building helper would pass
//! even if nothing called it.
//!
//! Every test here reads the variable back with `printf '%s' "$VAR"`, which is
//! POSIX-shell syntax, so the whole target is `cfg(not(windows))` rather than
//! three separately gated tests around ungated imports. Stated at file level
//! for the reason `bash_signal_exit_code.rs` and `evidence_tools.rs` state
//! theirs: a reader has to be able to see that this binary reports
//! `0 passed` on Windows on purpose. It also silences the four dead-code and
//! unused-import warnings the per-test gates left behind there.
#![cfg(not(windows))]

use serde_json::json;

use archon_tools::bash::BashTool;
use archon_tools::tool::{Tool, ToolContext};

fn ctx_for_session(session_id: &str) -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: session_id.into(),
        mode: archon_tools::tool::AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    }
}

#[tokio::test]
async fn a_command_in_a_workflow_run_can_read_the_run_id() {
    let result = BashTool::default()
        .execute(
            json!({ "command": "printf '%s' \"$ARCHON_WORKFLOW_RUN_ID\"" }),
            &ctx_for_session("wf-9d0b7ff3-5d5c-4609-8617-4b75d06d4939"),
        )
        .await;

    assert!(
        result
            .content
            .contains("wf-9d0b7ff3-5d5c-4609-8617-4b75d06d4939"),
        "run id must reach the command, got: {}",
        result.content
    );
}

/// The project alias is the whole point: a task set names its own key and gets
/// the same value, without that name existing in engine code.
#[tokio::test]
async fn a_project_alias_reaches_the_command_too() {
    let tool = BashTool {
        run_id_env_aliases: vec!["PROJECT_REVIEW_RUN_ID".to_string()],
        ..BashTool::default()
    };
    let result = tool
        .execute(
            json!({ "command": "printf '%s' \"$PROJECT_REVIEW_RUN_ID\"" }),
            &ctx_for_session("wf-abc123"),
        )
        .await;

    assert!(
        result.content.contains("wf-abc123"),
        "alias must reach the command, got: {}",
        result.content
    );
}

/// An ordinary interactive session has no run id and must not be handed a
/// misleading one.
#[tokio::test]
async fn an_interactive_session_gets_no_run_id() {
    let result = BashTool::default()
        .execute(
            json!({ "command": "printf '[%s]' \"$ARCHON_WORKFLOW_RUN_ID\"" }),
            &ctx_for_session("sess-not-a-run"),
        )
        .await;

    assert!(
        result.content.contains("[]"),
        "expected an empty value, got: {}",
        result.content
    );
}

use serde_json::json;

use archon_tools::ask_user::AskUserTool;
use archon_tools::plan_mode::{EnterPlanModeTool, ExitPlanModeTool, is_tool_allowed_in_mode};
use archon_tools::tool::{AgentMode, Tool, ToolContext};

fn ctx(mode: AgentMode) -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "test-plan".into(),
        mode,
    }
}

// -----------------------------------------------------------------------
// Plan mode tool restriction
// -----------------------------------------------------------------------

#[test]
fn read_allowed_in_plan_mode() {
    assert!(is_tool_allowed_in_mode("Read", AgentMode::Plan));
}

#[test]
fn glob_allowed_in_plan_mode() {
    assert!(is_tool_allowed_in_mode("Glob", AgentMode::Plan));
}

#[test]
fn grep_allowed_in_plan_mode() {
    assert!(is_tool_allowed_in_mode("Grep", AgentMode::Plan));
}

#[test]
fn ask_user_allowed_in_plan_mode() {
    assert!(is_tool_allowed_in_mode("AskUserQuestion", AgentMode::Plan));
}

#[test]
fn write_blocked_in_plan_mode() {
    assert!(!is_tool_allowed_in_mode("Write", AgentMode::Plan));
}

#[test]
fn edit_blocked_in_plan_mode() {
    assert!(!is_tool_allowed_in_mode("Edit", AgentMode::Plan));
}

#[test]
fn bash_blocked_in_plan_mode() {
    assert!(!is_tool_allowed_in_mode("Bash", AgentMode::Plan));
}

#[test]
fn all_tools_allowed_in_normal_mode() {
    for tool in &[
        "Read",
        "Write",
        "Edit",
        "Bash",
        "Glob",
        "Grep",
        "AskUserQuestion",
    ] {
        assert!(
            is_tool_allowed_in_mode(tool, AgentMode::Normal),
            "{tool} should be allowed in normal mode"
        );
    }
}

// -----------------------------------------------------------------------
// EnterPlanMode tool
// -----------------------------------------------------------------------

#[tokio::test]
async fn enter_plan_mode_succeeds() {
    let tool = EnterPlanModeTool;
    let result = tool.execute(json!({}), &ctx(AgentMode::Normal)).await;
    assert!(!result.is_error);
    assert!(result.content.contains("Plan mode entered"));
}

#[tokio::test]
async fn enter_plan_mode_idempotent() {
    let tool = EnterPlanModeTool;
    // Already in plan mode -- should still succeed
    let result = tool.execute(json!({}), &ctx(AgentMode::Plan)).await;
    assert!(!result.is_error);
}

// -----------------------------------------------------------------------
// ExitPlanMode tool
// -----------------------------------------------------------------------

#[tokio::test]
async fn exit_plan_mode_succeeds() {
    let tool = ExitPlanModeTool;
    let result = tool.execute(json!({}), &ctx(AgentMode::Plan)).await;
    assert!(!result.is_error);
    assert!(result.content.contains("Plan mode exited"));
}

#[tokio::test]
async fn exit_plan_mode_without_enter_errors() {
    let tool = ExitPlanModeTool;
    let result = tool.execute(json!({}), &ctx(AgentMode::Normal)).await;
    assert!(result.is_error);
    assert!(result.content.contains("Not in plan mode"));
}

// -----------------------------------------------------------------------
// AskUserQuestion tool
// -----------------------------------------------------------------------

#[tokio::test]
async fn ask_user_basic_question() {
    let tool = AskUserTool;
    let result = tool
        .execute(
            json!({ "question": "What is your name?" }),
            &ctx(AgentMode::Normal),
        )
        .await;
    assert!(!result.is_error);
    assert!(result.content.contains("What is your name?"));
}

#[tokio::test]
async fn ask_user_with_choices() {
    let tool = AskUserTool;
    let result = tool
        .execute(
            json!({
                "question": "Pick one:",
                "choices": ["Option A", "Option B", "Option C"]
            }),
            &ctx(AgentMode::Normal),
        )
        .await;
    assert!(!result.is_error);
    assert!(result.content.contains("1. Option A"));
    assert!(result.content.contains("2. Option B"));
    assert!(result.content.contains("3. Option C"));
}

#[tokio::test]
async fn ask_user_missing_question() {
    let tool = AskUserTool;
    let result = tool.execute(json!({}), &ctx(AgentMode::Normal)).await;
    assert!(result.is_error);
    assert!(result.content.contains("question"));
}

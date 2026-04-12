//! Smoke test: SendMessage tool end-to-end execution.
//!
//! Exercises the full tool interface (execute → JSON → deserialize) to verify
//! the SendMessage tool works as a real tool invocation, not just unit-level validation.

use archon_tools::send_message::{SendMessageRequest, SendMessageTool};
use archon_tools::tool::{AgentMode, Tool, ToolContext};
use serde_json::json;

fn smoke_ctx() -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "smoke-session-001".into(),
        mode: AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    }
}

/// Full round-trip: valid input → execute → parse JSON output → verify fields.
#[tokio::test]
async fn smoke_send_message_valid_round_trip() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "explorer-agent",
        "message": "Search for all TODO comments in src/",
        "summary": "Search TODOs in src"
    });

    // Execute the tool (real call, not just validation)
    let result = tool.execute(input, &smoke_ctx()).await;
    assert!(!result.is_error, "smoke test failed: {}", result.content);

    // Parse the JSON output back into a SendMessageRequest
    let request: SendMessageRequest =
        serde_json::from_str(&result.content).expect("output must be valid JSON");

    assert_eq!(request.to, "explorer-agent");
    assert_eq!(request.message, "Search for all TODO comments in src/");
    assert_eq!(request.summary.as_deref(), Some("Search TODOs in src"));
}

/// Verify guard rejects broadcast target.
#[tokio::test]
async fn smoke_broadcast_rejected() {
    let tool = SendMessageTool;
    let input = json!({ "to": "*", "message": "hi all", "summary": "broadcast" });
    let result = tool.execute(input, &smoke_ctx()).await;
    assert!(result.is_error, "broadcast must be rejected");
}

/// Verify guard rejects parent session targeting.
#[tokio::test]
async fn smoke_parent_session_rejected() {
    let tool = SendMessageTool;
    let ctx = smoke_ctx();
    let input = json!({ "to": ctx.session_id, "message": "hi parent", "summary": "greet parent" });
    let result = tool.execute(input, &ctx).await;
    assert!(result.is_error, "parent session must be rejected");
}

/// Verify missing summary is rejected for string messages.
#[tokio::test]
async fn smoke_missing_summary_rejected() {
    let tool = SendMessageTool;
    let input = json!({ "to": "agent-1", "message": "do stuff" });
    let result = tool.execute(input, &smoke_ctx()).await;
    assert!(result.is_error, "missing summary must be rejected");
    assert!(result.content.contains("summary"));
}

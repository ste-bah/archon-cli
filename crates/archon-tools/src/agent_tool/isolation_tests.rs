//! The Agent tool's `isolation` argument goes through the shared parser
//! (#236), and its schema offers exactly what that parser accepts.

use super::*;
use crate::tool::{Tool, ToolContext};
use serde_json::json;

fn make_ctx() -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "test-session".into(),
        ..Default::default()
    }
}

#[tokio::test]
async fn invalid_isolation_returns_error() {
    let tool = AgentTool::new();
    let result = tool
        .execute(
            json!({"prompt": "Read the code", "isolation": "inplace"}),
            &make_ctx(),
        )
        .await;

    assert!(result.is_error);
    assert!(
        result.content.contains("unknown isolation value 'inplace'")
            && result
                .content
                .contains("the Agent tool's isolation argument"),
        "{}",
        result.content
    );
}

#[test]
fn schema_includes_isolation() {
    let tool = AgentTool::new();
    let schema = tool.input_schema();
    let props = schema["properties"].as_object().unwrap();
    assert!(props.contains_key("isolation"));
    assert_eq!(props["isolation"]["type"], "string");
    assert_eq!(props["isolation"]["enum"][0], "none");
    assert_eq!(props["isolation"]["enum"][1], "worktree");
    // Every rung of the ladder has to be askable. #184 M3 added the third tier
    // and left the schema at two, so the only way to reach it was an agent
    // definition — a tier the tool refuses to accept is a tier that does not
    // exist for anyone calling the tool.
    assert_eq!(props["isolation"]["enum"][2], "worktree-with-builds");

    assert_eq!(props["isolation"]["enum"][3], "workspace-boundary");

    // The schema offers exactly what the tool accepts (#236): each value is
    // taken by the tool itself, and nothing outside the list is.
    let tool = AgentTool::new();
    let offered = props["isolation"]["enum"].as_array().unwrap();
    assert_eq!(offered.len(), crate::isolation::Isolation::ALL.len());
    for value in offered {
        let raw = value.as_str().unwrap();
        let request = tool
            .validate_and_build(&json!({"prompt": "x", "isolation": raw}))
            .unwrap_or_else(|e| panic!("the schema offers '{raw}' but the tool rejects it: {e}"));
        if raw != "none" {
            assert_eq!(request.isolation.as_deref(), Some(raw));
        }
    }
}

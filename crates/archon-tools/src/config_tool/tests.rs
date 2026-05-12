use serde_json::json;

use super::*;
use crate::tool::{AgentMode, ToolContext};

fn test_ctx() -> ToolContext {
    ToolContext {
        working_dir: std::env::temp_dir(),
        session_id: "test-config".into(),
        mode: AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    }
}

/// Clear the overlay between tests to avoid cross-contamination.
fn clear_overlay() {
    let mut guard = OVERLAY.lock().expect("lock poisoned");
    guard.clear();
}

#[test]
fn metadata() {
    let tool = ConfigTool;
    assert_eq!(tool.name(), "Config");
    assert!(!tool.description().is_empty());

    let schema = tool.input_schema();
    let required = schema["required"].as_array().expect("required array");
    assert!(required.iter().any(|v| v == "action"));
    assert!(required.iter().any(|v| v == "key"));
}

#[test]
fn permission_safe_for_get_risky_for_set() {
    let tool = ConfigTool;
    assert_eq!(
        tool.permission_level(&json!({"action": "get", "key": "api.default_model"})),
        PermissionLevel::Safe,
    );
    assert_eq!(
        tool.permission_level(&json!({"action": "set", "key": "api.default_model", "value": "x"})),
        PermissionLevel::Risky,
    );
}

#[tokio::test]
async fn get_returns_default_when_no_overlay() {
    clear_overlay();
    let tool = ConfigTool;
    let result = tool
        .execute(
            json!({"action": "get", "key": "api.default_model"}),
            &test_ctx(),
        )
        .await;
    assert!(!result.is_error);
    assert!(result.content.contains("claude-sonnet-4-6"));
    assert!(result.content.contains("default"));
}

#[tokio::test]
async fn set_then_get_returns_overlay_value() {
    clear_overlay();
    let tool = ConfigTool;
    let ctx = test_ctx();

    let set_res = tool
        .execute(
            json!({"action": "set", "key": "api.default_model", "value": "claude-opus-4-7"}),
            &ctx,
        )
        .await;
    assert!(!set_res.is_error, "set failed: {}", set_res.content);

    let get_res = tool
        .execute(json!({"action": "get", "key": "api.default_model"}), &ctx)
        .await;
    assert!(!get_res.is_error);
    assert!(get_res.content.contains("claude-opus-4-7"));
    assert!(get_res.content.contains("overlay"));
}

#[tokio::test]
async fn set_numeric_rejects_non_numeric() {
    clear_overlay();
    let tool = ConfigTool;
    let result = tool
        .execute(
            json!({"action": "set", "key": "api.thinking_budget", "value": "not-a-number"}),
            &test_ctx(),
        )
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("expected u32"));
}

#[tokio::test]
async fn personality_key_is_read_only() {
    let tool = ConfigTool;
    let result = tool
        .execute(
            json!({"action": "set", "key": "personality.name", "value": "evil"}),
            &test_ctx(),
        )
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("read-only"));
}

#[tokio::test]
async fn unknown_key_suggests_alternatives() {
    let tool = ConfigTool;
    let result = tool
        .execute(json!({"action": "get", "key": "api.model"}), &test_ctx())
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("Unknown config key"));
    // Should suggest api.default_model since "model" is a substring.
    assert!(result.content.contains("api.default_model"));
}

#[tokio::test]
async fn missing_action_returns_error() {
    let tool = ConfigTool;
    let result = tool
        .execute(json!({"key": "api.default_model"}), &test_ctx())
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("action"));
}

#[tokio::test]
async fn set_without_value_returns_error() {
    let tool = ConfigTool;
    let result = tool
        .execute(
            json!({"action": "set", "key": "api.default_model"}),
            &test_ctx(),
        )
        .await;
    assert!(result.is_error);
    assert!(result.content.contains("value"));
}

#[test]
fn get_config_value_helper() {
    clear_overlay();
    assert_eq!(
        get_config_value("tools.bash_timeout"),
        Some("120".to_string()),
    );
    assert!(get_config_value("nonexistent.key").is_none());
}

use super::*;

// --- Schema tests ---

#[test]
fn tool_name_is_send_message() {
    let tool = SendMessageTool;
    assert_eq!(tool.name(), "SendMessage");
}

#[test]
fn schema_requires_to_but_not_summary_or_message() {
    let tool = SendMessageTool;
    let schema = tool.input_schema();
    assert_eq!(schema["type"], "object");

    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("to")), "schema must require 'to'");
    // After TASK-T2 (G2) `message` is only required at runtime for text messages
    // (structured message types carry their payload via request_id/approve/etc).
    assert!(
        !required.contains(&json!("message")),
        "message must NOT be in schema-required (runtime-required for text only)"
    );
    // summary is schema-OPTIONAL — must NOT be in required
    assert!(
        !required.contains(&json!("summary")),
        "summary must NOT be in required (schema-optional)"
    );
}

#[test]
fn schema_has_summary_property() {
    let tool = SendMessageTool;
    let schema = tool.input_schema();
    let props = schema["properties"].as_object().unwrap();
    assert!(
        props.contains_key("summary"),
        "schema must define summary property"
    );
    assert_eq!(props["summary"]["type"], "string");
}

#[test]
fn permission_level_is_risky() {
    let tool = SendMessageTool;
    assert_eq!(tool.permission_level(&json!({})), PermissionLevel::Risky);
}

// --- Valid input tests ---

#[tokio::test]
async fn valid_input_returns_send_message_request() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "agent-42",
        "message": "Please review the parser module",
        "summary": "Review parser module"
    });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(!result.is_error, "unexpected error: {}", result.content);

    let request: SendMessageRequest =
        serde_json::from_str(&result.content).expect("should deserialize");
    assert_eq!(request.to, "agent-42");
    assert_eq!(request.message, "Please review the parser module");
    assert_eq!(request.summary.as_deref(), Some("Review parser module"));
}

#[tokio::test]
async fn valid_request_round_trips_through_json() {
    let request = SendMessageRequest {
        to: "test-agent".into(),
        message: "test message".into(),
        summary: Some("test summary".into()),
        message_type: default_message_type(),
        request_id: None,
        approve: None,
        reason: None,
        feedback: None,
    };

    let json_str = serde_json::to_string(&request).expect("serialize");
    let deserialized: SendMessageRequest = serde_json::from_str(&json_str).expect("deserialize");
    assert_eq!(request, deserialized);
}

#[tokio::test]
async fn request_without_summary_serializes_without_field() {
    let request = SendMessageRequest {
        to: "test-agent".into(),
        message: "test message".into(),
        summary: None,
        message_type: default_message_type(),
        request_id: None,
        approve: None,
        reason: None,
        feedback: None,
    };

    let json_str = serde_json::to_string(&request).expect("serialize");
    assert!(
        !json_str.contains("summary"),
        "None summary should be skipped in serialization"
    );
}

// --- Missing/empty field tests ---

#[tokio::test]
async fn missing_to_returns_error() {
    let tool = SendMessageTool;
    let input = json!({
        "message": "hello",
        "summary": "greeting"
    });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(result.is_error);
    assert!(
        result.content.contains("to"),
        "error should mention 'to': {}",
        result.content
    );
}

#[tokio::test]
async fn empty_to_returns_error() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "   ",
        "message": "hello",
        "summary": "greeting"
    });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(result.is_error);
    assert!(
        result.content.contains("to"),
        "error should mention 'to': {}",
        result.content
    );
}

#[tokio::test]
async fn missing_message_returns_error() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "agent-1",
        "summary": "greeting"
    });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(result.is_error);
    assert!(
        result.content.contains("message"),
        "error should mention 'message': {}",
        result.content
    );
}

#[tokio::test]
async fn empty_message_returns_error() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "agent-1",
        "message": "",
        "summary": "greeting"
    });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(result.is_error);
    assert!(
        result.content.contains("message"),
        "error should mention 'message': {}",
        result.content
    );
}

// --- Summary validation (schema-optional, validation-required) ---

#[tokio::test]
async fn missing_summary_returns_error_for_string_message() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "agent-1",
        "message": "Please review this code"
    });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(result.is_error, "missing summary should be an error");
    assert!(
        result.content.contains("summary"),
        "error should mention summary: {}",
        result.content
    );
}

#[tokio::test]
async fn empty_summary_returns_error() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "agent-1",
        "message": "Please review this code",
        "summary": "   "
    });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(
        result.is_error,
        "whitespace-only summary should be an error"
    );
    assert!(
        result.content.contains("summary"),
        "error should mention summary: {}",
        result.content
    );
}

// --- Early guards ---

#[tokio::test]
async fn broadcast_target_star_rejected() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "*",
        "message": "hello everyone",
        "summary": "broadcast greeting"
    });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(result.is_error, "broadcast should be rejected");
    assert!(
        result.content.contains("Broadcast") || result.content.contains("broadcast"),
        "error should mention broadcast: {}",
        result.content
    );
}

#[tokio::test]
async fn parent_session_id_rejected() {
    let tool = SendMessageTool;
    let ctx = make_ctx();
    let input = json!({
        "to": ctx.session_id,
        "message": "hello parent",
        "summary": "greeting parent"
    });

    let result = tool.execute(input, &ctx).await;
    assert!(
        result.is_error,
        "parent session targeting should be rejected"
    );
    assert!(
        result.content.contains("parent") || result.content.contains("main"),
        "error should mention parent/main: {}",
        result.content
    );
}

#[tokio::test]
async fn main_session_keyword_rejected() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "main",
        "message": "hello main",
        "summary": "greeting main"
    });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(result.is_error, "'main' targeting should be rejected");
    assert!(
        result.content.contains("parent") || result.content.contains("main"),
        "error should mention parent/main: {}",
        result.content
    );
}

// --- Agent ID validation ---

#[test]
fn is_valid_agent_id_accepts_uuid() {
    assert!(is_valid_agent_id("550e8400-e29b-41d4-a716-446655440000"));
}

#[test]
fn is_valid_agent_id_accepts_structured_id() {
    assert!(is_valid_agent_id(
        "agent-550e8400-e29b-41d4-a716-446655440000"
    ));
}

#[test]
fn is_valid_agent_id_accepts_simple_name() {
    // Names like "explore" or "code-reviewer" pass format check
    // (they're resolved via name registry first in the agent loop)
    assert!(is_valid_agent_id("explore"));
    assert!(is_valid_agent_id("code-reviewer"));
}

#[test]
fn is_valid_agent_id_rejects_empty() {
    assert!(!is_valid_agent_id(""));
}

#[test]
fn is_valid_agent_id_rejects_spaces() {
    assert!(!is_valid_agent_id("agent with spaces"));
}

#[test]
fn is_valid_agent_id_rejects_too_long() {
    let long = "a".repeat(129);
    assert!(!is_valid_agent_id(&long));
}

#[test]
fn is_valid_agent_id_accepts_max_length() {
    let max = "a".repeat(128);
    assert!(is_valid_agent_id(&max));
}

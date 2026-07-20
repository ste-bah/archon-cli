use super::*;

// -----------------------------------------------------------------------
// TASK-T2 (G2): Structured message types
// -----------------------------------------------------------------------

#[tokio::test]
async fn shutdown_request_without_summary_accepted() {
    // shutdown_request is a structured type — summary should not be required
    let tool = SendMessageTool;
    let input = json!({
        "to": "agent-1",
        "message": "please stop",
        "message_type": "shutdown_request"
    });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(
        !result.is_error,
        "shutdown_request without summary should be accepted: {}",
        result.content
    );
}

#[tokio::test]
async fn shutdown_response_without_message_accepted() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "agent-1",
        "message_type": "shutdown_response",
        "request_id": "req-abc",
        "approve": true
    });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(
        !result.is_error,
        "shutdown_response without message should be accepted: {}",
        result.content
    );

    let request: SendMessageRequest =
        serde_json::from_str(&result.content).expect("should deserialize");
    assert_eq!(request.message_type, "shutdown_response");
    assert_eq!(request.request_id.as_deref(), Some("req-abc"));
    assert_eq!(request.approve, Some(true));
}

#[tokio::test]
async fn shutdown_response_without_request_id_rejected() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "agent-1",
        "message_type": "shutdown_response",
        "approve": true
    });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(result.is_error);
    assert!(
        result.content.contains("request_id"),
        "error should mention request_id: {}",
        result.content
    );
}

#[tokio::test]
async fn shutdown_response_without_approve_rejected() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "agent-1",
        "message_type": "shutdown_response",
        "request_id": "req-abc"
    });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(result.is_error);
    assert!(
        result.content.contains("approve"),
        "error should mention approve: {}",
        result.content
    );
}

#[tokio::test]
async fn plan_approval_response_without_approve_rejected() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "agent-1",
        "message_type": "plan_approval_response",
        "request_id": "req-abc"
    });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(result.is_error);
    assert!(
        result.content.contains("approve"),
        "error should mention approve: {}",
        result.content
    );
}

#[tokio::test]
async fn plan_approval_response_with_all_required_fields_accepted() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "agent-1",
        "message_type": "plan_approval_response",
        "request_id": "req-abc",
        "approve": false,
        "feedback": "please split step 2 into two steps"
    });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(
        !result.is_error,
        "plan_approval_response with all required fields should be accepted: {}",
        result.content
    );

    let request: SendMessageRequest =
        serde_json::from_str(&result.content).expect("should deserialize");
    assert_eq!(request.message_type, "plan_approval_response");
    assert_eq!(request.request_id.as_deref(), Some("req-abc"));
    assert_eq!(request.approve, Some(false));
    assert_eq!(
        request.feedback.as_deref(),
        Some("please split step 2 into two steps")
    );
}

#[tokio::test]
async fn unknown_message_type_rejected() {
    let tool = SendMessageTool;
    let input = json!({
        "to": "agent-1",
        "message": "hello",
        "summary": "greeting",
        "message_type": "bogus"
    });

    let result = tool.execute(input, &make_ctx()).await;
    assert!(result.is_error);
    assert!(
        result.content.contains("message_type") || result.content.contains("bogus"),
        "error should mention unknown message_type: {}",
        result.content
    );
}

// --- build_structured_envelope tests ---

#[test]
fn build_structured_envelope_shutdown_response_with_reason() {
    let req = SendMessageRequest {
        to: "agent-1".into(),
        message: String::new(),
        summary: None,
        message_type: "shutdown_response".into(),
        request_id: Some("req-1".into()),
        approve: Some(true),
        reason: Some("timeout".into()),
        feedback: None,
    };

    let envelope = build_structured_envelope(&req);
    let expected = "<archon_structured_message type=\"shutdown_response\" request_id=\"req-1\" approve=\"true\">\n<reason>timeout</reason>\n</archon_structured_message>";
    assert_eq!(envelope, expected);
}

#[test]
fn build_structured_envelope_plan_approval_with_feedback() {
    let req = SendMessageRequest {
        to: "agent-1".into(),
        message: String::new(),
        summary: None,
        message_type: "plan_approval_response".into(),
        request_id: Some("req-2".into()),
        approve: Some(false),
        reason: None,
        feedback: Some("needs rework".into()),
    };

    let envelope = build_structured_envelope(&req);
    let expected = "<archon_structured_message type=\"plan_approval_response\" request_id=\"req-2\" approve=\"false\">\n<feedback>needs rework</feedback>\n</archon_structured_message>";
    assert_eq!(envelope, expected);
}

#[test]
fn build_structured_envelope_without_optional_inner() {
    let req = SendMessageRequest {
        to: "agent-1".into(),
        message: String::new(),
        summary: None,
        message_type: "shutdown_response".into(),
        request_id: Some("req-3".into()),
        approve: Some(true),
        reason: None,
        feedback: None,
    };

    let envelope = build_structured_envelope(&req);
    let expected = "<archon_structured_message type=\"shutdown_response\" request_id=\"req-3\" approve=\"true\">\n</archon_structured_message>";
    assert_eq!(envelope, expected);
}

#[test]
fn build_structured_envelope_escapes_special_chars() {
    let req = SendMessageRequest {
        to: "agent-1".into(),
        message: String::new(),
        summary: None,
        message_type: "shutdown_response".into(),
        request_id: Some("req-4".into()),
        approve: Some(true),
        reason: Some("<bad> & \"quote\"".into()),
        feedback: None,
    };

    let envelope = build_structured_envelope(&req);
    assert!(
        envelope.contains("&lt;bad&gt;"),
        "< and > should be escaped: {}",
        envelope
    );
    assert!(
        envelope.contains("&amp;"),
        "& should be escaped: {}",
        envelope
    );
    assert!(
        envelope.contains("&quot;quote&quot;"),
        "\" should be escaped: {}",
        envelope
    );
    // Ensure none of the raw special chars leak inside the <reason> body
    assert!(!envelope.contains("<bad>"));
    assert!(!envelope.contains("\"quote\""));
}

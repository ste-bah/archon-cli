use super::*;

#[tokio::test]
async fn gemini_rejects_tools_before_request_dispatch() {
    let provider = VertexProvider::new(
        "project".into(),
        "region".into(),
        "gemini-2.5-pro".into(),
        "google".into(),
        Some("missing-credentials.json".into()),
    );
    let request = LlmRequest {
        tools: std::sync::Arc::new(vec![serde_json::json!({"name": "lookup"})]),
        ..LlmRequest::default()
    };

    let error = provider.stream(request).await.expect_err("tools rejected");
    assert!(matches!(error, LlmError::Unsupported(message) if message.contains("Gemini")));
}

#[test]
fn vertex_token_overflow_maps_to_context_window() {
    let body =
        r#"{"error":{"status":"INVALID_ARGUMENT","message":"maximum context tokens exceeded"}}"#;
    assert!(matches!(
        map_http_error(400, body.to_string()),
        LlmError::ContextWindowExceeded { .. }
    ));
}

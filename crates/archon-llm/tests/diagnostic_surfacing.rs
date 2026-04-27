//! Diagnostic verification tests for v0.1.15 anthropic error surfacing.
//!
//! These tests assert that:
//! - `LlmError::RateLimited` Display includes the API error body preview
//! - `LlmRequest` carries `request_origin` for log correlation
//! - Subagent requests are tagged with `request_origin: "subagent"`

use archon_llm::provider::{LlmError, LlmRequest};

#[test]
fn rate_limited_display_includes_body_preview() {
    let err = LlmError::RateLimited {
        retry_after_secs: 30,
        body_preview: r#"{"type":"error","error":{"type":"rate_limit_error","message":"Usage limit exceeded"}}"#.into(),
    };
    let display = err.to_string();
    // Body must be visible in the Display output — this is what the TUI
    // failure line will show Steven.
    assert!(
        display.contains("rate_limit_error"),
        "RateLimited Display must include the API error body. Got: {display}"
    );
    assert!(
        display.contains("Usage limit exceeded"),
        "RateLimited Display must include the full error message. Got: {display}"
    );
    assert!(
        display.contains("retry after 30s"),
        "RateLimited Display must include retry_after_secs. Got: {display}"
    );
}

#[test]
fn rate_limited_display_default_body_for_unknown() {
    let err = LlmError::RateLimited {
        retry_after_secs: 5,
        body_preview: String::new(),
    };
    let display = err.to_string();
    assert!(
        display.contains("retry after 5s"),
        "RateLimited Display must include retry_after_secs even with empty body. Got: {display}"
    );
}

#[test]
fn llm_request_default_has_no_origin() {
    let req = LlmRequest::default();
    assert_eq!(req.request_origin, None);
}

#[test]
fn llm_request_can_tag_main_session() {
    let req = LlmRequest {
        request_origin: Some("main_session".into()),
        ..LlmRequest::default()
    };
    assert_eq!(req.request_origin.as_deref(), Some("main_session"));
}

#[test]
fn llm_request_can_tag_subagent() {
    let req = LlmRequest {
        request_origin: Some("subagent".into()),
        ..LlmRequest::default()
    };
    assert_eq!(req.request_origin.as_deref(), Some("subagent"));
}

#[test]
fn llm_request_can_tag_pipeline() {
    let req = LlmRequest {
        request_origin: Some("pipeline".into()),
        ..LlmRequest::default()
    };
    assert_eq!(req.request_origin.as_deref(), Some("pipeline"));
}

#[test]
fn request_origin_roundtrips_through_message_request() {
    use archon_llm::anthropic::MessageRequest;

    let llm = LlmRequest {
        request_origin: Some("subagent".into()),
        ..LlmRequest::default()
    };
    let msg: MessageRequest = llm.into();
    assert_eq!(msg.request_origin.as_deref(), Some("subagent"));

    let back: LlmRequest = msg.into();
    assert_eq!(back.request_origin.as_deref(), Some("subagent"));
}

#[test]
fn api_error_rate_limited_includes_body_preview() {
    use archon_llm::anthropic::ApiError;

    let err = ApiError::RateLimited {
        retry_after_secs: 10,
        body_preview: r#"{"type":"error"}"#.into(),
    };
    let display = err.to_string();
    assert!(
        display.contains(r#"{"type":"error"}"#),
        "ApiError::RateLimited Display must include body_preview. Got: {display}"
    );
}

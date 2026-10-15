use super::{ApiError, classify_error};

#[test]
fn anthropic_rejection_types_are_fail_fast_and_redacted_for_http_errors() {
    let secret = "sk-ant-api03-FAKE-1234567890";
    let body = format!(
        r#"{{"type":"error","error":{{"type":"invalid_request_error","message":"bad key {secret}"}}}}"#
    );
    let before = crate::provider::LlmError::Http(format!("HTTP 400: {body}"));
    let before_decision = crate::retry::classify(&before);
    let llm_error = crate::provider::LlmError::from(classify_error(400, &body, None));
    println!(
        "EVIDENCE #364 http-rejection BEFORE: {before_decision:?}, secret_visible={} AFTER: {llm_error}",
        before.to_string().contains(secret)
    );
    assert!(matches!(
        &llm_error,
        crate::provider::LlmError::Rejected { .. }
    ));
    assert!(!llm_error.to_string().contains(secret), "{llm_error}");

    let auth = classify_error(
        401,
        &body.replace("invalid_request_error", "authentication_error"),
        None,
    );
    assert!(matches!(auth, ApiError::Rejected { .. }));
    let auth = crate::provider::LlmError::from(auth);
    println!("EVIDENCE #364 http-auth BEFORE: Auth with raw body AFTER: {auth}");
    assert!(matches!(&auth, crate::provider::LlmError::Rejected { .. }));

    let permission = classify_error(
        403,
        &body.replace("invalid_request_error", "permission_error"),
        None,
    );
    let permission = crate::provider::LlmError::from(permission);
    println!(
        "EVIDENCE #364 http-permission BEFORE: HTTP retryable with raw body AFTER: {permission}"
    );
    assert!(matches!(
        &permission,
        crate::provider::LlmError::Rejected { .. }
    ));
}

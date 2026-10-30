const BODY: &str = "set request.value=AC-X-001; sk-ant-abcdefghijklmnopqrstuvwxyz123456; api_key=sk-abcdefghijklmnopqrstuvwxyz123456; Authorization: Bearer AC-X-002-secret";

fn check(status: u16) {
    let error = convert(status, BODY);
    let message = match &error {
        crate::provider::LlmError::RateLimited { message, .. }
        | crate::provider::LlmError::Overloaded { message, .. } => message.as_str(),
        crate::provider::LlmError::Server { message, .. } => message.as_str(),
        crate::provider::LlmError::Auth(message)
        | crate::provider::LlmError::Rejected { message, .. } => message.as_str(),
        other => panic!("unexpected classification: {other:?}"),
    };
    for text in [
        message,
        &error.to_string(),
        &format!("{error:?}"),
        &serde_json::to_string(&error).unwrap(),
    ] {
        assert!(
            !text.contains("sk-ant-abcdefghijklmnopqrstuvwxyz123456"),
            "Anthropic key leaked at status {status}"
        );
        assert!(
            !text.contains("sk-abcdefghijklmnopqrstuvwxyz123456"),
            "API key leaked at status {status}"
        );
        assert!(
            !text.contains("AC-X-002-secret"),
            "Bearer credential leaked at status {status}"
        );
        assert!(
            text.contains("set request.value=AC-X-001"),
            "diagnostic lost at status {status}"
        );
    }
    assert_eq!(error.is_operational(), matches!(status, 429 | 500..=599));
}

#[test]
fn rate_limit_diagnostic_redacts_secrets() {
    check(429);
}

#[test]
fn overload_diagnostic_redacts_secrets() {
    check(503);
}

#[test]
fn provider_fault_auth_diagnostic_storage_redacts_secrets() {
    for status in [400, 401, 500] {
        check(status);
    }
    let forwarded =
        crate::provider::LlmError::from(crate::anthropic::ApiError::AuthError(BODY.into()));
    assert!(!format!("{forwarded:?}").contains("sk-ant-abcdefghijklmnopqrstuvwxyz123456"));
}

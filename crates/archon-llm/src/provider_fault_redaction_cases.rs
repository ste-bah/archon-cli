const BODY: &str = "set request.value=AC-X-001; api_key=sk-abcdefghijklmnopqrstuvwxyz123456; Authorization: Bearer AC-X-002-secret";

fn check(status: u16) {
    let error = convert(status, BODY);
    let message = match &error {
        crate::provider::LlmError::RateLimited { message, .. }
        | crate::provider::LlmError::Overloaded { message, .. } => message.as_str(),
        crate::provider::LlmError::Server { message, .. } => message.as_str(),
        other => panic!("unexpected classification: {other:?}"),
    };
    for text in [message, &error.to_string()] {
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
    assert!(error.is_operational());
}

#[test]
fn rate_limit_diagnostic_redacts_secrets() {
    check(429);
}

#[test]
fn overload_diagnostic_redacts_secrets() {
    check(503);
}

fn convert(status: u16, body: &str) -> crate::provider::LlmError {
    let status = if status == 503 { 529 } else { status };
    let error = super::classify_error(status, body, Some("2"));
    let message = match &error {
        super::ApiError::RateLimited { message, .. }
        | super::ApiError::Overloaded { message, .. } => message,
        other => panic!("unexpected classification: {other:?}"),
    };
    assert!(
        !message.contains("sk-abcdefghijklmnopqrstuvwxyz123456"),
        "API key leaked in native field"
    );
    assert!(
        !error.to_string().contains("AC-X-002-secret"),
        "Bearer credential leaked in native display"
    );
    error.into()
}
include!("provider_fault_redaction_cases.rs");

fn convert(status: u16, body: &str) -> crate::provider::LlmError {
    let status = if status == 503 { 529 } else { status };
    let error = super::classify_error(status, body, Some("2"));
    match status {
        429 => assert!(matches!(&error, super::ApiError::RateLimited { .. })),
        529 => assert!(matches!(&error, super::ApiError::Overloaded { .. })),
        400 | 500 => assert!(
            matches!(&error, super::ApiError::ServerError { status: actual, .. } if *actual == status)
        ),
        401 => assert!(matches!(&error, super::ApiError::Rejected { .. })),
        other => panic!("unexpected test status: {other}"),
    }
    let message = match &error {
        super::ApiError::RateLimited { message, .. }
        | super::ApiError::Overloaded { message, .. }
        | super::ApiError::Rejected { message, .. }
        | super::ApiError::AuthError(message) => message.as_str(),
        super::ApiError::ServerError { message, .. } => message.as_str(),
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

use crate::{anthropic_support::classify_error, provider::LlmError, provider_fault};

#[test]
fn stream_faults_retain_original_message_and_class() {
    for (kind, operational) in [
        ("rate_limit_exceeded", true),
        ("server_error", true),
        ("overloaded_error", true),
        ("network", true),
        ("authentication_error", false),
        ("invalid_request_error", false),
        ("insufficient_quota", false),
        ("request_too_large", false),
    ] {
        let error = provider_fault::from_stream_error(kind, "AC-X-001");
        assert_eq!(error.is_operational(), operational, "{error:?}");
        assert!(error.to_string().contains("AC-X-001"), "{kind}: {error}");
        let crate::streaming::StreamEvent::Error {
            error_type,
            message,
        } = provider_fault::error_event(error)
        else {
            panic!("missing fault")
        };
        let error = provider_fault::from_stream_error(&error_type, &message);
        assert_eq!(error.is_operational(), operational, "{error:?}");
        assert!(error.to_string().contains("AC-X-001"), "{error}");
    }
}

#[test]
fn status_faults_retain_original_body_and_class() {
    for status in [400, 401, 403, 404, 408, 429, 500, 503, 529] {
        let error = provider_fault::from_status(status, &Default::default(), "AC-X-001".into());
        assert!(error.to_string().contains("AC-X-001"), "{status}: {error}");
    }
}

#[test]
fn native_status_faults_retain_original_body_and_class() {
    for status in [429, 529] {
        let api_error = classify_error(status, "AC-X-001", Some("2"));
        assert!(
            api_error.to_string().contains("AC-X-001"),
            "{status}: {api_error}"
        );
        let error = LlmError::from(api_error);
        assert!(error.is_operational(), "{error:?}");
        assert_eq!(error.retry_after_hint(), Some(2));
        assert!(error.to_string().contains("AC-X-001"), "{status}: {error}");
    }
}

async fn completion_fault(reply: &'static [u8]) -> LlmError {
    use crate::provider::{LlmProvider, LlmRequest};
    let (url, peer) = crate::provider_fault_http_tests::endpoint(reply).await;
    let provider =
        crate::providers::OpenAiProvider::new("AC-X-001".into(), Some(url), "AC-X-002".into());
    let error = provider_fault::defer_retry(provider.complete(LlmRequest {
        model: "AC-X-003".into(),
        ..Default::default()
    }))
    .await
    .unwrap_err();
    peer.await.unwrap();
    error
}

#[tokio::test]
async fn rate_limit_body_survives_http_completion() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let error = completion_fault(b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 8\r\nRetry-After: 2\r\nConnection: close\r\n\r\nAC-X-001").await;
        assert!(matches!(error, LlmError::RateLimited { .. }), "{error:?}");
        assert_eq!(error.retry_after_hint(), Some(2));
        assert!(error.to_string().contains("AC-X-001"), "{error}");
    }).await.unwrap();
}

#[tokio::test]
async fn overload_body_survives_http_completion() {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let error = completion_fault(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 8\r\nRetry-After: 2\r\nConnection: close\r\n\r\nAC-X-001").await;
        assert!(matches!(error, LlmError::Overloaded { .. }), "{error:?}");
        assert_eq!(error.retry_after_hint(), Some(2));
        assert!(error.to_string().contains("AC-X-001"), "{error}");
    }).await.unwrap();
}

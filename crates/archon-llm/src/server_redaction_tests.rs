use crate::{anthropic::ApiError, auth::AuthError, provider::LlmError};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

pub(crate) const BODY: &str = "request field needs correction; sk-ant-abcdefghijklmnopqrstuvwxyz123456; sk-abcdefghijklmnopqrstuvwxyz123456; Authorization: Bearer private-provider-credential";

pub(crate) fn check(error: impl std::fmt::Display + std::fmt::Debug + serde::Serialize) {
    check_text(&serde_json::to_string(&error).unwrap());
    check_text(&format!("{error:?}"));
    check_text(&error.to_string());
}

fn check_auth(error: AuthError) {
    check_text(&error.to_string());
    check_text(&format!("{error:?}"));
    check(LlmError::from(error));
}

fn check_text(text: &str) {
    for secret in [
        "sk-ant-abcdefghijklmnopqrstuvwxyz123456",
        "sk-abcdefghijklmnopqrstuvwxyz123456",
        "private-provider-credential",
    ] {
        assert!(!text.contains(secret), "credential leaked: {text}");
    }
    assert!(
        text.contains("request field needs correction"),
        "diagnostic lost: {text}"
    );
}

fn server() -> LlmError {
    LlmError::Server {
        status: 500,
        message: BODY.into(),
        pipeline_retry_after_secs: Some(7),
    }
}

#[test]
fn server_redaction_direct_construction() {
    let error = server();
    check(&error);
    if let LlmError::Server { message, .. } = error {
        check(message);
    }
}

#[test]
fn server_redaction_anthropic_404_fallback() {
    let error = crate::anthropic_support::classify_error(404, BODY, None);
    check(&error);
    check(LlmError::from(error));
}

#[test]
fn server_redaction_anthropic_5xx_forwarder() {
    check(crate::anthropic_support::classify_error(
        500,
        BODY,
        Some("7"),
    ));
    let error = ApiError::ServerError {
        status: 500,
        message: BODY.into(),
        retry_after_secs: Some(7),
    };
    check(&error);
    let forwarded = LlmError::from(error);
    assert_eq!(forwarded.retry_after_hint(), Some(7));
    check(forwarded);
}

#[test]
fn server_redaction_auth_forwarder() {
    check(crate::auth_fault::auth_error_to_api(AuthError::Provider(
        server(),
    )));
}

#[tokio::test]
async fn server_redaction_oauth_refresh_500() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string(BODY))
        .mount(&server)
        .await;
    let client = crate::oauth_codex::CodexOAuthClient::new_with_urls(
        reqwest::Client::new(),
        server.uri(),
        server.uri(),
    );
    check_auth(client.refresh("refresh-credential").await.unwrap_err());
}

#[tokio::test]
#[serial_test::serial]
async fn server_redaction_token_refresh_500() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500).set_body_string(BODY))
        .mount(&server)
        .await;
    let _guard = crate::tokens::set_token_endpoint_for_tests(server.uri());
    check_auth(
        crate::tokens::refresh_token(
            &crate::types::Secret::new("refresh-credential".into()),
            &reqwest::Client::new(),
        )
        .await
        .unwrap_err(),
    );
}

#[tokio::test]
async fn server_redaction_tls_forwarder() {
    // HEAD responses cannot carry a body, so exercise the forwarder's shared
    // response conversion with a GET response containing a provider diagnostic.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500).set_body_string(BODY))
        .mount(&server)
        .await;
    let response = reqwest::Client::new()
        .get(server.uri())
        .send()
        .await
        .unwrap();
    check(
        crate::transport_http::response_error(response, |status, message| LlmError::Server {
            status,
            message: message.into(),
            pipeline_retry_after_secs: None,
        })
        .await,
    );
}

#[test]
fn server_redaction_other_provider_text_variants() {
    for error in [
        LlmError::Unsupported(BODY.into()),
        LlmError::ProviderNotFound {
            name: BODY.into(),
            available: BODY.into(),
        },
        LlmError::Auth(BODY.into()),
        LlmError::Http(BODY.into()),
        LlmError::Transport(BODY.into()),
        LlmError::Serialize(BODY.into()),
        LlmError::QuotaExceeded(BODY.into()),
        LlmError::RateLimited {
            message: BODY.into(),
            retry_after_secs: 7,
            from_provider: true,
            pipeline_retry_after_secs: Some(7),
        },
        LlmError::Overloaded {
            message: BODY.into(),
            status: Some(503),
            retry_after_secs: Some(7),
        },
        LlmError::Rejected {
            error_type: BODY.into(),
            message: BODY.into(),
        },
        LlmError::ContextWindowExceeded {
            provider_message: BODY.into(),
            provider: Some(BODY.into()),
            model: Some(BODY.into()),
        },
    ] {
        check(error);
    }
    for error in [
        ApiError::AuthError(BODY.into()),
        ApiError::HttpError(BODY.into()),
        ApiError::Transport(BODY.into()),
        ApiError::SerializeError(BODY.into()),
        ApiError::RateLimited {
            message: BODY.into(),
            retry_after_secs: 7,
            from_provider: true,
        },
        ApiError::Overloaded {
            message: BODY.into(),
            retry_after_secs: Some(7),
        },
        ApiError::Rejected {
            error_type: BODY.into(),
            message: BODY.into(),
        },
    ] {
        check(error);
    }
}

#[test]
fn server_redaction_stream_event_storage_and_serde() {
    let event = crate::streaming::StreamEvent::Error {
        error_type: BODY.into(),
        message: BODY.into(),
    };
    check_text(&format!("{event:?}"));
    check_text(&serde_json::to_string(&event).unwrap());
    let decoded: crate::streaming::StreamEvent = serde_json::from_value(serde_json::json!({
        "Error": {"error_type": BODY, "message": BODY}
    }))
    .unwrap();
    check_text(&format!("{decoded:?}"));
}

#[test]
fn server_redaction_preserves_raw_text_classification() {
    use crate::context_window::RequestPressureKind;
    let error = LlmError::Server {
        status: 400,
        message: "Authorization: Bearer example-token; prompt is too long".into(),
        pipeline_retry_after_secs: None,
    };
    // 86fbab995 stored a server message redacted: the authorization line,
    // "prompt is too long" included, was gone before these classifiers ran.
    assert_eq!(error.request_pressure_kind(), None::<RequestPressureKind>);
    assert!(!error.is_context_window_exceeded());
    // An HTTP diagnostic was stored as given, so its raw text classifies.
    let error = LlmError::Http("Authorization: Bearer example-token; connection reset".into());
    assert!(error.is_operational());
    assert!(!format!("{error:?}").contains("example-token"));
}

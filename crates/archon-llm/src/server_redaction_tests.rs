use crate::{anthropic::ApiError, auth::AuthError, provider::LlmError};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

pub(crate) const BODY: &str = "request field needs correction; sk-ant-abcdefghijklmnopqrstuvwxyz123456; sk-abcdefghijklmnopqrstuvwxyz123456; Authorization: Bearer private-provider-credential";

pub(crate) fn check(error: impl std::fmt::Display) {
    let text = error.to_string();
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
    check(client.refresh("refresh-credential").await.unwrap_err());
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
    check(
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
            provider: None,
            model: None,
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

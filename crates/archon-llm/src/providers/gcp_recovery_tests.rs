use super::*;
use crate::provider_fault_http_tests::{STATUSES, assert_transient, endpoint};

async fn check(reply: &'static [u8], hint: Option<u64>) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let (url, peer) = endpoint(reply).await;
        let key = ServiceAccountKey {
            client_email: "AC-X-001@example.invalid".into(),
            private_key: include_str!("../../tests/fixtures/AC-X-001.pem").into(),
            token_uri: url,
        };
        let error = crate::provider_fault::defer_retry(get_access_token(
            &crate::transport_http::client_builder().build().unwrap(),
            &key,
        ))
        .await
        .unwrap_err();
        assert_transient(error, hint);
        peer.await.unwrap();
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn token_drop_is_typed() {
    check(b"", None).await;
}
#[tokio::test]
async fn token_status_is_typed() {
    for &(reply, hint) in STATUSES {
        check(reply, hint).await;
    }
}

#[tokio::test]
async fn token_body_drop_is_typed() {
    check(
        b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n",
        None,
    )
    .await;
}

#[tokio::test]
async fn provider_fault_gcp_auth_redaction() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    for status in [400, 401, 500] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(status).set_body_string(crate::server_redaction_tests::BODY),
            )
            .mount(&server)
            .await;
        let key = ServiceAccountKey {
            client_email: "test@example.invalid".into(),
            private_key: include_str!("../../tests/fixtures/AC-X-001.pem").into(),
            token_uri: server.uri(),
        };
        let error = get_access_token(&reqwest::Client::new(), &key)
            .await
            .unwrap_err();
        crate::server_redaction_tests::check(&error);
        if status < 500 {
            let LlmError::Auth(message) = error else {
                panic!("expected authentication error")
            };
            crate::server_redaction_tests::check(serde_json::to_string(&message).unwrap());
        } else {
            assert!(matches!(error, LlmError::Server { status: 500, .. }));
        }
    }
}

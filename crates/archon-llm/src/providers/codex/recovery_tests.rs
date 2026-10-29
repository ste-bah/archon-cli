use super::*;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn credentials(root: &std::path::Path) -> PathBuf {
    let path = root.join("AC-X-001.json");
    write_codex_credentials_atomic(
        &path,
        &CodexCredentials {
            access_token: crate::types::Secret::new("AC-X-001".into()),
            refresh_token: crate::types::Secret::new("AC-X-002".into()),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            account_id: "AC-X-001".into(),
        },
    )
    .unwrap();
    path
}
fn request() -> LlmRequest {
    LlmRequest {
        model: "AC-X-001".into(),
        messages: vec![serde_json::json!({"role":"user","content":"AC-X-001"})],
        ..Default::default()
    }
}

#[tokio::test]
async fn transient_preflight_is_probed_again_by_production_stream() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let server = MockServer::start().await;
        let root = tempfile::tempdir().unwrap();
        let provider = CodexProvider::new_with_base_url(
            credentials(root.path()),
            SpoofConfig::default(),
            crate::transport_http::client_builder().build().unwrap(),
            server.uri(),
        )
        .unwrap();
        Mock::given(method("HEAD"))
            .respond_with(ResponseTemplate::new(503).insert_header("Retry-After", "2"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        let first = crate::provider_fault::defer_retry(provider.stream(request())).await;
        assert!(first.is_err());
        server.reset().await;
        Mock::given(method("HEAD"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/codex/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("data: {\"type\":\"response.completed\",\"response\":{}}\n\n"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let next = crate::provider_fault::defer_retry(provider.stream(request())).await;
        assert!(next.is_ok(), "transient preflight was cached: {next:?}");
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn refresh_service_failure_stays_operational_at_production_stream_boundary() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let server = MockServer::start().await;
        let root = tempfile::tempdir().unwrap();
        let credential_path = credentials(root.path());
        let (mut creds, _) = read_codex_credentials_locked(&credential_path).unwrap();
        creds.expires_at = chrono::Utc::now() - chrono::Duration::hours(1);
        write_codex_credentials_atomic(&credential_path, &creds).unwrap();
        let mut provider = CodexProvider::new_with_base_url(
            credential_path,
            SpoofConfig::default(),
            crate::transport_http::client_builder().build().unwrap(),
            server.uri(),
        )
        .unwrap();
        provider.oauth_client = Arc::new(CodexOAuthClient::new_with_urls(
            provider.http.clone(),
            server.uri(),
            format!("{}/AC-X-001", server.uri()),
        ));
        Mock::given(method("HEAD"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/AC-X-001"))
            .respond_with(
                ResponseTemplate::new(503)
                    .insert_header("Retry-After", "2")
                    .set_body_string("AC-X-001"),
            )
            .mount(&server)
            .await;
        let error = crate::provider_fault::defer_retry(provider.stream(request()))
            .await
            .unwrap_err();
        assert!(
            error.is_operational(),
            "refresh outage became permanent: {error:?}"
        );
        assert_eq!(error.retry_after_hint(), Some(2));
        server.reset().await;
        Mock::given(method("HEAD"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        use base64::Engine;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"https://api.openai.com/auth":{"chatgpt_account_id":"AC-X-001"}}"#);
        Mock::given(method("POST"))
            .and(path("/AC-X-001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token":format!("AC-X-001.{payload}.AC-X-002"),
                "refresh_token":"AC-X-003", "expires_in":7200})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/codex/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("data: {\"type\":\"response.completed\",\"response\":{}}\n\n"),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert!(
            crate::provider_fault::defer_retry(provider.stream(request()))
                .await
                .is_ok()
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn refresh_body_disconnect_retains_operational_type_and_recovers() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let server = MockServer::start().await;
        let root = tempfile::tempdir().unwrap();
        let credential_path = credentials(root.path());
        let (mut creds, _) = read_codex_credentials_locked(&credential_path).unwrap();
        creds.expires_at = chrono::Utc::now() - chrono::Duration::hours(1);
        write_codex_credentials_atomic(&credential_path, &creds).unwrap();
        let mut provider = CodexProvider::new_with_base_url(
            credential_path,
            SpoofConfig::default(),
            crate::transport_http::client_builder().build().unwrap(),
            server.uri(),
        )
        .unwrap();
        use base64::Engine;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"https://api.openai.com/auth":{"chatgpt_account_id":"AC-X-001"}}"#);
        let body = serde_json::json!({"access_token":format!("AC-X-001.{payload}.AC-X-002"),
            "refresh_token":"AC-X-003", "expires_in":7200})
        .to_string();
        let (endpoint, peer) =
            crate::connection_tests::truncated_response_then_recovery(body).await;
        provider.oauth_client = Arc::new(CodexOAuthClient::new_with_urls(
            provider.http.clone(),
            server.uri(),
            endpoint,
        ));
        Mock::given(method("HEAD"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/codex/responses"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("data: {\"type\":\"response.completed\",\"response\":{}}\n\n"),
            )
            .mount(&server)
            .await;
        let error = crate::provider_fault::defer_retry(provider.stream(request()))
            .await
            .unwrap_err();
        assert!(
            error.is_operational(),
            "refresh body transport class lost: {error:?}"
        );
        assert!(
            crate::provider_fault::defer_retry(provider.stream(request()))
                .await
                .is_ok()
        );
        peer.await.unwrap();
    })
    .await
    .unwrap();
}

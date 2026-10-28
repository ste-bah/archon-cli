use archon_llm::{
    auth::CodexCredentials,
    provider::{LlmError, LlmProvider, LlmRequest},
    providers::{OpenAiProvider, codex::CodexProvider},
    types::Secret,
};
use std::time::Duration;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers};

fn request() -> LlmRequest {
    LlmRequest {
        model: "AC-X-001".into(),
        messages: vec![serde_json::json!({"role":"user","content":"AC-X-001"})],
        ..Default::default()
    }
}

async fn failed_completion(code: &str) -> LlmError {
    let server = MockServer::start().await;
    Mock::given(matchers::method("HEAD"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let event = serde_json::json!({
        "type": "response.failed",
        "response": {
            "id": "AC-X-001", "status": "failed",
            "error": {"code": code, "message": "AC-X-002"}
        }
    });
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/codex/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string(format!("event: response.failed\ndata: {event}\n\n")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("AC-X-001.json");
    archon_llm::tokens_codex::write_codex_credentials_atomic(
        &path,
        &CodexCredentials {
            access_token: Secret::new("AC-X-001".into()),
            refresh_token: Secret::new("AC-X-002".into()),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            account_id: "AC-X-003".into(),
        },
    )
    .unwrap();
    let provider = CodexProvider::new_with_base_url(
        path,
        Default::default(),
        archon_llm::transport_http::client_builder()
            .build()
            .unwrap(),
        server.uri(),
    )
    .unwrap();
    archon_llm::provider_fault::defer_retry(provider.complete(request()))
        .await
        .unwrap_err()
}

#[tokio::test]
async fn response_failure_rate_limit_keeps_its_type_through_completion() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let error = failed_completion("rate_limit_exceeded").await;
        assert!(error.is_operational(), "lost outage class: {error:?}");
        assert!(
            matches!(error, LlmError::RateLimited { .. }),
            "lost rate limit class: {error:?}"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn response_failure_overload_keeps_its_type_through_completion() {
    tokio::time::timeout(Duration::from_secs(10), async {
        for code in [
            "server_overloaded",
            "server_error",
            "overloaded_error",
            "api_error",
        ] {
            let error = failed_completion(code).await;
            assert!(
                error.is_operational(),
                "lost outage class for {code}: {error:?}"
            );
            assert!(
                matches!(error, LlmError::Overloaded { .. }),
                "lost overload class for {code}: {error:?}"
            );
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn redirect_loop_is_permanent_through_provider_request() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let server = MockServer::start().await;
        Mock::given(matchers::any())
            .respond_with(ResponseTemplate::new(307).insert_header("Location", "/AC-X-001"))
            .mount(&server)
            .await;
        let provider =
            OpenAiProvider::new("AC-X-002".into(), Some(server.uri()), "AC-X-001".into());
        let error = archon_llm::provider_fault::defer_retry(provider.complete(request()))
            .await
            .unwrap_err();
        assert!(
            error.is_permanent(),
            "redirect loop became an outage: {error:?}"
        );
        assert!(
            matches!(error, LlmError::Serialize(_)),
            "redirect policy failure lost its request class: {error:?}"
        );
        let diagnostic = error.to_string();
        assert!(diagnostic.contains("redirect"), "lost cause: {diagnostic}");
        assert!(diagnostic.contains("/AC-X-001"), "lost URL: {diagnostic}");
    })
    .await
    .unwrap();
}

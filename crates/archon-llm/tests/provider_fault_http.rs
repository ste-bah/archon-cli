use archon_llm::{
    ApiKey,
    provider::{LlmError, LlmProvider, LlmRequest},
    providers::*,
};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn request() -> LlmRequest {
    LlmRequest {
        model: "AC-X-001".into(),
        messages: vec![serde_json::json!({"role":"user","content":"AC-X-001"})],
        ..Default::default()
    }
}
fn compatible(url: &str) -> OpenAiCompatProvider {
    OpenAiCompatProvider::new(
        Box::leak(Box::new(ProviderDescriptor {
            id: "AC-X-001".into(),
            display_name: "AC-X-001".into(),
            base_url: url.parse().unwrap(),
            auth_flavor: AuthFlavor::None,
            env_key_var: String::new(),
            compat_kind: CompatKind::OpenAiCompat,
            default_model: "AC-X-001".into(),
            supports: ProviderFeatures {
                streaming: true,
                ..ProviderFeatures::chat_only()
            },
            headers: Default::default(),
            quirks: ProviderQuirks::default(),
            is_gap: false,
        })),
        Arc::new(
            archon_llm::transport_http::client_builder()
                .build()
                .unwrap(),
        ),
        ApiKey::new("AC-X-002".into()),
    )
}
async fn endpoint(reply: &'static [u8]) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let peer = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 8192];
            let n = socket.read(&mut buf).await.unwrap();
            if buf[..n].starts_with(b"HEAD ") || buf[..n].starts_with(b"GET ") {
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
                continue;
            }
            if !reply.is_empty() {
                socket.write_all(reply).await.unwrap();
            }
            break;
        }
    });
    (url, peer)
}
fn assert_transient(error: LlmError) {
    assert!(
        matches!(
            error,
            LlmError::Transport(_)
                | LlmError::Server {
                    status: 408 | 500..=599,
                    ..
                }
                | LlmError::RateLimited { .. }
                | LlmError::Overloaded { .. }
        ),
        "untyped operational fault: {error:?}"
    );
}
fn provider(family: u8, url: String, root: &std::path::Path) -> Box<dyn LlmProvider> {
    match family {
        0 => Box::new(OpenAiProvider::new(
            "AC-X-002".into(),
            Some(url),
            "AC-X-001".into(),
        )),
        1 => Box::new(compatible(&url)),
        2 => Box::new(LocalProvider::new(url, "AC-X-001".into(), 60, false)),
        3 => Box::new(AnthropicProvider::new(
            archon_llm::anthropic::AnthropicClient::new(
                archon_llm::auth::AuthProvider::ApiKey(archon_llm::types::Secret::new(
                    "AC-X-002".into(),
                )),
                archon_llm::identity::IdentityProvider::new(
                    archon_llm::identity::IdentityMode::Clean,
                    "AC-X-001".into(),
                    "AC-X-001".into(),
                    String::new(),
                ),
                Some(url),
            ),
        )),
        _ => {
            let path = root.join("AC-X-001.json");
            archon_llm::tokens_codex::write_codex_credentials_atomic(
                &path,
                &archon_llm::auth::CodexCredentials {
                    access_token: archon_llm::types::Secret::new("AC-X-002".into()),
                    refresh_token: archon_llm::types::Secret::new("AC-X-003".into()),
                    expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
                    account_id: "AC-X-004".into(),
                },
            )
            .unwrap();
            Box::new(
                codex::CodexProvider::new_with_base_url(
                    path,
                    codex::spoof::SpoofConfig::default(),
                    archon_llm::transport_http::client_builder()
                        .build()
                        .unwrap(),
                    url,
                )
                .unwrap(),
            )
        }
    }
}
async fn check(family: u8, reply: &'static [u8], hint: Option<u64>) {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (url, peer) = endpoint(reply).await;
        let root = tempfile::tempdir().unwrap();
        let provider = provider(family, url, root.path());
        let error = archon_llm::provider_fault::defer_retry(provider.stream(request()))
            .await
            .unwrap_err();
        if let Some(hint) = hint {
            assert_eq!(error.retry_after_hint(), Some(hint), "{error:?}");
        }
        assert_transient(error);
        peer.await.unwrap();
    })
    .await
    .unwrap();
}
macro_rules! family_tests {
    ($drop:ident, $status:ident, $id:expr) => {
        #[tokio::test] async fn $status() {
            check($id, b"HTTP/1.1 408 Request Timeout\r\nContent-Length: 2\r\nRetry-After: 7\r\nConnection: close\r\n\r\n{}", Some(7)).await;
            check($id, b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 2\r\nRetry-After: 7\r\nConnection: close\r\n\r\n{}", Some(7)).await;
            check($id, b"", None).await;
        }
    };
}
family_tests!(native_drop_is_typed, native_status_is_typed, 0);
family_tests!(compatible_drop_is_typed, compatible_status_is_typed, 1);
family_tests!(local_drop_is_typed, local_status_is_typed, 2);
family_tests!(messages_drop_is_typed, messages_status_is_typed, 3);
family_tests!(responses_drop_is_typed, responses_status_is_typed, 4);

#[tokio::test]
async fn local_structured_rejections_keep_the_remedy_once() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let server = wiremock::MockServer::start().await;
        for (status, kind) in [(429, "insufficient_quota"), (500, "model_not_found")] {
            server.reset().await;
            wiremock::Mock::given(wiremock::matchers::method("POST")).respond_with(wiremock::ResponseTemplate::new(status).set_body_json(
                serde_json::json!({"error":{"code":kind,"message":"set request.value=AC-X-001"}}))).expect(1).mount(&server).await;
            let error = archon_llm::provider_fault::defer_retry(LocalProvider::new(server.uri(), "AC-X-001".into(), 60, false).stream(request())).await.unwrap_err();
            assert!(error.is_permanent(), "structured rejection became transient: {error:?}");
            assert!(error.to_string().contains("set request.value=AC-X-001"));
        }
    }).await.unwrap();
}

#[tokio::test]
async fn embedding_drop_is_typed() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (url, peer) = endpoint(b"").await;
        let error = compatible(&url)
            .embed("AC-X-001", vec!["AC-X-001".into()])
            .await
            .unwrap_err();
        assert_transient(error);
        peer.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn completion_keeps_stream_event_classes() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let server = wiremock::MockServer::start().await;
        for kind in ["overloaded_error", "invalid_request_error"] {
            server.reset().await;
            wiremock::Mock::given(wiremock::matchers::method("POST")).respond_with(wiremock::ResponseTemplate::new(200).set_body_string(format!(
                "event: error\ndata: {{\"type\":\"error\",\"error\":{{\"type\":\"{kind}\",\"message\":\"set request.value=AC-X-001\"}}}}\n\n"))).mount(&server).await;
            let root = tempfile::tempdir().unwrap();
            let error = provider(3, server.uri(), root.path()).complete(request()).await.unwrap_err();
            if kind == "overloaded_error" { assert_transient(error); }
            else { assert!(matches!(error, LlmError::Rejected { .. }), "lost class: {error:?}"); }
        }
    }).await.unwrap();
}

#[tokio::test]
async fn agentic_keeps_stream_event_classes() {
    use archon_llm::agentic::*;
    tokio::time::timeout(Duration::from_secs(10), async {
        let server = wiremock::MockServer::start().await;
        for kind in ["overloaded_error", "invalid_request_error"] {
            server.reset().await;
            wiremock::Mock::given(wiremock::matchers::method("POST")).respond_with(wiremock::ResponseTemplate::new(200).set_body_string(format!(
                "event: error\ndata: {{\"type\":\"error\",\"error\":{{\"type\":\"{kind}\",\"message\":\"set request.value=AC-X-001\"}}}}\n\n"))).mount(&server).await;
            let root = tempfile::tempdir().unwrap();
            let adapter = LlmProviderAgenticAdapter::new(Arc::from(provider(3, server.uri(), root.path())), "AC-X-001", "AC-X-001", ProviderCapabilitySet::new([ProviderCapability::Streaming]));
            let (sink, _events) = TurnEventSink::channel(16);
            let error = adapter.stream_turn(request().into(), sink).await.unwrap_err();
            if kind == "overloaded_error" { assert_transient(error); }
            else { assert!(matches!(error, LlmError::Rejected { .. }), "lost class: {error:?}"); }
        }
    }).await.unwrap();
}

async fn body_drop(family: u8) {
    tokio::time::timeout(Duration::from_secs(10), async {
        let (url, peer) =
            endpoint(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n").await;
        let root = tempfile::tempdir().unwrap();
        let error = archon_llm::provider_fault::defer_retry(
            provider(family, url, root.path()).complete(request()),
        )
        .await
        .unwrap_err();
        assert_transient(error);
        peer.await.unwrap();
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn native_body_drop_keeps_class() {
    body_drop(0).await;
}
#[tokio::test]
async fn compatible_body_drop_keeps_class() {
    body_drop(1).await;
}
#[tokio::test]
async fn local_body_drop_keeps_class() {
    body_drop(2).await;
}
#[tokio::test]
async fn messages_body_drop_keeps_class() {
    body_drop(3).await;
}
#[tokio::test]
async fn responses_body_drop_keeps_class() {
    body_drop(4).await;
}

#[tokio::test]
async fn preflight_connection_drop_is_typed() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let peer = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0; 8192];
            let n = socket.read(&mut buf).await.unwrap();
            assert!(
                n > 0,
                "preflight must send request bytes before the peer drops"
            );
        });
        let error = codex::tls_preflight::run_codex_tls_preflight_url(
            &archon_llm::transport_http::client_builder()
                .build()
                .unwrap(),
            &url,
        )
        .await
        .unwrap_err();
        assert_transient(error);
        peer.await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn malformed_request_setup_stays_permanent() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let provider = OpenAiProvider::new(
            "AC-X-002".into(),
            Some("http://[".into()),
            "AC-X-001".into(),
        );
        let error = provider.stream(request()).await.unwrap_err();
        assert!(
            matches!(error, LlmError::Serialize(_)),
            "setup fault became operational: {error:?}"
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn preflight_status_keeps_retry_hint() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let server = wiremock::MockServer::start().await;
        for status in [408, 503] {
            server.reset().await;
            wiremock::Mock::given(wiremock::matchers::method("HEAD"))
                .respond_with(
                    wiremock::ResponseTemplate::new(status).insert_header("Retry-After", "7"),
                )
                .expect(1)
                .mount(&server)
                .await;
            let error = codex::tls_preflight::run_codex_tls_preflight_url(
                &archon_llm::transport_http::client_builder()
                    .build()
                    .unwrap(),
                &server.uri(),
            )
            .await
            .unwrap_err();
            assert_eq!(error.retry_after_hint(), Some(7));
            assert_transient(error);
        }
    })
    .await
    .unwrap();
}

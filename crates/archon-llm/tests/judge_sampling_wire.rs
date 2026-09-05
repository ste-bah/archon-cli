//! Sampling survives generic-to-wire conversion without affecting defaults.
use archon_llm::anthropic::{AnthropicClient, MessageRequest};
use archon_llm::auth::AuthProvider;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::LlmRequest;
use archon_llm::types::Secret;

#[test]
fn explicit_temperature_reaches_messages_wire_and_round_trips() {
    let client = AnthropicClient::new(
        AuthProvider::ApiKey(Secret::new("test-key".into())),
        IdentityProvider::new(IdentityMode::Clean, "s".into(), "d".into(), String::new()),
        Some("http://127.0.0.1:1/v1/messages".into()),
    );
    for temperature in [None, Some(0.0)] {
        let request = LlmRequest {
            extra: temperature
                .map(|v| serde_json::json!({"temperature":v}))
                .unwrap_or_default(),
            ..Default::default()
        };
        let message: MessageRequest = request.into();
        let body: serde_json::Value =
            serde_json::from_str(&client.build_request_body(&message).unwrap()).unwrap();
        assert_eq!(
            body.get("temperature").and_then(|v| v.as_f64()),
            temperature
        );
        let round_trip: LlmRequest = message.into();
        assert_eq!(
            round_trip.extra.get("temperature").and_then(|v| v.as_f64()),
            temperature
        );
    }
}

#[tokio::test]
async fn temperature_reaches_native_and_local_chat_wire() {
    use archon_llm::provider::LlmProvider;
    use archon_llm::providers::{LocalProvider, OpenAiProvider};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    for local in [false, true] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string("data: [DONE]\n\n"),
            )
            .mount(&server)
            .await;
        let provider: Box<dyn LlmProvider> = if local {
            Box::new(LocalProvider::new(
                server.uri(),
                "test-model".into(),
                10,
                false,
            ))
        } else {
            Box::new(OpenAiProvider::new(
                "test-key".into(),
                Some(server.uri()),
                "test-model".into(),
            ))
        };
        for extra in [
            serde_json::Value::Null,
            serde_json::json!({"temperature":0.0}),
        ] {
            let mut stream = provider
                .stream(LlmRequest {
                    model: "test-model".into(),
                    extra,
                    ..Default::default()
                })
                .await
                .unwrap();
            while stream.recv().await.is_some() {}
        }
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2, "local={local}, requests={:?}", requests.iter().map(|r| (&r.method, &r.url)).collect::<Vec<_>>());
        assert!(
            requests[0]
                .body_json::<serde_json::Value>()
                .unwrap()
                .get("temperature")
                .is_none()
        );
        assert_eq!(
            requests[1].body_json::<serde_json::Value>().unwrap()["temperature"],
            0.0
        );
    }
}

//! TASK-AGS-707 Gate 1: integration tests for NDJSON-delimited streaming
//! (Mistral quirk). Asserts that the delimiter dispatch in
//! `OpenAiCompatProvider::stream` correctly routes to `decode_ndjson_line`
//! when `descriptor.quirks.stream_delimiter == StreamDelimiter::MistralNdjson`.

use std::collections::HashMap;
use std::sync::Arc;

use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::{
    AuthFlavor, CompatKind, OpenAiCompatProvider, ProviderDescriptor, ProviderFeatures,
    ProviderQuirks,
};
use archon_llm::secrets::ApiKey;
use archon_llm::streaming::StreamEvent;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn http() -> Arc<reqwest::Client> {
    Arc::new(reqwest::Client::new())
}

fn leak_mistral_descriptor(mock: &MockServer) -> &'static ProviderDescriptor {
    let desc = ProviderDescriptor {
        id: "mistral-test".into(),
        display_name: "MistralTest".into(),
        base_url: Url::parse(&format!("{}/v1", mock.uri())).expect("mock url parses"),
        auth_flavor: AuthFlavor::BearerApiKey,
        env_key_var: "MISTRAL_API_KEY".into(),
        compat_kind: CompatKind::OpenAiCompat,
        default_model: "mistral-large-latest".into(),
        supports: ProviderFeatures::chat_only(),
        headers: HashMap::new(),
        // Mistral quirks set stream_delimiter to MistralNdjson.
        quirks: ProviderQuirks::MISTRAL,
    };
    Box::leak(Box::new(desc))
}

fn sample_request() -> LlmRequest {
    LlmRequest {
        model: "mistral-large-latest".into(),
        max_tokens: 64,
        system: Vec::new(),
        messages: vec![serde_json::json!({"role": "user", "content": "bonjour"})],
        tools: Vec::new(),
        thinking: None,
        speed: None,
        effort: None,
        extra: serde_json::Value::Null,
            request_origin: None,
    }
}

async fn drain_stream(mut rx: tokio::sync::mpsc::Receiver<StreamEvent>) -> Vec<StreamEvent> {
    let mut out = Vec::new();
    for _ in 0..256 {
        match tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv()).await {
            Ok(Some(ev)) => out.push(ev),
            Ok(None) => break,
            Err(_) => panic!("stream recv timed out; events so far: {out:?}"),
        }
    }
    out
}

#[tokio::test]
async fn ndjson_stream_yields_two_text_deltas_and_message_stop() {
    let mock = MockServer::start().await;

    // Mistral-style NDJSON: one JSON object per line, no `data:` prefix,
    // no `[DONE]` sentinel (end-of-stream = server closes the connection).
    let ndjson_body = concat!(
        "{\"choices\":[{\"delta\":{\"content\":\"bon\"}}]}\n",
        "{\"choices\":[{\"delta\":{\"content\":\"jour\"}}]}\n",
    );

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/x-ndjson")
                .set_body_raw(ndjson_body.as_bytes().to_vec(), "application/x-ndjson"),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let descriptor = leak_mistral_descriptor(&mock);
    let provider = OpenAiCompatProvider::new(descriptor, http(), ApiKey::new("test".into()));

    let rx = provider
        .stream(sample_request())
        .await
        .expect("mistral stream() must succeed on healthy NDJSON endpoint");

    let events = drain_stream(rx).await;

    let deltas: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::TextDelta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        deltas,
        vec!["bon", "jour"],
        "expected two NDJSON text deltas in order, got {events:?}"
    );

    let has_stop = events.iter().any(|e| matches!(e, StreamEvent::MessageStop));
    assert!(
        has_stop,
        "NDJSON stream must emit MessageStop on EOF (no [DONE] sentinel in NDJSON); got {events:?}"
    );
}

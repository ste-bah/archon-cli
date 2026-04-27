//! TASK-AGS-707 Gate 1: integration tests for `OpenAiCompatProvider::stream`
//! on SSE-delimited providers (default for every OpenAI-compat except Mistral).
//!
//! Written BEFORE the `stream()` body is rewritten so the impl has a
//! compile-and-pass target. Uses `wiremock` to stand in for a live
//! provider endpoint and asserts the shape of the emitted `StreamEvent`
//! stream, not just that the call succeeds.

use std::collections::HashMap;
use std::sync::Arc;

use archon_llm::provider::{LlmError, LlmProvider, LlmRequest};
use archon_llm::providers::{
    AuthFlavor, CompatKind, OpenAiCompatProvider, ProviderDescriptor, ProviderFeatures,
    ProviderQuirks,
};
use archon_llm::secrets::ApiKey;
use archon_llm::streaming::StreamEvent;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn http() -> Arc<reqwest::Client> {
    Arc::new(reqwest::Client::new())
}

/// Build a `&'static ProviderDescriptor` pointed at a wiremock `MockServer`.
///
/// `supports_streaming` toggles `ProviderFeatures::streaming` so the
/// "streaming-unsupported returns Err without network" criterion (#6) can
/// reuse this helper.
fn leak_descriptor(
    mock: &MockServer,
    supports_streaming: bool,
    quirks: ProviderQuirks,
) -> &'static ProviderDescriptor {
    let features = ProviderFeatures {
        streaming: supports_streaming,
        tool_calling: false,
        vision: false,
        embeddings: false,
        json_mode: false,
    };
    let desc = ProviderDescriptor {
        id: "test-compat".into(),
        display_name: "TestCompat".into(),
        base_url: Url::parse(&format!("{}/v1", mock.uri())).expect("mock url parses"),
        auth_flavor: AuthFlavor::BearerApiKey,
        env_key_var: "TEST_COMPAT_API_KEY".into(),
        compat_kind: CompatKind::OpenAiCompat,
        default_model: "test-model".into(),
        supports: features,
        headers: HashMap::new(),
        quirks,
    };
    Box::leak(Box::new(desc))
}

fn sample_request() -> LlmRequest {
    LlmRequest {
        model: "test-model".into(),
        max_tokens: 64,
        system: Vec::new(),
        messages: vec![serde_json::json!({"role": "user", "content": "ping"})],
        tools: Vec::new(),
        thinking: None,
        speed: None,
        effort: None,
        request_origin: None,
        extra: serde_json::Value::Null,
    }
}

/// Collect a stream until it closes or the helper's safety cap fires.
async fn drain_stream(mut rx: tokio::sync::mpsc::Receiver<StreamEvent>) -> Vec<StreamEvent> {
    let mut out = Vec::new();
    // Safety cap — 256 events is far beyond what the tests produce.
    for _ in 0..256 {
        match tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv()).await {
            Ok(Some(ev)) => out.push(ev),
            Ok(None) => break,
            Err(_) => panic!(
                "stream recv timed out after 3s — channel was not closed cleanly; events so far: {out:?}"
            ),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Validation Criterion 2: SSE happy path — 3 data frames + [DONE]
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_stream_yields_three_text_deltas_and_message_stop() {
    let mock = MockServer::start().await;

    let sse_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"hel\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"lo \"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"world\"}}]}\n\n",
        "data: [DONE]\n\n",
    );

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(sse_body.as_bytes().to_vec(), "text/event-stream"),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let descriptor = leak_descriptor(&mock, true, ProviderQuirks::DEFAULT);
    let provider = OpenAiCompatProvider::new(descriptor, http(), ApiKey::new("test".into()));

    let rx = provider
        .stream(sample_request())
        .await
        .expect("stream() must succeed on healthy SSE endpoint");

    let events = drain_stream(rx).await;

    // Expect (at least) 3 TextDelta events in the order "hel", "lo ", "world".
    let deltas: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::TextDelta { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        deltas,
        vec!["hel", "lo ", "world"],
        "expected three text deltas in order, got {events:?}"
    );

    // MessageStop must be emitted once — either triggered by [DONE] or by
    // our EOF fallback. Exactly one is sufficient.
    let stops = events
        .iter()
        .filter(|e| matches!(e, StreamEvent::MessageStop))
        .count();
    assert_eq!(
        stops, 1,
        "expected exactly one MessageStop, got {stops} in {events:?}"
    );
}

// ---------------------------------------------------------------------------
// Validation Criterion 6: streaming-unsupported returns Err WITHOUT network
// ---------------------------------------------------------------------------

#[tokio::test]
async fn stream_unsupported_returns_error_without_network_call() {
    let mock = MockServer::start().await;

    // Mount NO mock — any HTTP hit would result in a 404 and we'd see it
    // via a test failure below. The `.expect(0)` assertion on an empty
    // mock server means "zero requests total", which wiremock verifies on
    // `mock.verify()` drop.
    //
    // For extra certainty we set up an always-matching mock that asserts
    // zero calls.
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&mock)
        .await;

    let descriptor = leak_descriptor(&mock, false, ProviderQuirks::DEFAULT);
    let provider = OpenAiCompatProvider::new(descriptor, http(), ApiKey::new("test".into()));

    match provider.stream(sample_request()).await {
        Err(LlmError::Unsupported(msg)) => {
            assert!(
                msg.contains("streaming"),
                "Unsupported error must mention streaming; got: {msg}"
            );
        }
        Err(other) => panic!("expected LlmError::Unsupported, got {other:?}"),
        Ok(_) => panic!("expected Err when descriptor.supports.streaming = false"),
    }

    // Wiremock verifies .expect(0) on drop via Drop impl.
    drop(mock);
}

// ---------------------------------------------------------------------------
// Validation Criterion 7 (adapted): truncated SSE (no [DONE]) still closes
// ---------------------------------------------------------------------------
//
// SPEC DEVIATION NOTE: The original criterion asks for a "mid-stream error
// yielding 1 chunk then Err(Unreachable)". The `stream()` return type is
// `Receiver<StreamEvent>` (not `Receiver<Result<StreamEvent, _>>`), so
// errors can't be delivered as `Err` on the channel — we surface them as
// `StreamEvent::Error`, matching the existing `OpenAiProvider::do_stream`
// pattern. This test exercises the closest wiremock-reproducible case:
// the server serves one valid frame then closes without [DONE], and we
// verify the stream yields the text delta and cleanly terminates with
// `MessageStop` rather than hanging.

#[tokio::test]
async fn sse_truncated_stream_closes_cleanly_with_message_stop() {
    let mock = MockServer::start().await;

    let sse_body = "data: {\"choices\":[{\"delta\":{\"content\":\"only\"}}]}\n\n";

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(sse_body.as_bytes().to_vec(), "text/event-stream"),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let descriptor = leak_descriptor(&mock, true, ProviderQuirks::DEFAULT);
    let provider = OpenAiCompatProvider::new(descriptor, http(), ApiKey::new("test".into()));

    let rx = provider
        .stream(sample_request())
        .await
        .expect("stream() must accept truncated body");
    let events = drain_stream(rx).await;

    let has_only = events
        .iter()
        .any(|e| matches!(e, StreamEvent::TextDelta { text, .. } if text == "only"));
    assert!(
        has_only,
        "expected TextDelta{{text:\"only\"}} in {events:?}"
    );

    let has_stop = events.iter().any(|e| matches!(e, StreamEvent::MessageStop));
    assert!(
        has_stop,
        "stream must terminate with MessageStop even without [DONE] sentinel; got {events:?}"
    );
}

// ---------------------------------------------------------------------------
// Extra safety: 401 surfaces as LlmError::Auth, not a hung stream
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sse_stream_maps_401_to_auth_error() {
    let mock = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
        .expect(1)
        .mount(&mock)
        .await;

    let descriptor = leak_descriptor(&mock, true, ProviderQuirks::DEFAULT);
    let provider = OpenAiCompatProvider::new(descriptor, http(), ApiKey::new("bad".into()));

    match provider.stream(sample_request()).await {
        Err(LlmError::Auth(msg)) => {
            assert!(
                msg.contains("bad key"),
                "auth error should carry body: {msg}"
            );
        }
        Err(other) => panic!("expected LlmError::Auth, got {other:?}"),
        Ok(_) => panic!("expected Err on 401 response"),
    }
}

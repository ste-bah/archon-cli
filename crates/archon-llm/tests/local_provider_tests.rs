/// Tests for Local/Ollama provider adapter (TASK-CLI-405).
/// Written BEFORE implementation (Gate 01).
use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::LocalProvider;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ---------------------------------------------------------------------------
// Test 1: LocalProvider implements LlmProvider (object-safe)
// ---------------------------------------------------------------------------

fn check_object_safe(_: Box<dyn LlmProvider>) {}

#[test]
fn local_provider_is_object_safe() {
    let provider = LocalProvider::new(
        "http://localhost:11434/v1".to_string(),
        "llama3:8b".to_string(),
        300,
        true,
    );
    check_object_safe(Box::new(provider));
}

// ---------------------------------------------------------------------------
// Test 2: Default base URL is the Ollama OpenAI-compat endpoint
// ---------------------------------------------------------------------------

#[test]
fn local_default_base_url_is_ollama() {
    let provider = LocalProvider::default();
    let url = provider.base_url();
    assert_eq!(
        url, "http://localhost:11434/v1",
        "default URL should be Ollama: {url}"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Custom base URL is used
// ---------------------------------------------------------------------------

#[test]
fn local_custom_base_url_used() {
    let provider = LocalProvider::new(
        "http://my-server:8080/v1".to_string(),
        "llama3:8b".to_string(),
        120,
        false,
    );
    assert_eq!(provider.base_url(), "http://my-server:8080/v1");
}

// ---------------------------------------------------------------------------
// Test 4: Model list parsed from Ollama /api/tags response
// ---------------------------------------------------------------------------

#[test]
fn local_model_list_from_ollama_tags() {
    let tags_response = serde_json::json!({
        "models": [
            {"name": "llama3:8b", "size": 4661211136_u64},
            {"name": "mistral:7b", "size": 3825820672_u64}
        ]
    });
    let models = LocalProvider::parse_ollama_tags(&tags_response);
    assert_eq!(models.len(), 2);
    assert_eq!(models[0].id, "llama3:8b");
    assert_eq!(models[1].id, "mistral:7b");
}

// ---------------------------------------------------------------------------
// Test 5: Health check URL is correct
// ---------------------------------------------------------------------------

#[test]
fn local_health_check_url() {
    let provider = LocalProvider::new(
        "http://localhost:11434/v1".to_string(),
        "llama3:8b".to_string(),
        300,
        true,
    );
    let url = provider.health_check_url();
    assert_eq!(
        url, "http://localhost:11434/v1/models",
        "health check URL should be <base>/models, got: {url}"
    );
}

// ---------------------------------------------------------------------------
// Test 6: Timeout is configured on the HTTP client
// ---------------------------------------------------------------------------

#[test]
fn local_timeout_configurable() {
    // LocalProvider with short timeout — just verify it builds without panic.
    let provider = LocalProvider::new(
        "http://localhost:11434/v1".to_string(),
        "llama3:8b".to_string(),
        5, // 5 second timeout
        false,
    );
    // Verify the model is correct.
    assert!(provider.models().iter().any(|m| m.id == "llama3:8b"));
}

// ---------------------------------------------------------------------------
// Test 7: SSE parsing reuses OpenAI format
// ---------------------------------------------------------------------------

#[test]
fn local_usage_only_chunk_preserves_explicit_zero_usage() {
    let chunk =
        r#"{"id":"chatcmpl-local","choices":[],"usage":{"prompt_tokens":0,"completion_tokens":0}}"#;

    let events = LocalProvider::parse_sse_chunk(chunk);

    assert!(matches!(
        events.as_slice(),
        [archon_llm::streaming::StreamEvent::MessageDelta {
            usage: Some(usage),
            ..
        }] if usage.input_tokens_available
            && usage.output_tokens_available
            && usage.input_tokens == 0
            && usage.output_tokens == 0
    ));
}

#[tokio::test]
async fn local_complete_returns_usage_from_final_stream_chunk() {
    let server = MockServer::start().await;
    let body = concat!(
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":4}}\n\n",
        "data: [DONE]\n\n"
    );
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;
    let provider = LocalProvider::new(server.uri(), "local-model".into(), 30, false);

    let response = provider
        .complete(LlmRequest {
            model: "local-model".into(),
            ..LlmRequest::default()
        })
        .await
        .expect("mock completion");

    assert_eq!(response.usage.input_tokens, 9);
    assert_eq!(response.usage.output_tokens, 4);
    assert!(response.usage.input_tokens_available);
    assert!(response.usage.output_tokens_available);
}

#[test]
fn local_uses_openai_sse_format() {
    // LocalProvider should use the same SSE parsing as OpenAI provider.
    let chunk = r#"{"id":"chatcmpl-xyz","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"Hello from Ollama"},"finish_reason":null}]}"#;
    let events = LocalProvider::parse_sse_chunk(chunk);
    let has_text = events.iter().any(|e| {
        matches!(e, archon_llm::streaming::StreamEvent::TextDelta { text, .. } if text == "Hello from Ollama")
    });
    assert!(
        has_text,
        "expected TextDelta with Ollama content, got: {events:?}"
    );
}

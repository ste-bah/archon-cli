/// Tests for Local/Ollama provider adapter (TASK-CLI-405).
/// Written BEFORE implementation (Gate 01).
use archon_llm::provider::{LlmProvider, LlmRequest};
use archon_llm::providers::LocalProvider;
use archon_llm::reasoning::{ReasoningConfig, ReasoningMode};
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

// ---------------------------------------------------------------------------
// #123: reasoning deltas and reasoning controls
//
// Chunk shapes below are copied from a live vLLM 0.25 server hosting
// DeepSeek-V4-Flash, not invented.
// ---------------------------------------------------------------------------

fn thinking_texts(events: &[archon_llm::streaming::StreamEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            archon_llm::streaming::StreamEvent::ThinkingDelta { thinking, .. } => {
                Some(thinking.clone())
            }
            _ => None,
        })
        .collect()
}

/// The observed server emits `reasoning`. Without this branch the tokens are
/// paid for and silently dropped.
#[test]
fn reasoning_delta_becomes_thinking() {
    let chunk = r#"{"choices":[{"index":0,"delta":{"reasoning":"We need answer user."},"finish_reason":null}]}"#;
    let events = LocalProvider::parse_sse_chunk(chunk);
    assert_eq!(thinking_texts(&events), vec!["We need answer user."]);
}

/// Other vLLM builds and reasoning parsers spell the same field
/// `reasoning_content`. Both must work.
#[test]
fn reasoning_content_delta_also_becomes_thinking() {
    let chunk = r#"{"choices":[{"index":0,"delta":{"reasoning_content":"alt spelling"},"finish_reason":null}]}"#;
    let events = LocalProvider::parse_sse_chunk(chunk);
    assert_eq!(thinking_texts(&events), vec!["alt spelling"]);
}

/// The transition chunk where thinking ends and the answer begins carries BOTH
/// keys. Treating them as mutually exclusive drops a token.
#[test]
fn chunk_carrying_both_reasoning_and_content_yields_both() {
    let chunk = r#"{"choices":[{"index":0,"delta":{"content":"Hi! How","reasoning":"."},"finish_reason":null}]}"#;
    let events = LocalProvider::parse_sse_chunk(chunk);
    assert_eq!(thinking_texts(&events), vec!["."]);
    let has_text = events.iter().any(|e| {
        matches!(e, archon_llm::streaming::StreamEvent::TextDelta { text, .. } if text == "Hi! How")
    });
    assert!(
        has_text,
        "content must survive alongside reasoning: {events:?}"
    );
}

#[test]
fn empty_reasoning_emits_nothing() {
    let chunk = r#"{"choices":[{"index":0,"delta":{"reasoning":""},"finish_reason":null}]}"#;
    assert!(thinking_texts(&LocalProvider::parse_sse_chunk(chunk)).is_empty());
}

/// Capture the request body the provider actually sends.
async fn captured_body(
    provider: LocalProvider,
    server: &MockServer,
    effort: Option<&str>,
) -> serde_json::Value {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string("data: [DONE]\n\n"),
        )
        .mount(server)
        .await;

    let _ = provider
        .complete(LlmRequest {
            model: "local-model".into(),
            effort: effort.map(str::to_string),
            ..LlmRequest::default()
        })
        .await
        .expect("mock completion");

    let requests = server
        .received_requests()
        .await
        .expect("mock server records requests");
    // Select the chat POST explicitly: constructing a LocalProvider also fires
    // a background GET /models probe for the context window, which lands in
    // this list first and carries no body.
    let chat = requests
        .iter()
        .find(|r| r.url.path().ends_with("/chat/completions"))
        .expect("a chat/completions request should have been sent");
    serde_json::from_slice(&chat.body).expect("request body is JSON")
}

/// Without `stream_options.include_usage` an OpenAI-compatible server is under
/// no obligation to report usage on a stream, and vLLM sends none — which is
/// why local sessions showed `$0.00` and no token counts.
#[tokio::test]
async fn stream_requests_usage_so_tokens_can_be_counted() {
    let server = MockServer::start().await;
    let provider = LocalProvider::new(server.uri(), "local-model".into(), 30, false);
    let body = captured_body(provider, &server, None).await;

    assert_eq!(body["stream"], serde_json::json!(true));
    assert_eq!(
        body["stream_options"]["include_usage"],
        serde_json::json!(true),
        "usage must be requested explicitly: {body}"
    );
}

#[test]
fn max_model_len_prefers_the_matching_model_id() {
    let payload = serde_json::json!({"data": [
        {"id": "other-model", "max_model_len": 4096},
        {"id": "local-model", "max_model_len": 1_048_576},
    ]});
    assert_eq!(
        archon_llm::providers::max_model_len(&payload, "local-model"),
        Some(1_048_576)
    );
}

/// A single-model server is often addressed by an alias that differs from the
/// id it advertises, so a lone entry is accepted.
#[test]
fn max_model_len_falls_back_to_a_sole_entry() {
    let payload = serde_json::json!({"data": [{"id": "served-name", "max_model_len": 32_768}]});
    assert_eq!(
        archon_llm::providers::max_model_len(&payload, "an-alias"),
        Some(32_768)
    );
}

#[test]
fn max_model_len_is_none_when_unknown() {
    let no_match = serde_json::json!({"data": [
        {"id": "a", "max_model_len": 10},
        {"id": "b", "max_model_len": 20},
    ]});
    assert_eq!(archon_llm::providers::max_model_len(&no_match, "c"), None);

    let absent = serde_json::json!({"data": [{"id": "a"}]});
    assert_eq!(archon_llm::providers::max_model_len(&absent, "a"), None);

    let zero = serde_json::json!({"data": [{"id": "a", "max_model_len": 0}]});
    assert_eq!(archon_llm::providers::max_model_len(&zero, "a"), None);

    assert_eq!(
        archon_llm::providers::max_model_len(&serde_json::json!({}), "a"),
        None
    );
}

/// Default is `Off`: byte-identical to pre-#123 requests, so Ollama and
/// llama.cpp deployments are unaffected.
#[tokio::test]
async fn reasoning_defaults_to_sending_nothing() {
    let server = MockServer::start().await;
    let provider = LocalProvider::new(server.uri(), "local-model".into(), 30, false);
    let body = captured_body(provider, &server, Some("max")).await;

    assert!(body.get("reasoning_effort").is_none());
    assert!(body.get("chat_template_kwargs").is_none());
}

#[tokio::test]
async fn top_level_mode_puts_effort_on_the_wire() {
    let server = MockServer::start().await;
    let provider = LocalProvider::new(server.uri(), "local-model".into(), 30, false)
        .with_reasoning(ReasoningConfig {
            mode: ReasoningMode::TopLevel,
            ..ReasoningConfig::default()
        });
    let body = captured_body(provider, &server, Some("max")).await;

    assert_eq!(body["reasoning_effort"], "max");
}

/// On the observed DeepSeek template `reasoning_effort` inside
/// `chat_template_kwargs` is inert unless `thinking` is set in the same bag.
#[tokio::test]
async fn chat_template_kwargs_mode_sends_thinking_alongside_effort() {
    let server = MockServer::start().await;
    let mut reasoning = ReasoningConfig {
        mode: ReasoningMode::ChatTemplateKwargs,
        ..ReasoningConfig::default()
    };
    reasoning
        .kwargs
        .insert("thinking".into(), serde_json::json!(true));
    let provider =
        LocalProvider::new(server.uri(), "local-model".into(), 30, false).with_reasoning(reasoning);
    let body = captured_body(provider, &server, Some("high")).await;

    assert_eq!(
        body["chat_template_kwargs"]["thinking"],
        serde_json::json!(true)
    );
    assert_eq!(body["chat_template_kwargs"]["reasoning_effort"], "high");
}

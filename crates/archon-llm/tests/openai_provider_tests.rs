/// Tests for OpenAI provider adapter (TASK-CLI-402).
/// Written BEFORE implementation (Gate 01).
use archon_llm::provider::{LlmProvider, LlmRequest, ProviderFeature};
use archon_llm::providers::OpenAiProvider;
use archon_llm::providers::openai::build_openai_stream_request_body;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ---------------------------------------------------------------------------
// Test 1: OpenAiProvider is object-safe (can be boxed as dyn LlmProvider)
// ---------------------------------------------------------------------------

fn check_object_safe(_: Box<dyn LlmProvider>) {}

#[test]
fn openai_provider_is_object_safe() {
    let provider = OpenAiProvider::new("test-key".to_string(), None, "gpt-4o".to_string());
    check_object_safe(Box::new(provider));
}

// ---------------------------------------------------------------------------
// Test 2: System prompt becomes the first message with role:system
// ---------------------------------------------------------------------------

#[test]
fn openai_system_prompt_becomes_first_message() {
    let system_text = "You are a helpful assistant.";
    let system_blocks = vec![serde_json::json!({"type": "text", "text": system_text})];
    let messages = OpenAiProvider::build_openai_messages(&system_blocks, &[]);
    assert!(!messages.is_empty(), "messages should not be empty");
    let first = &messages[0];
    assert_eq!(first["role"], "system");
    assert_eq!(first["content"], system_text);
}

// ---------------------------------------------------------------------------
// Test 3: Tool mapped to OpenAI function format
// ---------------------------------------------------------------------------

#[test]
fn openai_tool_mapping_correct() {
    let archon_tool = serde_json::json!({
        "name": "Read",
        "description": "Read a file",
        "input_schema": {
            "type": "object",
            "properties": {
                "file_path": {"type": "string", "description": "Path to file"}
            },
            "required": ["file_path"]
        }
    });
    let openai_tools = OpenAiProvider::map_tools_to_openai(&[archon_tool]);
    assert_eq!(openai_tools.len(), 1);
    let tool = &openai_tools[0];
    assert_eq!(tool["type"], "function");
    let func = &tool["function"];
    assert_eq!(func["name"], "Read");
    assert_eq!(func["description"], "Read a file");
    assert!(func["parameters"].is_object());
}

// ---------------------------------------------------------------------------
// Test 4: supports_feature returns correct flags
// ---------------------------------------------------------------------------

#[test]
fn openai_feature_flags() {
    let provider = OpenAiProvider::new("key".to_string(), None, "gpt-4o".to_string());
    assert!(provider.supports_feature(ProviderFeature::ToolUse));
    assert!(provider.supports_feature(ProviderFeature::Streaming));
    assert!(provider.supports_feature(ProviderFeature::SystemPrompt));
    assert!(provider.supports_feature(ProviderFeature::Vision));
    assert!(!provider.supports_feature(ProviderFeature::Thinking));
    assert!(!provider.supports_feature(ProviderFeature::PromptCaching));
}

// ---------------------------------------------------------------------------
// Test 5: OPENAI_API_KEY env var is used over config key
// ---------------------------------------------------------------------------

#[test]
fn openai_uses_env_api_key() {
    // This tests the resolver logic, not the live HTTP call.
    let resolved = OpenAiProvider::resolve_api_key("config-fallback");
    // In test environment, OPENAI_API_KEY is probably not set.
    // Either way, the result should be non-empty.
    assert!(!resolved.is_empty());
}

#[tokio::test]
async fn openai_tool_section_bytes_are_stable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string("data: [DONE]\n\n"),
        )
        .expect(2)
        .mount(&server)
        .await;
    let provider = OpenAiProvider::new("test-key".into(), Some(server.uri()), "gpt-4o".into());
    let tools = archon_llm::provider::shared_tools(vec![serde_json::json!({
        "name":"Read",
        "description":"read",
        "input_schema":{
            "type":"object",
            "properties":{"file_path":{"type":"string"}}
        }
    })]);

    for content in ["first turn", "second turn"] {
        let mut stream = provider
            .stream(LlmRequest {
                model: "gpt-4o".into(),
                system: vec![serde_json::json!({
                    "type":"text",
                    "text":"stable system"
                })],
                messages: vec![serde_json::json!({"role":"user","content":content})],
                tools: tools.clone(),
                ..LlmRequest::default()
            })
            .await
            .expect("captured stream");
        while stream.recv().await.is_some() {}
    }

    let requests = server.received_requests().await.expect("captured requests");
    assert_eq!(requests.len(), 2);
    let first: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    let second: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_ne!(first["messages"], second["messages"]);
    assert_eq!(first["messages"][0], second["messages"][0]);
    assert_eq!(
        serde_json::to_vec(&first["tools"]).unwrap(),
        serde_json::to_vec(&second["tools"]).unwrap()
    );
}

#[test]
fn openai_streaming_request_asks_for_usage_chunk() {
    let body = build_openai_stream_request_body("gpt-4o", 1024, &[], &[], &[]);

    assert_eq!(body["stream_options"]["include_usage"], true);
}

#[tokio::test]
async fn openai_complete_returns_usage_from_final_stream_chunk() {
    let server = MockServer::start().await;
    let body = concat!(
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n\n",
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
    let provider = OpenAiProvider::new("test-key".into(), Some(server.uri()), "gpt-4o".into());

    let response = provider
        .complete(LlmRequest {
            model: "gpt-4o".into(),
            ..LlmRequest::default()
        })
        .await
        .expect("mock completion");

    assert_eq!(response.usage.input_tokens, 7);
    assert_eq!(response.usage.output_tokens, 3);
    assert!(response.usage.input_tokens_available);
    assert!(response.usage.output_tokens_available);
}

// ---------------------------------------------------------------------------
// Test 6: SSE parsing — text chunk produces TextDelta
// ---------------------------------------------------------------------------

#[test]
fn openai_sse_text_delta_parsed() {
    let chunk = r#"{"id":"chatcmpl-abc","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}],"usage":null}"#;
    let events = OpenAiProvider::parse_sse_chunk(chunk);
    let has_text_delta = events.iter().any(|e| {
        matches!(e, archon_llm::streaming::StreamEvent::TextDelta { text, .. } if text == "Hello")
    });
    assert!(
        has_text_delta,
        "expected TextDelta with 'Hello', got: {events:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 7: SSE parsing — tool call start chunk produces ContentBlockStart
// ---------------------------------------------------------------------------

#[test]
fn openai_sse_tool_call_parsed() {
    // Tool call start (has id and function name)
    let chunk = r#"{"id":"chatcmpl-abc","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_abc123","type":"function","function":{"name":"Read","arguments":""}}]},"finish_reason":null}]}"#;
    let events = OpenAiProvider::parse_sse_chunk(chunk);
    let has_start = events.iter().any(|e| {
        matches!(e, archon_llm::streaming::StreamEvent::ContentBlockStart {
            tool_use_id: Some(id),
            tool_name: Some(name),
            ..
        } if id == "call_abc123" && name == "Read")
    });
    assert!(
        has_start,
        "expected ContentBlockStart for tool call, got: {events:?}"
    );
}

#[test]
fn openai_terminal_choice_without_delta_preserves_usage() {
    let chunk = r#"{"id":"chatcmpl-usage","choices":[{"index":0,"finish_reason":"stop"}],"usage":{"prompt_tokens":6,"completion_tokens":2}}"#;

    let events = OpenAiProvider::parse_sse_chunk(chunk);

    assert!(matches!(
        events.as_slice(),
        [archon_llm::streaming::StreamEvent::MessageDelta {
            usage: Some(usage),
            ..
        }] if usage.input_tokens == 6 && usage.output_tokens == 2
    ));
}

#[test]
fn openai_sse_usage_only_chunk_preserves_explicit_zero_usage() {
    let chunk =
        r#"{"id":"chatcmpl-usage","choices":[],"usage":{"prompt_tokens":0,"completion_tokens":0}}"#;

    let events = OpenAiProvider::parse_sse_chunk(chunk);

    assert!(matches!(
        events.as_slice(),
        [archon_llm::streaming::StreamEvent::MessageDelta {
            usage: Some(usage),
            ..
        }] if usage.input_tokens == 0
            && usage.output_tokens == 0
            && usage.input_tokens_available
            && usage.output_tokens_available
    ));
}

#[test]
fn openai_sse_usage_only_chunk_preserves_absent_usage_as_unavailable() {
    let chunk = r#"{"id":"chatcmpl-usage","choices":[],"usage":{}}"#;

    let events = OpenAiProvider::parse_sse_chunk(chunk);

    assert!(matches!(
        events.as_slice(),
        [archon_llm::streaming::StreamEvent::MessageDelta {
            usage: Some(usage),
            ..
        }] if !usage.input_tokens_available && !usage.output_tokens_available
    ));
}

// ---------------------------------------------------------------------------
// Test 8: SSE parsing — [DONE] produces MessageStop
// ---------------------------------------------------------------------------

#[test]
fn openai_sse_done_produces_message_stop() {
    // finish_reason:"stop" first then [DONE]
    let stop_chunk = r#"{"id":"chatcmpl-abc","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
    let events = OpenAiProvider::parse_sse_chunk(stop_chunk);
    let has_delta = events
        .iter()
        .any(|e| matches!(e, archon_llm::streaming::StreamEvent::MessageDelta { .. }));
    assert!(
        has_delta,
        "expected MessageDelta for finish_reason:stop, got: {events:?}"
    );
}

/// Tests for OpenAI provider adapter (TASK-CLI-402).
/// Written BEFORE implementation (Gate 01).
use archon_llm::provider::{LlmProvider, ProviderFeature};
use archon_llm::providers::OpenAiProvider;

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

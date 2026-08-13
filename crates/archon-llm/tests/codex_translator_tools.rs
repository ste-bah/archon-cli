use std::path::PathBuf;

use archon_llm::provider::LlmRequest;
use archon_llm::providers::codex::client::CodexProvider;
use archon_llm::providers::codex::spoof_default::SpoofConfig;
use archon_llm::providers::codex::translator::tools_to_responses_tools;

#[test]
fn maps_anthropic_tool_schema_to_response_tool() {
    let tools = vec![serde_json::json!({
        "name": "lookup",
        "description": "Lookup thing",
        "input_schema": {"type": "object", "properties": {"q": {"type": "string"}}}
    })];

    let mapped = tools_to_responses_tools(&tools).expect("tools");
    assert_eq!(mapped[0].kind, "function");
    assert_eq!(mapped[0].name, "lookup");
    assert_eq!(mapped[0].description.as_deref(), Some("Lookup thing"));
}

#[test]
fn codex_tool_section_bytes_are_stable() {
    let provider = CodexProvider::new(
        PathBuf::from("/tmp/archon-test-codex-auth.json"),
        SpoofConfig::default(),
        reqwest::Client::new(),
    )
    .expect("provider");
    let request = LlmRequest {
        model: "gpt-5.3-codex".into(),
        tools: archon_llm::provider::shared_tools(vec![serde_json::json!({
            "name":"lookup",
            "description":"Lookup thing",
            "input_schema":{
                "type":"object",
                "properties":{"q":{"type":"string"}}
            }
        })]),
        ..LlmRequest::default()
    };

    let first = provider.build_request_body(&request).expect("first body");
    let second = provider.build_request_body(&request).expect("second body");

    assert_eq!(
        serde_json::to_vec(&first.tools).unwrap(),
        serde_json::to_vec(&second.tools).unwrap()
    );
    // Two requests with the same model, system prompt and tools must carry the
    // same cache key. It used to be a fresh UUID per request, which is the one
    // value that guarantees a miss: the key is how the service groups requests
    // onto a cache, so a key that never repeats never hits.
    assert_eq!(first.prompt_cache_key, second.prompt_cache_key);
    assert!(first.prompt_cache_key.is_some());
}

/// The key must still separate prefixes that genuinely cannot share a cache.
/// Stability is only useful if it is stability of the *right* thing.
#[test]
fn a_different_prefix_gets_a_different_cache_key() {
    let provider = CodexProvider::new(
        PathBuf::from("/tmp/archon-test-codex-auth.json"),
        SpoofConfig::default(),
        reqwest::Client::new(),
    )
    .expect("provider");

    let base = LlmRequest {
        model: "gpt-5.3-codex".into(),
        system: vec![serde_json::json!({"type": "text", "text": "you are archon"})],
        ..LlmRequest::default()
    };
    let baseline = provider.build_request_body(&base).expect("baseline");

    let other_system = provider
        .build_request_body(&LlmRequest {
            system: vec![serde_json::json!({"type": "text", "text": "you are something else"})],
            ..base.clone()
        })
        .expect("other system");
    assert_ne!(baseline.prompt_cache_key, other_system.prompt_cache_key);

    let other_model = provider
        .build_request_body(&LlmRequest {
            model: "gpt-5.4".into(),
            ..base.clone()
        })
        .expect("other model");
    assert_ne!(baseline.prompt_cache_key, other_model.prompt_cache_key);

    let other_tools = provider
        .build_request_body(&LlmRequest {
            tools: archon_llm::provider::shared_tools(vec![serde_json::json!({
                "name": "lookup",
                "description": "Lookup thing",
                "input_schema": {"type": "object"}
            })]),
            ..base.clone()
        })
        .expect("other tools");
    assert_ne!(baseline.prompt_cache_key, other_tools.prompt_cache_key);
}

/// The conversation must **not** feed the key. Including it would rebuild the
/// per-request-uniqueness bug by hand, since the messages change every turn.
#[test]
fn the_conversation_does_not_feed_the_cache_key() {
    let provider = CodexProvider::new(
        PathBuf::from("/tmp/archon-test-codex-auth.json"),
        SpoofConfig::default(),
        reqwest::Client::new(),
    )
    .expect("provider");

    let turn = |text: &str| LlmRequest {
        model: "gpt-5.3-codex".into(),
        system: vec![serde_json::json!({"type": "text", "text": "you are archon"})],
        messages: vec![serde_json::json!({"role": "user", "content": text})],
        ..LlmRequest::default()
    };

    let first = provider
        .build_request_body(&turn("first turn"))
        .expect("first");
    let second = provider
        .build_request_body(&turn("second turn"))
        .expect("second");

    assert_eq!(
        first.prompt_cache_key, second.prompt_cache_key,
        "turns of one session share a prefix and must share its cache key"
    );
}

#[test]
fn missing_name_is_error() {
    let err = tools_to_responses_tools(&[serde_json::json!({"description": "no"})])
        .expect_err("missing name");

    assert!(err.to_string().contains("tool missing name"));
}

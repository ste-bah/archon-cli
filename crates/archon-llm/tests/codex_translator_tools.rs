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
    assert_ne!(first.prompt_cache_key, second.prompt_cache_key);
}

#[test]
fn missing_name_is_error() {
    let err = tools_to_responses_tools(&[serde_json::json!({"description": "no"})])
        .expect_err("missing name");

    assert!(err.to_string().contains("tool missing name"));
}

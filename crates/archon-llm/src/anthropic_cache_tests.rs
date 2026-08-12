use crate::anthropic::{AnthropicClient, MessageRequest};
use crate::anthropic_tests::{make_auth, make_identity};
use crate::identity::{IdentityMode, IdentityProvider};
use crate::provider::{LlmProvider, ProviderFeature};
use crate::providers::AnthropicProvider;

/// `MessageRequest::tools` is shared rather than owned (#171 part 3), so the
/// test fixtures build it through the same wrapper the runtime uses.
macro_rules! shared_tools {
    ($($tool:expr),* $(,)?) => { crate::provider::shared_tools(vec![$($tool),*]) };
}

fn spoof_identity() -> IdentityProvider {
    IdentityProvider::new(
        IdentityMode::Spoof {
            version: "2.1.89".to_string(),
            entrypoint: "cli".to_string(),
            betas: Vec::new(),
            workload: None,
            anti_distillation: false,
        },
        "test-session".to_string(),
        "test-device".to_string(),
        String::new(),
    )
}

#[test]
fn prompt_caching_capability_requires_official_anthropic_endpoint() {
    let direct = AnthropicProvider::new(AnthropicClient::new(make_auth(), make_identity(), None));
    let proxy = AnthropicProvider::new(AnthropicClient::new(
        make_auth(),
        make_identity(),
        Some("http://localhost:11434/v1/messages".to_string()),
    ));

    assert!(direct.supports_feature(ProviderFeature::PromptCaching));
    assert!(!proxy.supports_feature(ProviderFeature::PromptCaching));
}

#[test]
fn proxy_wire_does_not_add_anthropic_tool_cache_marker() {
    let client = AnthropicClient::new(
        make_auth(),
        make_identity(),
        Some("http://localhost:11434/v1/messages".to_string()),
    );
    let request = MessageRequest {
        messages: vec![serde_json::json!({"role":"user","content":"hello"})],
        tools: shared_tools![serde_json::json!({
            "name":"Read",
            "description":"read",
            "input_schema":{"type":"object"}
        })],
        ..MessageRequest::default()
    };

    let body: serde_json::Value =
        serde_json::from_str(&client.build_request_body(&request).unwrap()).unwrap();

    assert_eq!(body["tools"][0].get("cache_control"), None);
}

#[test]
fn spoof_proxy_wire_strips_anthropic_directives_but_preserves_schema_property() {
    let client = AnthropicClient::new(
        make_auth(),
        spoof_identity(),
        Some("http://localhost:11434/v1/messages".to_string()),
    );
    let request = MessageRequest {
        system: vec![serde_json::json!({
            "type":"text",
            "text":"stable",
            "cache_control":{"type":"ephemeral"}
        })],
        messages: vec![serde_json::json!({
            "role":"user",
            "content":[{
                "type":"text",
                "text":"latest",
                "cache_control":{"type":"ephemeral"}
            }]
        })],
        tools: shared_tools![serde_json::json!({
            "name":"Configure",
            "description":"configure",
            "cache_control":{"type":"ephemeral"},
            "input_schema":{
                "type":"object",
                "properties":{
                    "cache_control":{"type":"string"}
                }
            }
        })],
        ..MessageRequest::default()
    };

    let body = client.build_request_body(&request).unwrap();
    let body_json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(body.matches("\"cache_control\"").count(), 1);
    assert_eq!(body_json["system"][0].get("cache_control"), None);
    assert_eq!(
        body_json["messages"][0]["content"][0].get("cache_control"),
        None
    );
    assert_eq!(body_json["tools"][0].get("cache_control"), None);
    assert_eq!(
        body_json["tools"][0]["input_schema"]["properties"]["cache_control"]["type"],
        "string"
    );
}

#[test]
fn spoof_wire_keeps_conversation_cache_marker_when_budget_allows() {
    let client = AnthropicClient::new(make_auth(), spoof_identity(), None);
    let request = MessageRequest {
        messages: vec![serde_json::json!({
            "role": "user",
            "content": [{
                "type": "text",
                "text": "latest",
                "cache_control": {"type": "ephemeral"}
            }]
        })],
        tools: shared_tools![serde_json::json!({
            "name": "Read",
            "description": "read",
            "input_schema": {"type": "object"}
        })],
        ..MessageRequest::default()
    };

    let body = client.build_request_body(&request).unwrap();
    let body_json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(body.matches("\"cache_control\"").count(), 4);
    assert_eq!(
        body_json["messages"][0]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
}

#[test]
fn spoof_wire_billing_fingerprint_uses_array_message_text() {
    let identity = spoof_identity();
    let expected_billing = identity.billing_header("actual first prompt").unwrap();
    let client = AnthropicClient::new(make_auth(), identity, None);
    let request = MessageRequest {
        messages: vec![serde_json::json!({
            "role": "user",
            "content": [{
                "type": "text",
                "text": "actual first prompt",
                "cache_control": {"type": "ephemeral"}
            }]
        })],
        ..MessageRequest::default()
    };

    let body = client.build_request_body(&request).unwrap();
    let body_json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(body_json["system"][0]["text"], expected_billing);
}

#[test]
fn spoof_wire_does_not_duplicate_existing_billing_cache_marker() {
    let identity = spoof_identity();
    let billing = identity.billing_header("latest").unwrap();
    let client = AnthropicClient::new(make_auth(), identity, None);
    let request = MessageRequest {
        system: vec![serde_json::json!({
            "type": "text",
            "text": billing,
            "cache_control": {"type": "ephemeral"}
        })],
        messages: vec![serde_json::json!({
            "role": "user",
            "content": [{
                "type": "text",
                "text": "latest",
                "cache_control": {"type": "ephemeral"}
            }]
        })],
        tools: shared_tools![serde_json::json!({
            "name": "Read",
            "description": "read",
            "input_schema": {"type": "object"}
        })],
        ..MessageRequest::default()
    };

    let body = client.build_request_body(&request).unwrap();
    let body_json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let system = body_json["system"].as_array().unwrap();

    assert_eq!(body.matches("x-anthropic-billing-header:").count(), 1);
    assert_eq!(body.matches("\"cache_control\"").count(), 4);
    assert_eq!(
        body_json["messages"][0]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
    assert_eq!(
        system
            .iter()
            .filter(|block| block["text"]
                .as_str()
                .is_some_and(|text| text.starts_with("You are Claude Code,")))
            .count(),
        1
    );
}

#[test]
fn spoof_wire_prioritizes_conversation_marker_over_tool_marker() {
    let client = AnthropicClient::new(make_auth(), spoof_identity(), None);
    let request = MessageRequest {
        system: vec![serde_json::json!({
            "type": "text",
            "text": "stable system",
            "cache_control": {"type": "ephemeral"}
        })],
        messages: vec![serde_json::json!({
            "role": "user",
            "content": [{
                "type": "text",
                "text": "latest",
                "cache_control": {"type": "ephemeral"}
            }]
        })],
        tools: shared_tools![serde_json::json!({
            "name": "Read",
            "description": "read",
            "input_schema": {"type": "object"}
        })],
        ..MessageRequest::default()
    };

    let body = client.build_request_body(&request).unwrap();
    let body_json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(body.matches("\"cache_control\"").count(), 4);
    assert_eq!(
        body_json["messages"][0]["content"][0]["cache_control"]["type"],
        "ephemeral"
    );
    assert_eq!(body_json["tools"][0].get("cache_control"), None);
}

#[test]
fn official_anthropic_tool_section_bytes_are_stable() {
    let client = AnthropicClient::new(make_auth(), spoof_identity(), None);
    let request = MessageRequest {
        messages: vec![serde_json::json!({"role":"user","content":"hello"})],
        tools: shared_tools![serde_json::json!({
            "name":"Read",
            "description":"read",
            "input_schema":{
                "type":"object",
                "properties":{"file_path":{"type":"string"}}
            }
        })],
        ..MessageRequest::default()
    };

    let first: serde_json::Value =
        serde_json::from_str(&client.build_request_body(&request).unwrap()).unwrap();
    let second: serde_json::Value =
        serde_json::from_str(&client.build_request_body(&request).unwrap()).unwrap();

    assert_eq!(
        serde_json::to_vec(&first["tools"]).unwrap(),
        serde_json::to_vec(&second["tools"]).unwrap()
    );
}

#[test]
fn cache_budget_preserves_tool_schema_property_named_cache_control() {
    let client = AnthropicClient::new(make_auth(), spoof_identity(), None);
    let request = MessageRequest {
        system: vec![serde_json::json!({
            "type":"text",
            "text":"stable",
            "cache_control":{"type":"ephemeral"}
        })],
        messages: vec![serde_json::json!({
            "role":"user",
            "content":[{
                "type":"text",
                "text":"latest",
                "cache_control":{"type":"ephemeral"}
            }]
        })],
        tools: shared_tools![serde_json::json!({
            "name":"Configure",
            "description":"configure",
            "input_schema":{
                "type":"object",
                "properties":{
                    "cache_control":{"type":"string"}
                }
            }
        })],
        ..MessageRequest::default()
    };

    let body = client.build_request_body(&request).unwrap();
    let body_json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(
        body_json["tools"][0]["input_schema"]["properties"]["cache_control"]["type"],
        "string"
    );
    assert_eq!(body_json["tools"][0].get("cache_control"), None);
}

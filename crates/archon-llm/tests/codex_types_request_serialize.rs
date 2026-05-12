use archon_llm::providers::codex::types::{
    ReasoningConfig, ResponseContentBlock, ResponseInputItem, ResponseTool, ResponsesRequest,
    TextConfig,
};

fn base_request() -> ResponsesRequest {
    ResponsesRequest {
        model: "gpt-5.3-codex".into(),
        store: false,
        stream: true,
        instructions: None,
        input: vec![ResponseInputItem::Message {
            role: "user".into(),
            content: vec![ResponseContentBlock::InputText { text: "hi".into() }],
        }],
        tools: None,
        tool_choice: None,
        parallel_tool_calls: None,
        temperature: None,
        reasoning: None,
        service_tier: None,
        text: None,
        include: None,
        prompt_cache_key: None,
    }
}

#[test]
fn minimal_request_omits_optional_fields() {
    let value = serde_json::to_value(base_request()).expect("serialize");

    assert_eq!(value["model"], "gpt-5.3-codex");
    assert_eq!(value["input"][0]["type"], "message");
    assert!(value.get("instructions").is_none());
    assert!(value.get("tools").is_none());
}

#[test]
fn request_serializes_instructions_and_reasoning() {
    let mut req = base_request();
    req.instructions = Some("system".into());
    req.reasoning = Some(ReasoningConfig {
        effort: Some("high".into()),
        summary: Some("auto".into()),
    });

    let value = serde_json::to_value(req).expect("serialize");
    assert_eq!(value["instructions"], "system");
    assert_eq!(value["reasoning"]["effort"], "high");
}

#[test]
fn request_serializes_tools_and_text_config() {
    let mut req = base_request();
    req.tools = Some(vec![ResponseTool {
        kind: "function".into(),
        name: "lookup".into(),
        description: Some("Lookup".into()),
        parameters: serde_json::json!({"type": "object"}),
        strict: Some(true),
    }]);
    req.text = Some(TextConfig {
        verbosity: Some("low".into()),
    });

    let value = serde_json::to_value(req).expect("serialize");
    assert_eq!(value["tools"][0]["type"], "function");
    assert_eq!(value["text"]["verbosity"], "low");
}

#[test]
fn request_serializes_reasoning_input_item() {
    let mut req = base_request();
    req.input.insert(
        0,
        ResponseInputItem::Reasoning {
            encrypted_content: "opaque".into(),
        },
    );

    let value = serde_json::to_value(req).expect("serialize");
    assert_eq!(value["input"][0]["type"], "reasoning");
    assert_eq!(value["input"][0]["encrypted_content"], "opaque");
}

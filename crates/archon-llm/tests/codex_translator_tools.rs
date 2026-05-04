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
fn missing_name_is_error() {
    let err = tools_to_responses_tools(&[serde_json::json!({"description": "no"})])
        .expect_err("missing name");

    assert!(err.to_string().contains("tool missing name"));
}

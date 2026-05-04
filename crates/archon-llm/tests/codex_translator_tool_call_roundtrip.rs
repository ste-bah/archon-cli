use archon_llm::provider::LlmRequest;
use archon_llm::providers::codex::translator::messages_to_responses_input;
use archon_llm::providers::codex::types::ResponseInputItem;

#[test]
fn tool_use_maps_to_function_call() {
    let req = LlmRequest {
        messages: vec![
            serde_json::json!({"role": "assistant", "content": [{"type": "tool_use", "id": "call_1", "name": "lookup", "input": {"q": "x"}}]}),
        ],
        ..LlmRequest::default()
    };

    let input = messages_to_responses_input(&req).expect("translate");
    assert!(matches!(
        &input[0],
        ResponseInputItem::FunctionCall { call_id, name, .. }
            if call_id == "call_1" && name == "lookup"
    ));
}

#[test]
fn errored_tool_result_gets_error_prefix() {
    let req = LlmRequest {
        messages: vec![
            serde_json::json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": "call_1", "content": "bad", "is_error": true}]}),
        ],
        ..LlmRequest::default()
    };

    let input = messages_to_responses_input(&req).expect("translate");
    assert!(matches!(
        &input[0],
        ResponseInputItem::FunctionCallOutput { output, .. } if output == "[ERROR]: bad"
    ));
}

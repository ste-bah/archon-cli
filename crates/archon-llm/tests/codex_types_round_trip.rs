use archon_llm::providers::codex::types::{ResponseOutputItem, ResponseSnapshot, ResponseUsage};

#[test]
fn response_snapshot_deserializes_core_fields() {
    let snapshot: ResponseSnapshot = serde_json::from_value(serde_json::json!({
        "id": "resp_1",
        "status": "completed",
        "model": "gpt-5.1-codex",
        "usage": {"input_tokens": 1, "output_tokens": 2, "total_tokens": 3}
    }))
    .expect("snapshot");

    assert_eq!(snapshot.id, "resp_1");
    assert_eq!(snapshot.model.as_deref(), Some("gpt-5.1-codex"));
    assert_eq!(snapshot.usage.expect("usage").total_tokens, 3);
}

#[test]
fn response_usage_details_are_optional() {
    let usage: ResponseUsage = serde_json::from_value(serde_json::json!({
        "input_tokens": 1,
        "output_tokens": 2,
        "total_tokens": 3
    }))
    .expect("usage");

    assert!(usage.input_tokens_details.is_none());
    assert!(usage.output_tokens_details.is_none());
}

#[test]
fn reasoning_output_item_carries_encrypted_content() {
    let item: ResponseOutputItem = serde_json::from_value(serde_json::json!({
        "type": "reasoning",
        "id": "rs_1",
        "encrypted_content": "opaque",
        "status": "completed"
    }))
    .expect("reasoning item");

    match item {
        ResponseOutputItem::Reasoning {
            encrypted_content, ..
        } => assert_eq!(encrypted_content.as_deref(), Some("opaque")),
        other => panic!("unexpected item: {other:?}"),
    }
}

#[test]
fn function_call_output_item_carries_arguments() {
    let item: ResponseOutputItem = serde_json::from_value(serde_json::json!({
        "type": "function_call",
        "id": "fc_1",
        "call_id": "call_1",
        "name": "lookup",
        "arguments": "{}",
        "status": "completed"
    }))
    .expect("function call");

    match item {
        ResponseOutputItem::FunctionCall {
            name, arguments, ..
        } => {
            assert_eq!(name, "lookup");
            assert_eq!(arguments, "{}");
        }
        other => panic!("unexpected item: {other:?}"),
    }
}

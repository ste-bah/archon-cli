use archon_llm::providers::codex::types::{ResponseOutputItem, ResponseStreamEvent};

#[test]
fn unknown_stream_event_deserializes_to_unknown() {
    let event: ResponseStreamEvent =
        serde_json::from_value(serde_json::json!({"type": "response.future"})).expect("unknown");

    assert!(matches!(event, ResponseStreamEvent::Unknown));
}

#[test]
fn missing_type_is_error_not_unknown() {
    let err = serde_json::from_value::<ResponseStreamEvent>(serde_json::json!({}))
        .expect_err("missing type");

    assert!(err.to_string().contains("missing field"));
}

#[test]
fn unknown_output_item_deserializes_to_unknown() {
    let item: ResponseOutputItem =
        serde_json::from_value(serde_json::json!({"type": "future_item"})).expect("unknown item");

    assert!(matches!(item, ResponseOutputItem::Unknown));
}

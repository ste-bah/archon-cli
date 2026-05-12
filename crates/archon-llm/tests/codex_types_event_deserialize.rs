use archon_llm::providers::codex::types::{ResponseOutputItem, ResponseStreamEvent};

fn snapshot_event(event_type: &str) -> serde_json::Value {
    serde_json::json!({
        "type": event_type,
        "response": {"id": "resp_1", "status": "completed", "model": "gpt-5.3-codex"}
    })
}

#[test]
fn deserializes_response_lifecycle_events() {
    for event_type in [
        "response.created",
        "response.in_progress",
        "response.completed",
        "response.failed",
        "response.incomplete",
    ] {
        let event: ResponseStreamEvent =
            serde_json::from_value(snapshot_event(event_type)).expect(event_type);
        assert!(matches!(
            event,
            ResponseStreamEvent::Created { .. }
                | ResponseStreamEvent::InProgress { .. }
                | ResponseStreamEvent::Completed { .. }
                | ResponseStreamEvent::Failed { .. }
                | ResponseStreamEvent::Incomplete { .. }
        ));
    }
}

#[test]
fn deserializes_output_item_events() {
    let value = serde_json::json!({
        "type": "response.output_item.added",
        "output_index": 0,
        "item": {"type": "message", "id": "msg_1", "status": "in_progress", "role": "assistant", "content": []}
    });
    let event: ResponseStreamEvent = serde_json::from_value(value).expect("item added");

    match event {
        ResponseStreamEvent::OutputItemAdded {
            item: ResponseOutputItem::Message { id, .. },
            ..
        } => assert_eq!(id, "msg_1"),
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn deserializes_delta_events() {
    for value in [
        serde_json::json!({"type":"response.output_text.delta","item_id":"i","output_index":0,"content_index":0,"delta":"a"}),
        serde_json::json!({"type":"response.output_text.done","item_id":"i","output_index":0,"content_index":0,"text":"a"}),
        serde_json::json!({"type":"response.reasoning.delta","item_id":"i","output_index":0,"delta":"r"}),
        serde_json::json!({"type":"response.reasoning.done","item_id":"i","output_index":0,"text":"r"}),
        serde_json::json!({"type":"response.reasoning_summary.delta","item_id":"i","output_index":0,"summary_index":0,"delta":"s"}),
        serde_json::json!({"type":"response.reasoning_summary.done","item_id":"i","output_index":0,"summary_index":0,"text":"s"}),
        serde_json::json!({"type":"response.function_call_arguments.delta","item_id":"i","output_index":0,"delta":"{}"}),
        serde_json::json!({"type":"response.function_call_arguments.done","item_id":"i","output_index":0,"arguments":"{}"}),
        serde_json::json!({"type":"response.refusal.delta","item_id":"i","output_index":0,"content_index":0,"delta":"no"}),
    ] {
        let _event: ResponseStreamEvent = serde_json::from_value(value).expect("delta event");
    }
}

#[test]
fn deserializes_content_part_events_and_error() {
    for value in [
        serde_json::json!({"type":"response.content_part.added","item_id":"i","output_index":0,"content_index":0,"part":{"type":"output_text","text":"x"}}),
        serde_json::json!({"type":"response.content_part.done","item_id":"i","output_index":0,"content_index":0,"part":{"type":"output_text","text":"x"}}),
        serde_json::json!({"type":"error","code":"bad","message":"nope","param":null}),
    ] {
        let _event: ResponseStreamEvent = serde_json::from_value(value).expect("event");
    }
}

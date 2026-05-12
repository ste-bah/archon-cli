use archon_llm::providers::codex::translator::process_responses_stream;
use archon_llm::providers::codex::types::ResponseStreamEvent;
use archon_llm::streaming::StreamEvent;

#[test]
fn created_maps_to_message_start() {
    let event = serde_json::from_value::<ResponseStreamEvent>(serde_json::json!({
        "type": "response.created",
        "response": {"id": "resp_1", "model": "gpt-5.3-codex"}
    }))
    .expect("created");

    let events = process_responses_stream(vec![event]);
    assert!(matches!(events[0], Ok(StreamEvent::MessageStart { .. })));
}

#[test]
fn text_delta_uses_tracked_block_index() {
    let events = [
        serde_json::json!({"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"msg_1","status":"in_progress","role":"assistant","content":[]}}),
        serde_json::json!({"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"hi"}),
    ]
    .into_iter()
    .map(|v| serde_json::from_value::<ResponseStreamEvent>(v).expect("event"));

    let events = process_responses_stream(events);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Ok(StreamEvent::TextDelta { text, .. }) if text == "hi"))
    );
}

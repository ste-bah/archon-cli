use archon_llm::provider::LlmRequest;
use archon_llm::providers::codex::translator::{
    messages_to_responses_input, process_responses_stream,
};
use archon_llm::providers::codex::types::{ResponseInputItem, ResponseStreamEvent};
use archon_llm::streaming::StreamEvent;

#[test]
fn request_reasoning_round_trips_through_stream_event() {
    let req = LlmRequest {
        messages: vec![
            serde_json::json!({"role": "assistant", "content": [{"type": "text", "text": "hi"}]}),
        ],
        ..LlmRequest::default().with_reasoning_encrypted(Some("opaque".into()))
    };
    let input = messages_to_responses_input(&req).expect("translate");
    assert!(matches!(
        input[0],
        ResponseInputItem::Reasoning {
            ref encrypted_content,
            ref summary,
        } if encrypted_content == "opaque" && summary.is_empty()
    ));

    let events = process_responses_stream(vec![
        serde_json::from_value::<ResponseStreamEvent>(serde_json::json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {"type": "reasoning", "id": "rs_1", "encrypted_content": "opaque"}
        }))
        .expect("reasoning done"),
        serde_json::from_value::<ResponseStreamEvent>(serde_json::json!({
            "type": "response.completed",
            "response": {"id": "resp_1", "status": "completed"}
        }))
        .expect("completed"),
    ]);

    assert!(events.into_iter().any(|event| {
        matches!(
            event,
            Ok(StreamEvent::ReasoningEncrypted { blob }) if blob == "opaque"
        )
    }));
}

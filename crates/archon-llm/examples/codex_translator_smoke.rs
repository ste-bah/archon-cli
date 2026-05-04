use archon_llm::provider::LlmRequest;
use archon_llm::providers::codex::translator::{
    join_system_prompt, messages_to_responses_input, process_responses_stream,
};
use archon_llm::providers::codex::types::{ResponseInputItem, ResponseStreamEvent};
use archon_llm::streaming::StreamEvent;

fn main() {
    let request = LlmRequest {
        system: vec![serde_json::json!({"type": "text", "text": "system"})],
        messages: vec![
            serde_json::json!({"role": "user", "content": [{"type": "text", "text": "hello"}]}),
            serde_json::json!({"role": "assistant", "content": [{"type": "text", "text": "hi"}]}),
        ],
        tools: vec![serde_json::json!({"name": "lookup", "input_schema": {"type": "object"}})],
        ..LlmRequest::default().with_reasoning_encrypted(Some("blob".into()))
    };

    let input = messages_to_responses_input(&request).expect("messages translate");
    assert!(matches!(input[1], ResponseInputItem::Reasoning { .. }));
    println!("OK: messages translated ({} items)", input.len());

    assert_eq!(
        join_system_prompt(&request.system).as_deref(),
        Some("system")
    );
    println!("OK: system instructions joined");

    let bad_system = vec![serde_json::json!({"role": "system", "content": "bad"})];
    assert!(join_system_prompt(&bad_system).is_none());
    println!("OK: role-content system rejected");

    let events = process_responses_stream(vec![
        serde_json::from_value::<ResponseStreamEvent>(serde_json::json!({
            "type":"response.output_item.done",
            "output_index":0,
            "item":{"type":"reasoning","id":"rs_1","encrypted_content":"blob"}
        }))
        .expect("reasoning"),
        serde_json::from_value::<ResponseStreamEvent>(serde_json::json!({
            "type":"response.completed",
            "response":{"id":"resp_1","status":"completed"}
        }))
        .expect("completed"),
    ]);
    assert!(events.into_iter().any(|event| {
        matches!(event, Ok(StreamEvent::ReasoningEncrypted { blob }) if blob == "blob")
    }));
    println!("OK: reasoning round-tripped");
}

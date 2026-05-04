use archon_llm::provider::LlmRequest;
use archon_llm::providers::codex::translator::messages_to_responses_input;
use archon_llm::providers::codex::types::{ResponseContentBlock, ResponseInputItem};

#[test]
fn user_text_maps_to_input_text_message() {
    let req = LlmRequest {
        messages: vec![
            serde_json::json!({"role": "user", "content": [{"type": "text", "text": "hello"}]}),
        ],
        ..LlmRequest::default()
    };

    let input = messages_to_responses_input(&req).expect("translate");
    match &input[0] {
        ResponseInputItem::Message { role, content } => {
            assert_eq!(role, "user");
            assert!(matches!(content[0], ResponseContentBlock::InputText { .. }));
        }
        other => panic!("unexpected input: {other:?}"),
    }
}

#[test]
fn assistant_text_maps_to_output_text_message() {
    let req = LlmRequest {
        messages: vec![
            serde_json::json!({"role": "assistant", "content": [{"type": "text", "text": "hi"}]}),
        ],
        ..LlmRequest::default()
    };

    let input = messages_to_responses_input(&req).expect("translate");
    match &input[0] {
        ResponseInputItem::Message { role, content } => {
            assert_eq!(role, "assistant");
            assert!(matches!(
                content[0],
                ResponseContentBlock::OutputText { .. }
            ));
        }
        other => panic!("unexpected input: {other:?}"),
    }
}

#[test]
fn reasoning_blob_precedes_assistant_message() {
    let req = LlmRequest {
        messages: vec![
            serde_json::json!({"role": "user", "content": [{"type": "text", "text": "hello"}]}),
            serde_json::json!({"role": "assistant", "content": [{"type": "text", "text": "hi"}]}),
        ],
        ..LlmRequest::default().with_reasoning_encrypted(Some("opaque".into()))
    };

    let input = messages_to_responses_input(&req).expect("translate");
    assert!(matches!(input[1], ResponseInputItem::Reasoning { .. }));
    assert!(matches!(input[2], ResponseInputItem::Message { .. }));
}

use archon_llm::provider::LlmRequest;
use archon_llm::providers::codex::translator::messages_to_responses_input;

#[test]
fn unknown_content_block_is_skipped() {
    let req = LlmRequest {
        messages: vec![
            serde_json::json!({"role": "user", "content": [{"type": "document", "text": "x"}]}),
        ],
        ..LlmRequest::default()
    };

    let input = messages_to_responses_input(&req).expect("translate");
    assert!(input.is_empty());
}

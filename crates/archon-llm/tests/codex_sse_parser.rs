use archon_llm::provider::LlmError;
use archon_llm::providers::codex::sse::parse_codex_sse_frame;
use archon_llm::providers::codex::types::ResponseStreamEvent;

#[test]
fn parser_ignores_comments_and_empty_frames() {
    assert!(parse_codex_sse_frame(": keepalive\n\n").is_empty());
    assert!(parse_codex_sse_frame("\n").is_empty());
}

#[test]
fn parser_reads_json_data_frame() {
    let events = parse_codex_sse_frame(
        r#"event: response.created
data: {"type":"response.created","response":{"id":"r1","status":"in_progress","model":"gpt-5.3-codex"}}"#,
    );

    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0],
        Ok(ResponseStreamEvent::Created { response }) if response.id == "r1"
    ));
}

#[test]
fn parser_concatenates_multiline_data() {
    let events = parse_codex_sse_frame(
        "data: {\"type\":\"response.created\",\ndata: \"response\":{\"id\":\"r2\"}}\n",
    );

    assert!(matches!(
        &events[0],
        Ok(ResponseStreamEvent::Created { response }) if response.id == "r2"
    ));
}

#[test]
fn parser_treats_done_as_end_marker() {
    assert!(parse_codex_sse_frame("data: [DONE]\n").is_empty());
}

#[test]
fn parser_skips_malformed_json_without_error_event() {
    assert!(parse_codex_sse_frame("data: {not-json}\n").is_empty());
}

#[test]
fn parser_preserves_unicode_delta() {
    let events = parse_codex_sse_frame(
        r#"data: {"type":"response.output_text.delta","item_id":"i1","output_index":0,"content_index":0,"delta":"hello 世界"}"#,
    );

    assert!(matches!(
        &events[0],
        Ok(ResponseStreamEvent::OutputTextDelta { delta, .. }) if delta == "hello 世界"
    ));
}

#[test]
fn parser_type_is_result_based_for_http_errors() {
    let _: Vec<Result<ResponseStreamEvent, LlmError>> = parse_codex_sse_frame("data: [DONE]");
}

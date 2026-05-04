use archon_llm::providers::codex::sse::parse_codex_sse_frame;
use archon_llm::providers::codex::translator::StreamAccumulator;
use archon_llm::streaming::StreamEvent;

#[test]
fn captured_sse_fixture_parses_and_translates() {
    let sse = std::fs::read_to_string("tests/fixtures/codex/captured/responses_sse_simple.txt")
        .unwrap_or_default();
    let mut accumulator = StreamAccumulator::default();
    let mut translated = Vec::new();

    for frame in sse.split("\n\n") {
        for parsed in parse_codex_sse_frame(frame) {
            let event =
                parsed.unwrap_or(archon_llm::providers::codex::types::ResponseStreamEvent::Unknown);
            for item in accumulator.process(event) {
                translated.push(item.unwrap_or(StreamEvent::Ping));
            }
        }
    }

    assert!(translated.iter().any(
        |event| matches!(event, StreamEvent::TextDelta { text, .. } if text == "ARCHON_SMOKE_OK_42")
    ));
    assert!(
        translated
            .iter()
            .any(|event| matches!(event, StreamEvent::MessageStop))
    );
}

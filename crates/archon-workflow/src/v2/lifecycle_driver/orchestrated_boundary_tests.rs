use super::*;

#[test]
fn workflow_v3_transcript_tail_is_independently_bounded() {
    let mut transcript = Vec::new();
    for turn in 0..(TRANSCRIPT_TAIL + 5) {
        push_bounded_orchestrator_turn(
            &mut transcript,
            serde_json::json!({"turn": turn, "outcome": "invalid_action"}),
        );
    }

    assert_eq!(transcript.len(), TRANSCRIPT_TAIL);
    assert_eq!(transcript.first().unwrap()["turn"], 5);
    assert_eq!(transcript.last().unwrap()["turn"], TRANSCRIPT_TAIL + 4);
}

#[test]
fn workflow_v3_uses_its_own_ledger_and_not_agent_compaction_segments() {
    let source = include_str!("orchestrated.rs");

    assert!(source.contains("OrchestrationLedger"));
    assert!(source.contains("TRANSCRIPT_TAIL"));
    assert!(!source.contains("CompactionSegment"));
    assert!(!source.contains("SessionStore"));
}

use archon_reasoning_quality::store::ReasoningQualityStore;
use archon_reasoning_quality::{
    DeterministicExtractor, EvidenceKind, EvidenceRef, ExtractorConfig, ReasoningEventKind,
    ReasoningTurnInput, build_superseding_source_events,
};

#[test]
fn simulated_session_claim_then_source_contradiction_is_append_only() {
    let temp = tempfile::tempdir().unwrap();
    let store = ReasoningQualityStore::open(temp.path()).unwrap();
    let extractor = DeterministicExtractor::new(ExtractorConfig::default());

    let first = extractor.extract_turn(&ReasoningTurnInput {
        session_id: "sim-session".into(),
        turn_number: 1,
        assistant_text: "The codebase has src/not_real.rs defining the parser function.".into(),
        ..ReasoningTurnInput::default()
    });
    assert!(
        first
            .iter()
            .any(|event| event.event_kind == ReasoningEventKind::ClaimBeforeSourceRead)
    );
    store.append_events(&first).unwrap();

    let evidence = EvidenceRef {
        evidence_id: "read:not-real".into(),
        kind: EvidenceKind::FileRead,
        entity_key: Some("src/not_real.rs".into()),
        redacted_excerpt: Some("No such file or directory".into()),
        ..EvidenceRef::default()
    };
    let prior = store.events_for_session("sim-session").unwrap();
    let superseding = build_superseding_source_events(&prior, &[evidence]);
    assert_eq!(superseding.len(), 1);
    assert_eq!(
        superseding[0].event_kind,
        ReasoningEventKind::ClaimContradictedBySource
    );
    store.append_events(&superseding).unwrap();

    let all = store.events_for_session("sim-session").unwrap();
    assert!(all.len() >= 2);
    assert!(
        all.iter()
            .any(|event| event.event_kind == ReasoningEventKind::ClaimBeforeSourceRead)
    );
    assert!(
        all.iter()
            .any(|event| event.event_kind == ReasoningEventKind::ClaimContradictedBySource)
    );
}

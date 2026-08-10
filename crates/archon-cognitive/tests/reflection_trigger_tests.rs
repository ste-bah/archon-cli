use archon_cognitive::reflection_trigger::evaluate;
use archon_cognitive::{
    CandidateActionKind, ReflectionTrigger, ReflectionWriter, SituationKind, TriggeredReflectInput,
    TriggeredReflection, TurnSignals,
};
use cozo::{DbInstance, ScriptMutability};

fn signals() -> TurnSignals {
    TurnSignals::new(SituationKind::CodeChange)
}

fn triggered_input(trigger: ReflectionTrigger, refs: Vec<String>) -> TriggeredReflectInput {
    TriggeredReflectInput {
        decision_id: "decision-1".into(),
        session_id: "session-1".into(),
        turn_number: 3,
        situation_kind: SituationKind::CodeChange,
        goal_action: Some(CandidateActionKind::RunTests),
        observed_action: Some(CandidateActionKind::InspectFiles),
        trigger: TriggeredReflection {
            trigger,
            confidence: 0.9,
        },
        evidence_refs: refs,
    }
}

#[test]
fn an_ordinary_turn_triggers_nothing() {
    assert!(evaluate(&signals()).is_none());
}

#[test]
fn a_single_tool_failure_is_not_repeated_failure() {
    let mut signals = signals();
    signals.tool_failures = 1;

    assert!(evaluate(&signals).is_none());
}

#[test]
fn two_tool_failures_trigger_a_reflection() {
    let mut signals = signals();
    signals.tool_failures = 2;

    let triggered = evaluate(&signals).expect("trigger");

    assert_eq!(triggered.trigger, ReflectionTrigger::RepeatedToolFailure);
    assert!((0.0..=1.0).contains(&triggered.confidence));
}

/// A plain action mismatch is 0.5 and must not fire on its own: disagreeing
/// with a shadow planner is ordinary, and reflecting on it every time would
/// turn the reflection relation into a turn log.
#[test]
fn a_bare_action_mismatch_does_not_trigger_high_surprise() {
    let mut signals = signals();
    signals.shadow_surprise = Some(0.5);

    assert!(evaluate(&signals).is_none());

    signals.shadow_surprise = Some(0.8);
    assert_eq!(
        evaluate(&signals).expect("trigger").trigger,
        ReflectionTrigger::HighSurprise
    );
}

#[test]
fn a_correction_outranks_the_other_triggers() {
    let mut signals = signals();
    signals.tool_failures = 5;
    signals.shadow_surprise = Some(1.0);
    signals.correction_confidence = Some(0.95);

    let triggered = evaluate(&signals).expect("trigger");

    assert_eq!(
        triggered.trigger,
        ReflectionTrigger::HighConfidenceCorrection
    );
    assert_eq!(triggered.confidence, 0.95);
}

#[test]
fn a_low_confidence_correction_does_not_trigger() {
    let mut signals = signals();
    signals.correction_confidence = Some(0.4);

    assert!(evaluate(&signals).is_none());
}

/// A NaN comparison is false in every direction, so an unchecked NaN would
/// silently disable the trigger rather than announce itself.
#[test]
fn out_of_range_scores_are_rejected_rather_than_clamped() {
    let mut signals = signals();
    signals.shadow_surprise = Some(f32::NAN);
    assert!(evaluate(&signals).is_none());

    signals.shadow_surprise = Some(7.0);
    assert!(evaluate(&signals).is_none());

    signals.shadow_surprise = None;
    signals.correction_confidence = Some(f32::INFINITY);
    assert!(evaluate(&signals).is_none());
}

#[test]
fn a_triggered_reflection_persists_goal_mismatch_adjustment_and_evidence() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let writer = ReflectionWriter::new(&db, dir.path(), true).unwrap();

    let outcome = writer
        .reflect_triggered(triggered_input(
            ReflectionTrigger::RepeatedToolFailure,
            vec![
                "shadow_decision:abc".into(),
                "cognitive_decision:def".into(),
            ],
        ))
        .unwrap();

    let record = outcome.reflection.expect("reflection");
    assert!(outcome.degraded.is_empty(), "{:?}", outcome.degraded);
    assert!(record.attempted.starts_with("goal:code_change:run_tests"));
    assert!(record.failed.contains("repeated_tool_failure"));
    assert!(record.lesson.contains("re-plan"));
    // Nothing was verified on this path, so nothing may claim to have worked.
    assert!(record.worked.is_empty());

    let evidence = db
        .run_script(
            "?[trigger, confidence, evidence_refs_json] := *cognitive_reflection_evidence{reflection_id, trigger, confidence, evidence_refs_json}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(evidence.rows.len(), 1);
    assert_eq!(evidence.rows[0][0].get_str(), Some("repeated_tool_failure"));
    assert!((evidence.rows[0][1].get_float().unwrap() - 0.9).abs() < 1e-6);
    assert!(
        evidence.rows[0][2]
            .get_str()
            .unwrap()
            .contains("shadow_decision:abc")
    );
}

/// The one guarantee issue #81 is explicit about: no raw chain-of-thought.
///
/// `TriggeredReflectInput` carries only ids and enums, so the turn's text
/// cannot reach the writer through any field except `evidence_refs` — and that
/// field rejects anything that is not an identifier.
#[test]
fn raw_chain_of_thought_cannot_reach_a_persisted_reflection() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let writer = ReflectionWriter::new(&db, dir.path(), true).unwrap();
    let smuggled = "Let me think about this. The user said hyperspecific_SECRET_word so I will";

    let outcome = writer
        .reflect_triggered(triggered_input(
            ReflectionTrigger::HighSurprise,
            vec![
                smuggled.to_string(),
                "shadow_decision:kept".into(),
                "x".repeat(500),
            ],
        ))
        .unwrap();

    let record = outcome.reflection.expect("reflection");
    let serialized = serde_json::to_string(&record).unwrap();
    assert!(
        !serialized.contains("hyperspecific_SECRET_word"),
        "{serialized}"
    );

    // Rejections are reported as a count; echoing the content back would put
    // the text straight into the audit this filter exists to keep clean.
    assert_eq!(
        outcome.degraded,
        vec!["evidence_refs_rejected:2".to_string()]
    );

    let stored = db
        .run_script(
            "?[evidence_refs_json] := *cognitive_reflection_evidence{reflection_id, evidence_refs_json}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    let refs = stored.rows[0][0].get_str().unwrap();
    assert_eq!(refs, "[\"shadow_decision:kept\"]");

    let ledger = std::fs::read_to_string(dir.path().join("cognitive-reflections.jsonl")).unwrap();
    assert!(!ledger.contains("hyperspecific_SECRET_word"), "{ledger}");
}

#[test]
fn a_disabled_writer_records_nothing() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let writer = ReflectionWriter::new(&db, dir.path(), false).unwrap();

    let outcome = writer
        .reflect_triggered(triggered_input(ReflectionTrigger::HighSurprise, Vec::new()))
        .unwrap();

    assert!(outcome.reflection.is_none());
    assert!(!dir.path().join("cognitive-reflections.jsonl").exists());
}

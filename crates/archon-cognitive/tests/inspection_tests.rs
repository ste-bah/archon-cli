use archon_cognitive::self_model::{FactKind, SelfModelFact, SelfModelStore};
use archon_cognitive::{
    CognitiveInspection, CognitiveTick, DecisionRecord, DecisionStore, OutcomeSummary,
    ReflectInput, ReflectionWriter, SituationKind, VerificationVerdict,
};
use chrono::Utc;
use cozo::{DbInstance, ScriptMutability};

fn decision(id: &str, session: &str, turn: u64) -> DecisionRecord {
    DecisionRecord {
        decision_id: id.into(),
        situation_id: "situation-1".into(),
        session_id: session.into(),
        turn_number: turn,
        selected_candidate_id: "candidate-1".into(),
        rejected_alternatives: Vec::new(),
        heuristic_scores: Vec::new(),
        policy_verdict: Some("allowed".into()),
        verification_contract: Some("tests required".into()),
        user_visible_summary: "code_change -> run_tests".into(),
        created_at: Utc::now(),
    }
}

#[test]
fn inspection_status_summarizes_safe_cognitive_state() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let decision = decision("decision-1", "session-1", 7);
    DecisionStore::new(&db, dir.path().join("decisions.jsonl"))
        .unwrap()
        .record(&decision)
        .unwrap();
    ReflectionWriter::new(&db, dir.path(), true)
        .unwrap()
        .reflect(ReflectInput {
            decision: decision.clone(),
            situation_kind: SituationKind::CodeChange,
            verification: VerificationVerdict::Failed {
                reason: "test failed".into(),
            },
            outcome: OutcomeSummary::Failure,
            user_corrected: false,
        })
        .unwrap();
    SelfModelStore::new(&db)
        .unwrap()
        .write_fact(&SelfModelFact::new(
            "code_change",
            FactKind::CautionRule,
            "require tests before completion",
            0.8,
            3,
        ))
        .unwrap();
    CognitiveTick::new(&db, None).unwrap().tick().unwrap();

    let status = CognitiveInspection::new(&db, dir.path())
        .unwrap()
        .status()
        .unwrap();

    assert_eq!(status.executive_decision_count, 1);
    assert_eq!(status.reflection_count, 1);
    assert_eq!(status.self_model_fact_count, 1);
    assert!(status.latest_tick.is_some());
    assert_eq!(status.recent_decisions[0].session_id, "session-1");
    assert!(status.recent_reflections[0].lesson.contains("code_change"));
    assert_eq!(status.self_model.fact_count, 1);
}

#[test]
fn inspection_lists_session_decisions_and_reflections() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = DecisionStore::new(&db, dir.path().join("decisions.jsonl")).unwrap();
    store
        .record(&decision("decision-1", "session-a", 1))
        .unwrap();
    store
        .record(&decision("decision-2", "session-b", 2))
        .unwrap();

    let inspection = CognitiveInspection::new(&db, dir.path()).unwrap();
    let session = inspection.decisions_for_session("session-a", 10).unwrap();
    let inspected = inspection.inspect_decision("decision-2").unwrap();

    assert_eq!(session.len(), 1);
    assert_eq!(session[0].decision_id, "decision-1");
    assert_eq!(inspected.unwrap().session_id, "session-b");
}

#[test]
fn pending_proposals_include_latest_apply_result_without_raw_text() {
    let db = DbInstance::new("mem", "", "").unwrap();
    archon_cognitive::ensure_cognitive_schema(&db).unwrap();
    db.run_script(
        "?[proposal_id, reflection_ids_json, manifest_kind, risk_level, evidence_count, lesson_tag, domain, diff_summary, rollback_plan, created_at] <- [[\
         'proposal-1', '[\"r1\"]', 'tool_preference', 'low', 3, 'prefer exact evidence', 'code_change', 'manifest diff', 'remove entry', '2026-05-25T00:00:00Z']] \
         :put governed_proposals { proposal_id => reflection_ids_json, manifest_kind, risk_level, evidence_count, lesson_tag, domain, diff_summary, rollback_plan, created_at }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    db.run_script(
        "?[apply_id, proposal_id, result_kind, reason, canary_outcome_ref, rollback_ref, created_at] <- [[\
         'apply-1', 'proposal-1', 'auto_applied', 'ok', 'canary-1', 'rollback-1', '2026-05-25T00:01:00Z']] \
         :put autonomous_apply_results { apply_id => proposal_id, result_kind, reason, canary_outcome_ref, rollback_ref, created_at }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();

    let proposals = CognitiveInspection::new(&db, tempfile::tempdir().unwrap().path())
        .unwrap()
        .pending_proposals(5)
        .unwrap();

    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0].latest_result.as_deref(), Some("auto_applied"));
    assert_eq!(proposals[0].lesson_tag, "prefer exact evidence");
}

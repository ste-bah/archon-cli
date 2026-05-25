use archon_cognitive::self_model::{
    ConfidenceCalibration, DomainTrust, MemoryContext, SelfModelProfile,
};
use archon_cognitive::{
    CandidatePlanner, ClassifyInput, CognitiveSurface, DecisionStore, SituationClassifier,
};
use chrono::Utc;
use cozo::DbInstance;

fn situation() -> archon_cognitive::Situation {
    SituationClassifier.classify(ClassifyInput {
        user_text: "fix the failing rust test",
        session_id: "session-1",
        turn_number: 7,
        surface: CognitiveSurface::Cli,
    })
}

fn profile() -> SelfModelProfile {
    SelfModelProfile {
        domain_trust: vec![DomainTrust {
            domain: "coding".into(),
            trust_score: 0.8,
            evidence_count: 8,
            last_correction_at: None,
            failure_cluster_ids: Vec::new(),
        }],
        active_failure_clusters: Vec::new(),
        confidence_calibration: ConfidenceCalibration::default(),
        caution_rules: Vec::new(),
        generated_at: Utc::now(),
    }
}

#[test]
fn decision_store_records_cozo_and_jsonl() {
    let db = DbInstance::new("mem", "", "").expect("mem db");
    let dir = tempfile::tempdir().unwrap();
    let store = DecisionStore::new(&db, dir.path().join("cognitive-decisions.jsonl")).unwrap();
    let situation = situation();
    let candidates = CandidatePlanner::new(&db, 5)
        .unwrap()
        .generate(&situation, &profile(), &MemoryContext::default())
        .unwrap();
    let decision = DecisionStore::decision_from_candidates(&situation, &candidates).unwrap();

    let id = store.record(&decision).unwrap();
    assert_eq!(store.get(&id).unwrap().unwrap().decision_id, id);
    assert_eq!(store.list_for_session("session-1", 10).unwrap().len(), 1);
    assert_eq!(store.replay_ledger(None).unwrap().len(), 1);
}

#[test]
fn decision_store_updates_policy_and_verification_fields() {
    let db = DbInstance::new("mem", "", "").expect("mem db");
    let dir = tempfile::tempdir().unwrap();
    let store = DecisionStore::new(&db, dir.path().join("decisions.jsonl")).unwrap();
    let situation = situation();
    let candidates = CandidatePlanner::new(&db, 5)
        .unwrap()
        .generate(&situation, &profile(), &MemoryContext::default())
        .unwrap();
    let decision = DecisionStore::decision_from_candidates(&situation, &candidates).unwrap();
    let id = store.record(&decision).unwrap();

    store.update_policy_verdict(&id, "allow").unwrap();
    store
        .update_verification_contract(&id, "run focused test")
        .unwrap();
    let updated = store.get(&id).unwrap().unwrap();

    assert_eq!(updated.policy_verdict.as_deref(), Some("allow"));
    assert_eq!(
        updated.verification_contract.as_deref(),
        Some("run focused test")
    );
}

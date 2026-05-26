use archon_cognitive::self_model::{
    ConfidenceCalibration, DomainTrust, MemoryContext, SelfModelProfile,
};
use archon_cognitive::{
    CandidateActionKind, CandidatePlanner, ClassifyInput, CognitiveSurface, SituationClassifier,
};
use chrono::Utc;
use cozo::{DbInstance, ScriptMutability};

fn classify(text: &str) -> archon_cognitive::Situation {
    SituationClassifier.classify(ClassifyInput {
        user_text: text,
        session_id: "session-1",
        turn_number: 1,
        surface: CognitiveSurface::Cli,
    })
}

fn profile(domain: &str, trust_score: f32) -> SelfModelProfile {
    SelfModelProfile {
        domain_trust: vec![DomainTrust {
            domain: domain.to_string(),
            trust_score,
            evidence_count: 10,
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
fn greeting_yields_single_direct_answer() {
    let planner = CandidatePlanner::without_store(5);
    let candidates = planner
        .generate(
            &classify("hello"),
            &profile("general", 0.5),
            &MemoryContext::default(),
        )
        .unwrap();

    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0].action_kind,
        CandidateActionKind::AnswerDirectly
    );
}

#[test]
fn code_change_generates_ranked_persisted_candidates() {
    let db = DbInstance::new("mem", "", "").expect("mem db");
    let planner = CandidatePlanner::new(&db, 5).unwrap();
    let candidates = planner
        .generate(
            &classify("fix the failing rust test"),
            &profile("coding", 0.8),
            &MemoryContext::default(),
        )
        .unwrap();

    assert!(candidates.len() >= 2);
    assert_eq!(candidates[0].action_kind, CandidateActionKind::InspectFiles);

    let rows = db
        .run_script(
            "?[candidate_id] := *cognitive_action_candidates{candidate_id}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(rows.rows.len(), candidates.len());
}

#[test]
fn low_domain_trust_promotes_clarification() {
    let planner = CandidatePlanner::without_store(5);
    let candidates = planner
        .generate(
            &classify("fix the failing rust test"),
            &profile("coding", 0.1),
            &MemoryContext::default(),
        )
        .unwrap();

    assert_eq!(
        candidates[0].action_kind,
        CandidateActionKind::AskClarification
    );
    assert_eq!(
        candidates[1].action_kind,
        CandidateActionKind::DeferOrDecline
    );
}

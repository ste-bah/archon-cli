use archon_cognitive::{
    ApplyResult, BehaviourManifestKind, GovernedAutonomousApply, OutcomeSummary, Proposal,
    ReflectionRecord, RiskLevel, SituationKind,
};
use archon_policy::CognitivePolicy;
use chrono::Utc;
use cozo::{DbInstance, ScriptMutability};

fn policy(allow_apply: bool) -> CognitivePolicy {
    CognitivePolicy {
        enabled: true,
        allow_autonomous_low_risk_apply: allow_apply,
        max_autonomous_risk: "Low".into(),
        ..CognitivePolicy::default()
    }
}

fn reflection(lesson: &str, kind: SituationKind) -> ReflectionRecord {
    ReflectionRecord {
        reflection_id: "reflection-main".into(),
        session_id: "session-1".into(),
        turn_number: 1,
        decision_id: "decision-1".into(),
        situation_kind: kind,
        attempted: "compact attempted".into(),
        worked: String::new(),
        failed: "compact failed".into(),
        lesson: lesson.into(),
        should_propose: true,
        proposed_rule_id: None,
        outcome: OutcomeSummary::Failure,
        created_at: Utc::now(),
    }
}

fn proposal(kind: BehaviourManifestKind, evidence_count: u64) -> Proposal {
    Proposal {
        proposal_id: "proposal-1".into(),
        reflection_ids: vec!["r1".into(), "r2".into(), "r3".into()],
        manifest_kind: kind,
        risk_level: match kind {
            BehaviourManifestKind::ToolPreference
            | BehaviourManifestKind::MemoryRecallStrategy
            | BehaviourManifestKind::AnswerFormatting => RiskLevel::Low,
            BehaviourManifestKind::PolicyMutation
            | BehaviourManifestKind::PromptMutation
            | BehaviourManifestKind::NetworkConfig
            | BehaviourManifestKind::BlockingGate => RiskLevel::Critical,
            _ => RiskLevel::Medium,
        },
        evidence_count,
        lesson_tag: "prefer tool use after repeated failure".into(),
        domain: "general".into(),
        diff_summary: format!("manifest={}", kind.as_str()),
        rollback_plan: Some("remove manifest entry".into()),
        created_at: Utc::now(),
    }
}

fn insert_reflection(db: &DbInstance, id: &str, lesson: &str, kind: &str) {
    let script = format!(
        "?[reflection_id, session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at] <- \
         [['{id}', 'session-1', 1, 'decision-{id}', '{kind}', 'attempted', '', 'failed', 'failure', '{lesson}', true, '', '{}']]
         :put cognitive_reflections {{ reflection_id => session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at }}",
        Utc::now().to_rfc3339()
    );
    db.run_script(&script, Default::default(), ScriptMutability::Mutable)
        .unwrap();
}

fn result_count(db: &DbInstance, relation: &str, key: &str) -> usize {
    let rows = db
        .run_script(
            format!("?[id] := *{relation}{{{key}: id}}").as_str(),
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    rows.rows.len()
}

#[test]
fn propose_counts_reflection_evidence_and_stores_proposal() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let apply = GovernedAutonomousApply::new(&db, Some(policy(true))).unwrap();
    for id in ["r1", "r2", "r3"] {
        insert_reflection(&db, id, "answer format should be concise", "greeting");
    }

    let proposal = apply
        .propose(&reflection(
            "answer format should be concise",
            SituationKind::Greeting,
        ))
        .unwrap();

    assert_eq!(proposal.evidence_count, 3);
    assert_eq!(
        proposal.manifest_kind,
        BehaviourManifestKind::AnswerFormatting
    );
    assert_eq!(result_count(&db, "governed_proposals", "proposal_id"), 1);
}

#[test]
fn low_risk_with_evidence_and_rollback_auto_applies() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let apply = GovernedAutonomousApply::new(&db, Some(policy(true))).unwrap();

    let result = apply
        .apply(&proposal(BehaviourManifestKind::ToolPreference, 3))
        .unwrap();

    assert!(matches!(result, ApplyResult::AutoApplied { .. }));
    assert_eq!(result_count(&db, "canary_outcomes", "canary_id"), 1);
    assert_eq!(result_count(&db, "autonomous_apply_results", "apply_id"), 1);
}

#[test]
fn disabled_policy_returns_pending_review() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let apply = GovernedAutonomousApply::new(&db, Some(policy(false))).unwrap();

    let result = apply
        .apply(&proposal(BehaviourManifestKind::ToolPreference, 3))
        .unwrap();

    assert!(matches!(result, ApplyResult::PendingReview { .. }));
}

#[test]
fn must_deny_categories_never_auto_apply() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let apply = GovernedAutonomousApply::new(&db, Some(policy(true))).unwrap();

    for kind in [
        BehaviourManifestKind::PromptMutation,
        BehaviourManifestKind::PolicyMutation,
        BehaviourManifestKind::NetworkConfig,
        BehaviourManifestKind::BlockingGate,
    ] {
        let result = apply.apply(&proposal(kind, 3)).unwrap();
        assert!(matches!(result, ApplyResult::Denied { .. }));
    }
}

#[test]
fn missing_rollback_is_denied() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let apply = GovernedAutonomousApply::new(&db, Some(policy(true))).unwrap();
    let mut proposal = proposal(BehaviourManifestKind::ToolPreference, 3);
    proposal.rollback_plan = None;

    let result = apply.apply(&proposal).unwrap();

    assert!(matches!(result, ApplyResult::Denied { .. }));
}

#[test]
fn rollback_records_result() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let apply = GovernedAutonomousApply::new(&db, Some(policy(true))).unwrap();

    let result = apply
        .rollback("proposal-1", "manual safety rollback")
        .unwrap();

    assert!(matches!(result, ApplyResult::RolledBack { .. }));
    assert_eq!(result_count(&db, "autonomous_apply_results", "apply_id"), 1);
}

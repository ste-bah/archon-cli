use archon_cognitive::{
    Candidate, CandidateActionKind, PolicyGate, ProposalCheck, RiskLevel, ScoreSource,
};
use archon_policy::CognitivePolicy;
use chrono::Utc;

fn policy(enabled: bool, allow_apply: bool, max_risk: &str) -> CognitivePolicy {
    CognitivePolicy {
        enabled,
        allow_autonomous_low_risk_apply: allow_apply,
        max_autonomous_risk: max_risk.into(),
        ..CognitivePolicy::default()
    }
}

fn candidate(kind: CandidateActionKind, risk: RiskLevel, evidence: &str) -> Candidate {
    Candidate {
        id: format!("candidate-{}", kind.as_str()),
        situation_id: "situation-1".into(),
        action_kind: kind,
        tool_name: None,
        expected_evidence: evidence.into(),
        expected_user_output: "summary".into(),
        risk_class: risk,
        rollback_path: None,
        heuristic_score: 0.5,
        score_source: ScoreSource::Heuristic,
        created_at: Utc::now(),
    }
}

fn proposal(paths: &[&str], risk: RiskLevel) -> ProposalCheck {
    ProposalCheck {
        proposal_id: "proposal-1".into(),
        touched_paths: paths.iter().map(|path| path.to_string()).collect(),
        risk_level: risk,
        evidence_count: 3,
        recent_incidents: 0,
        rollback_available: true,
    }
}

#[test]
fn disabled_policy_allows_regular_candidates() {
    let gate = PolicyGate::new(Some(policy(false, false, "Low")));
    let (allowed, denied) = gate.filter(vec![candidate(
        CandidateActionKind::RunSafeShellProbe,
        RiskLevel::Medium,
        "read-only command output",
    )]);

    assert_eq!(allowed.len(), 1);
    assert!(denied.is_empty());
}

#[test]
fn missing_policy_fails_closed_to_direct_or_clarification() {
    let gate = PolicyGate::new(None);
    let (allowed, denied) = gate.filter(vec![
        candidate(
            CandidateActionKind::AnswerDirectly,
            RiskLevel::Low,
            "context",
        ),
        candidate(
            CandidateActionKind::InspectFiles,
            RiskLevel::Low,
            "file contents",
        ),
    ]);

    assert_eq!(allowed.len(), 1);
    assert_eq!(denied[0].rule_name, "policy_unavailable");
}

#[test]
fn enabled_policy_denies_forbidden_config_touches_and_high_risk() {
    let gate = PolicyGate::new(Some(policy(true, false, "Medium")));
    let (_, denied) = gate.filter(vec![
        candidate(
            CandidateActionKind::InspectFiles,
            RiskLevel::Low,
            "prompt config",
        ),
        candidate(
            CandidateActionKind::InspectFiles,
            RiskLevel::Critical,
            "file",
        ),
    ]);
    let rules = denied
        .iter()
        .map(|item| item.rule_name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        rules,
        ["prompt_mutation_forbidden", "high_risk_requires_human"]
    );
}

#[test]
fn autonomous_apply_respects_policy_risk_ceiling() {
    let gate = PolicyGate::new(Some(policy(true, true, "Low")));

    assert!(gate.allow_autonomous_apply(RiskLevel::Low));
    assert!(!gate.allow_autonomous_apply(RiskLevel::Medium));
    assert!(!gate.allow_autonomous_apply(RiskLevel::Critical));
}

#[test]
fn proposal_denial_prefers_hard_config_blocks() {
    let gate = PolicyGate::new(Some(policy(true, true, "Medium")));
    let denial = gate
        .deny_proposal(&proposal(&[".archon/policy.toml"], RiskLevel::Low))
        .unwrap();

    assert_eq!(denial.policy_rule, "policy_mutation_forbidden");
}

#[test]
fn proposal_denies_insufficient_evidence_incidents_and_missing_rollback() {
    let gate = PolicyGate::new(Some(policy(true, true, "Medium")));
    let mut check = proposal(&["src/lib.rs"], RiskLevel::Low);
    check.evidence_count = 1;
    assert_eq!(
        gate.deny_proposal(&check).unwrap().policy_rule,
        "insufficient_evidence"
    );

    check.evidence_count = 3;
    check.recent_incidents = 1;
    assert_eq!(
        gate.deny_proposal(&check).unwrap().policy_rule,
        "recent_incident_threshold_exceeded"
    );

    check.recent_incidents = 0;
    check.rollback_available = false;
    assert_eq!(
        gate.deny_proposal(&check).unwrap().policy_rule,
        "rollback_unavailable"
    );
}

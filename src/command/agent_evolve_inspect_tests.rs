use super::*;

fn test_db() -> DbInstance {
    let path = format!("/tmp/test-agent-evolve-inspect-{}.db", uuid::Uuid::new_v4());
    let db = DbInstance::new("sqlite", &path, "").unwrap();
    archon_learning::schema::ensure_learning_schema(&db).unwrap();
    db
}

#[test]
fn inspect_summarizes_proposal_evidence_and_shadow() {
    let db = test_db();
    archon_learning::agent_evolution_ledger::insert_agent_performance_ledger_record(
        &db,
        &archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord::new(
            "ledger-1",
            "reviewer",
            "failed",
            "2026-05-08T12:00:00Z",
        )
        .with_model_provider("claude-sonnet-4-6", "anthropic")
        .with_provider_incident("provider-event-1"),
    )
    .unwrap();
    archon_learning::permission_runtime_events::insert_permission_runtime_event(
        &db,
        &archon_learning::permission_runtime_events::PermissionRuntimeEventRecord::new(
            "permission-1",
            "Bash",
            "ask",
            "denied",
            "2026-05-08T12:01:00Z",
        )
        .with_policy_context(
            Some("permission_rule_denied".to_string()),
            Some("deny_shell".to_string()),
            Some("docker".to_string()),
        ),
    )
    .unwrap();
    archon_learning::agent_evolution_proposals::insert_agent_evolution_proposal(
        &db,
        &archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord::new(
            "prop-1",
            "reviewer",
            "agentv-1",
            "agentv-2",
            "tool_access_profile",
            "2026-05-08T12:02:00Z",
        )
        .with_risk("high", "manual_review_required")
        .with_expected_impact("Review repeated denied shell use.")
        .with_evidence("ledger-1")
        .with_evidence("permission-1")
        .with_permission_impact(),
    )
    .unwrap();
    archon_learning::agent_shadow_evaluations::insert_agent_shadow_evaluation(
        &db,
        &archon_learning::agent_shadow_evaluations::AgentShadowEvaluationRecord::new(
            "shadow-1",
            "prop-1",
            "reviewer",
            "needs_review",
            "2026-05-08T12:03:00Z",
        )
        .with_scores(0.4, 0.7)
        .with_counts(1, 3),
    )
    .unwrap();

    let inspection = AgentEvolutionInspection::load(&db, "prop-1").unwrap();

    assert_eq!(inspection.proposal.current_version, "agentv-1");
    assert_eq!(inspection.proposal.proposed_version, "agentv-2");
    assert_eq!(
        inspection.compatibility.anthropic_spoof_status,
        "unaffected"
    );
    assert!(inspection.compatibility.permissions_affected);
    assert!(inspection.compatibility.manual_review_required);
    assert_eq!(inspection.evidence.len(), 2);
    assert_eq!(inspection.evidence[0].source, "agent_performance_ledger");
    assert_eq!(inspection.evidence[1].source, "permission_runtime_events");
    assert_eq!(inspection.shadow_evaluations[0].verdict, "needs_review");
}

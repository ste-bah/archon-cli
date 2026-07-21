use super::*;

fn test_db() -> std::sync::Arc<DbInstance> {
    crate::command::test_support::registered_learning_test_db("test-agent-evolve-history")
}

#[test]
fn history_and_status_summarize_agent_state() {
    let db = test_db();
    archon_learning::agent_profile_versions::insert_agent_profile_version(
        &db,
        &archon_learning::agent_profile_versions::AgentProfileVersionRecord::new(
            "profile-1",
            "reviewer",
            1,
            "baseline",
            "2026-05-08T12:00:00Z",
        ),
    )
    .unwrap();
    archon_learning::agent_profile_versions::insert_agent_profile_version(
        &db,
        &archon_learning::agent_profile_versions::AgentProfileVersionRecord::new(
            "profile-2",
            "reviewer",
            2,
            "proposal",
            "2026-05-08T12:01:00Z",
        )
        .with_parent("profile-1")
        .with_proposal("prop-1")
        .mark_active(),
    )
    .unwrap();
    archon_learning::agent_evolution_proposals::insert_agent_evolution_proposal(
        &db,
        &archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord::new(
            "prop-1",
            "reviewer",
            "profile-1",
            "profile-2",
            "tool_access_profile",
            "2026-05-08T12:02:00Z",
        )
        .with_risk("high", "manual_review_required")
        .with_permission_impact(),
    )
    .unwrap();
    archon_learning::agent_evolution_ledger::insert_agent_performance_ledger_record(
        &db,
        &archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord::new(
            "ledger-1",
            "reviewer",
            "failed",
            "2026-05-08T12:03:00Z",
        )
        .with_provider_incident("provider-event-1"),
    )
    .unwrap();
    archon_learning::agent_shadow_evaluations::insert_agent_shadow_evaluation(
        &db,
        &archon_learning::agent_shadow_evaluations::AgentShadowEvaluationRecord::new(
            "shadow-1",
            "prop-1",
            "reviewer",
            "needs_review",
            "2026-05-08T12:04:00Z",
        )
        .with_scores(0.4, 0.8),
    )
    .unwrap();

    let history = AgentHistory::load(&db, "reviewer").unwrap();
    let status = AgentStatus::load(&db, "reviewer").unwrap();

    assert_eq!(history.versions.len(), 2);
    assert_eq!(history.versions[0].version_id, "profile-2");
    assert!(history.versions[0].is_active);
    assert_eq!(status.version_count, 2);
    assert_eq!(status.pending_high_risk, 1);
    assert_eq!(status.pending_permission_impacts, 1);
    assert_eq!(status.ledger_failures, 1);
    assert_eq!(
        status.latest_shadow_evaluation.unwrap().verdict,
        "needs_review"
    );
}

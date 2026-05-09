use super::*;

fn test_db() -> DbInstance {
    let path = format!("/tmp/test-agent-evolve-report-{}.db", uuid::Uuid::new_v4());
    let db = DbInstance::new("sqlite", &path, "").unwrap();
    archon_learning::schema::ensure_learning_schema(&db).unwrap();
    db
}

#[test]
fn report_summarizes_agent_evolution_state() {
    let db = test_db();
    archon_learning::agent_profile_versions::insert_agent_profile_version(
        &db,
        &archon_learning::agent_profile_versions::AgentProfileVersionRecord::new(
            "profile-1",
            "reviewer",
            1,
            "proposal",
            "2026-05-08T12:00:00Z",
        )
        .mark_active(),
    )
    .unwrap();
    archon_learning::agent_evolution_proposals::insert_agent_evolution_proposal(
        &db,
        &archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord::new(
            "prop-1",
            "reviewer",
            "v1",
            "v2",
            "tool_access_profile",
            "2026-05-08T12:01:00Z",
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
            "2026-05-08T12:02:00Z",
        )
        .with_model_provider("claude-sonnet-4-6", "anthropic")
        .with_provider_incident("provider-event-1")
        .with_scores(Some(0.8), Some(0.6))
        .with_user_feedback(None, Some(true)),
    )
    .unwrap();
    archon_learning::agent_shadow_evaluations::insert_agent_shadow_evaluation(
        &db,
        &archon_learning::agent_shadow_evaluations::AgentShadowEvaluationRecord::new(
            "shadow-1",
            "prop-1",
            "reviewer",
            "hold",
            "2026-05-08T12:03:00Z",
        )
        .with_counts(1, 2),
    )
    .unwrap();
    archon_learning::memory_promotion_candidates::insert_memory_promotion_candidate(
        &db,
        &archon_learning::memory_promotion_candidates::MemoryPromotionCandidateRecord::new(
            "memory-1",
            "reviewer",
            "user_correction",
            "governed_learning_event",
            "Repeated correction",
            "2026-05-08T12:04:00Z",
        )
        .with_scores(1.0, 1.0, 1.0, 1.0, 1.0),
    )
    .unwrap();

    let report = AgentEvolutionReport::load(&db, "reviewer").unwrap();

    assert_eq!(report.profile_version_count, 1);
    assert_eq!(report.active_profile.unwrap().version_id, "profile-1");
    assert_eq!(report.proposals.by_risk.get("high"), Some(&1));
    assert_eq!(report.proposals.permission_impacts, 1);
    assert_eq!(report.ledger.provider_incidents, 1);
    assert_eq!(report.ledger.average_quality, Some(0.8));
    assert_eq!(report.shadow.regressions, 1);
    assert_eq!(report.memory.top_candidates[0].score, 1.0);
}

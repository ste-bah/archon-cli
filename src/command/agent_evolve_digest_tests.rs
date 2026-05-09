use super::*;

fn test_db() -> DbInstance {
    let path = format!("/tmp/test-agent-evolve-digest-{}.db", uuid::Uuid::new_v4());
    let db = DbInstance::new("sqlite", &path, "").unwrap();
    archon_learning::schema::ensure_learning_schema(&db).unwrap();
    db
}

#[test]
fn digest_generates_and_persists_structured_claims() {
    let db = test_db();
    archon_learning::agent_evolution_ledger::insert_agent_performance_ledger_record(
        &db,
        &archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord::new(
            "ledger-1",
            "reviewer",
            "failed",
            "2026-05-08T12:00:00Z",
        )
        .with_provider_incident("provider-event-1")
        .with_user_feedback(None, Some(true)),
    )
    .unwrap();
    archon_learning::agent_evolution_ledger::insert_agent_performance_ledger_record(
        &db,
        &archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord::new(
            "ledger-2",
            "reviewer",
            "succeeded",
            "2026-05-08T12:01:00Z",
        )
        .with_user_feedback(Some(true), Some(true)),
    )
    .unwrap();
    archon_learning::agent_evolution_ledger::insert_agent_performance_ledger_record(
        &db,
        &archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord::new(
            "ledger-3",
            "reviewer",
            "succeeded",
            "2026-05-08T12:02:00Z",
        )
        .with_user_feedback(Some(true), Some(true)),
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
            "2026-05-08T12:03:00Z",
        )
        .with_risk("high", "manual_review_required")
        .with_permission_impact(),
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
        ),
    )
    .unwrap();

    let digest = AgentKnowledgeDigestReport::load(&db, "reviewer").unwrap();
    let persisted = persist_claims(&db, &digest.claims).unwrap();
    let claims =
        archon_learning::agent_knowledge_digest::list_agent_knowledge_claims(&db, "reviewer")
            .unwrap();

    assert!(digest.claims.len() >= 4);
    assert!(!digest.contradictions.is_empty());
    assert!(!digest.open_questions.is_empty());
    assert_eq!(persisted.len(), digest.claims.len());
    assert_eq!(claims.len(), digest.claims.len());
    assert!(
        claims
            .iter()
            .any(|claim| claim.claim_type == "provider_reliability")
    );
}

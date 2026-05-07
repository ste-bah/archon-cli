use super::*;
use archon_core::agent::UserCorrectionEventPayload;
use cozo::DbInstance;
use std::sync::Arc;

#[test]
fn on_agent_start_returns_context_with_all_none() {
    let config = LearningIntegrationConfig::default();
    let mut integration = LearningIntegration::new(None, None, config, None);

    let ctx = integration.on_agent_start("test-agent", "phase1", "build widget", "pipe-001");

    // Should return default (empty) context without panicking
    assert!(ctx.sona_context.is_empty());
    assert!(ctx.reasoning_context.is_empty());
    assert!(ctx.desc_episodes.is_empty());
    assert!(ctx.reflexion.is_none());
}

#[test]
fn on_agent_complete_works_with_sona_none() {
    let config = LearningIntegrationConfig::default();
    let mut integration = LearningIntegration::new(None, None, config, None);

    // Should not panic
    integration.on_agent_complete("test-agent", 0.95, "completed successfully");
}

fn test_event_db() -> Arc<DbInstance> {
    let path = format!(
        "/tmp/test-user-correction-event-{}.db",
        uuid::Uuid::new_v4()
    );
    let db = DbInstance::new("sqlite", &path, "").unwrap();
    archon_learning::schema::ensure_learning_schema(&db).unwrap();
    Arc::new(db)
}

#[test]
fn record_user_correction_event_writes_to_store() {
    let db = test_event_db();
    let integration =
        LearningIntegration::new(None, None, LearningIntegrationConfig::default(), None)
            .with_event_store(Arc::clone(&db));

    integration.record_user_correction_event(UserCorrectionEventPayload {
        correction_type: "ApproachCorrection".into(),
        top_rule_id: Some("rule-123".into()),
        user_input_excerpt: "use this instead".into(),
        session_context: "turn:7".into(),
    });

    let events = archon_learning::store::list_all_learning_events(db.as_ref()).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].event_type,
        archon_learning::models::LearningEventType::UserCorrected
    );
    assert_eq!(events[0].source_artifact_id, "rule-123");
    assert_eq!(events[0].workspace_id, "turn:7");
    assert_eq!(events[0].signal["correction_type"], "ApproachCorrection");
    assert_eq!(events[0].signal["user_input_excerpt"], "use this instead");

    let proposals =
        archon_learning::store::list_behaviour_proposals(db.as_ref(), Some("Pending")).unwrap();
    assert!(proposals.is_empty());
}

#[test]
fn record_user_correction_event_skips_when_store_absent() {
    let integration =
        LearningIntegration::new(None, None, LearningIntegrationConfig::default(), None);

    integration.record_user_correction_event(UserCorrectionEventPayload {
        correction_type: "ApproachCorrection".into(),
        top_rule_id: Some("rule-123".into()),
        user_input_excerpt: "use this instead".into(),
        session_context: "turn:7".into(),
    });
}

#[test]
fn sherlock_approved_maps_to_high_quality() {
    let mut sherlock = SherlockLearningIntegration::new(None);
    sherlock.record_verdict("agent-a", SherlockVerdict::Approved);

    // Pass rate should be 1.0 with a single approval
    assert!(sherlock.pass_rate() >= 0.8);
}

#[test]
fn sherlock_rejected_maps_to_low_quality() {
    let mut sherlock = SherlockLearningIntegration::new(None);
    sherlock.record_verdict("agent-a", SherlockVerdict::Rejected);

    // Pass rate with a single rejection should be 0.0
    assert!(sherlock.pass_rate() <= 0.3);
    assert!(!sherlock.failed_patterns().is_empty());
}

#[test]
fn sherlock_pass_rate_calculation() {
    let mut sherlock = SherlockLearningIntegration::new(None);
    sherlock.record_verdict("agent-a", SherlockVerdict::Approved);
    sherlock.record_verdict("agent-b", SherlockVerdict::Approved);
    sherlock.record_verdict("agent-c", SherlockVerdict::Rejected);

    let rate = sherlock.pass_rate();
    // 2 approved / 3 total ~ 0.667
    assert!((rate - 2.0 / 3.0).abs() < 0.001);
}

#[test]
fn memory_coordinator_stores_in_priority_order() {
    let mut coord = PipelineMemoryCoordinator::new();
    coord.coordinate_store("low", "val-low", 1);
    coord.coordinate_store("high", "val-high", 10);
    coord.coordinate_store("mid", "val-mid", 5);

    // First item should be highest priority
    let first = coord.coordinate_recall("high");
    assert!(first.is_some());
    assert_eq!(first.unwrap().priority, 10);

    // Verify ordering via flush
    let flushed = coord.flush();
    assert_eq!(flushed.len(), 3);
    assert_eq!(flushed[0].key, "high");
    assert_eq!(flushed[1].key, "mid");
    assert_eq!(flushed[2].key, "low");
}

#[test]
fn memory_coordinator_flush_clears_pending() {
    let mut coord = PipelineMemoryCoordinator::new();
    coord.coordinate_store("k1", "v1", 5);
    coord.coordinate_store("k2", "v2", 3);

    assert_eq!(coord.pending_count(), 2);

    let flushed = coord.flush();
    assert_eq!(flushed.len(), 2);
    assert_eq!(coord.pending_count(), 0);
    assert_eq!(coord.total_flushes(), 1);
}

#[test]
fn phd_citation_quality_average() {
    let mut phd = PhDLearningIntegration::new();
    phd.record_citation_quality("agent-a", 0.8);
    phd.record_citation_quality("agent-b", 0.6);
    phd.record_citation_quality("agent-c", 0.9);

    let avg = phd.get_citation_quality_avg();
    // (0.8 + 0.6 + 0.9) / 3 ~ 0.7667
    assert!((avg - 0.7667).abs() < 0.01);
}

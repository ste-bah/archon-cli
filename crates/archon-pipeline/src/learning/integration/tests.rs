use super::*;
use crate::learning::desc::{DescEpisode, DescEpisodeStore, EpisodeQuery};
use crate::learning::patterns::PatternStore;
use crate::learning::reasoning::{ReasoningBank, ReasoningBankConfig, ReasoningBankDeps};
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
fn correction_cluster_persists_policy_evaluated_pending_proposal() {
    let db = test_event_db();
    let integration =
        LearningIntegration::new(None, None, LearningIntegrationConfig::default(), None)
            .with_event_store(Arc::clone(&db));

    for _ in 0..3 {
        integration.record_user_correction_event(UserCorrectionEventPayload {
            correction_type: "ApproachCorrection".into(),
            top_rule_id: Some("rule-policy-queued".into()),
            user_input_excerpt: "use this instead".into(),
            session_context: "turn:7".into(),
        });
    }

    let proposals =
        archon_learning::store::list_behaviour_proposals(db.as_ref(), Some("Pending")).unwrap();
    assert_eq!(proposals.len(), 1);
    assert_eq!(proposals[0].current_version, "none");
    let policy_rows = archon_learning::store::list_policy_decisions_for_proposal(
        db.as_ref(),
        &proposals[0].proposal_id,
    )
    .unwrap();
    assert!(
        policy_rows
            .iter()
            .any(|row| row.rule_name == "autonomous_disabled")
    );
}

#[test]
fn persistent_sona_trajectory_reaches_trainer_query_source() {
    let db = Arc::new(DbInstance::new("mem", "", "").unwrap());
    crate::learning::schema::initialize_learning_schemas(db.as_ref()).unwrap();
    let mut integration = LearningIntegration::new_with_persistent_sona(
        Arc::clone(&db),
        LearningIntegrationConfig::default(),
        None,
        8,
    );

    let ctx = integration.on_agent_start("agent-a", "phase1", "build widget", "pipe-001");
    assert!(ctx.sona_context.contains("trajectory_id="));
    integration.on_agent_complete("agent-a", 0.95, "completed successfully");

    let samples =
        crate::learning::gnn::auto_trainer_runtime::query_trajectories_for_training(db.as_ref(), 8)
            .unwrap();
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].embedding.len(), 8);
    assert!(samples[0].quality > 0.0);
}

#[test]
fn persistent_learning_stack_injects_reasoning_bank_and_desc_episodes() {
    let db = Arc::new(DbInstance::new("mem", "", "").unwrap());
    crate::learning::schema::initialize_learning_schemas(db.as_ref()).unwrap();
    let desc_store = DescEpisodeStore::from_arc(Arc::clone(&db));
    desc_store
        .store_episode(&DescEpisode {
            episode_id: "episode-seed".into(),
            session_id: "prior-session".into(),
            task_type: "phase1".into(),
            description: "need schema design before API wiring".into(),
            solution: "Designed schema first, then wired API boundaries.".into(),
            outcome: "success".into(),
            quality_score: 0.95,
            reward: 0.95,
            tags: vec!["pipeline".into()],
            trajectory_id: None,
            created_at: 0,
            updated_at: 0,
        })
        .unwrap();

    let reasoning_bank = ReasoningBank::new(ReasoningBankDeps {
        pattern_store: PatternStore::new(),
        causal_memory: None,
        gnn_enhancer: None,
        sona_engine: None,
        config: ReasoningBankConfig::default(),
    });
    let mut integration =
        LearningIntegration::new(None, None, LearningIntegrationConfig::default(), None)
            .with_reasoning_bank(reasoning_bank)
            .with_desc_store(desc_store);

    let ctx = integration.on_agent_start(
        "agent-a",
        "phase1",
        "break down the implementation steps",
        "pipe-001",
    );

    assert!(!ctx.desc_episodes.is_empty());
    assert!(ctx.reasoning_context.contains("Decomposition"));

    integration.on_agent_complete("agent-a", 0.9, "implemented in order");
    let episodes = DescEpisodeStore::from_arc(db)
        .find_episodes(&EpisodeQuery {
            task_type: Some("phase1".into()),
            min_quality: Some(0.8),
            limit: 10,
        })
        .unwrap();
    assert!(episodes.len() >= 2);
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

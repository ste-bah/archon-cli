use std::sync::Arc;

use archon_core::agent::UserCorrectionEventPayload;
use archon_learning::models::{BehaviourManifestKind, LearningEventType};
use archon_pipeline::learning::integration::{LearningIntegration, LearningIntegrationConfig};
use cozo::DbInstance;

fn test_db() -> Arc<DbInstance> {
    let path = format!("/tmp/corrections-close-loop-{}.db", uuid::Uuid::new_v4());
    let db = DbInstance::new("sqlite", &path, "").expect("open test db");
    archon_learning::schema::ensure_learning_schema(&db).expect("ensure schema");
    Arc::new(db)
}

#[test]
fn corrections_close_loop_e2e() {
    let db = test_db();
    let integration =
        LearningIntegration::new(None, None, LearningIntegrationConfig::default(), None)
            .with_event_store(Arc::clone(&db));

    for _ in 0..3 {
        integration.record_user_correction_event(UserCorrectionEventPayload {
            correction_type: "ApproachCorrection".into(),
            top_rule_id: Some("rule-close-loop".into()),
            user_input_excerpt: "use this instead".into(),
            session_context: "session-close-loop".into(),
        });
    }

    let events = archon_learning::store::list_all_learning_events(db.as_ref())
        .expect("list learning events");
    let corrections: Vec<_> = events
        .iter()
        .filter(|event| event.event_type == LearningEventType::UserCorrected)
        .collect();
    assert_eq!(corrections.len(), 3);

    let proposals = archon_learning::proposal::generate_proposals(&events);
    assert_eq!(proposals.len(), 1);
    assert_eq!(
        proposals[0].manifest_kind,
        BehaviourManifestKind::BehaviouralRuleAdjustment
    );
    assert!(
        proposals[0]
            .diff
            .contains("\"rule_id\":\"rule-close-loop\"")
    );
    assert!(proposals[0].diff.contains("\"correction_count\":3"));

    let pending = archon_learning::store::list_behaviour_proposals(db.as_ref(), Some("Pending"))
        .expect("list pending proposals");
    assert_eq!(pending.len(), 1);
    assert_eq!(
        pending[0].manifest_kind,
        BehaviourManifestKind::BehaviouralRuleAdjustment
    );
    assert!(pending[0].diff.contains("\"rule_id\":\"rule-close-loop\""));
    assert!(pending[0].diff.contains("\"correction_count\":3"));
}

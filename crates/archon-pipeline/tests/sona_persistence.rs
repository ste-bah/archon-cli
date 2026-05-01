//! Integration tests for SonaEngine persistence.
//!
//! Validates: CozoDB persistence, hashmap-only mode, and embedding computation.

use std::sync::Arc;

use archon_memory::embedding::EmbeddingProvider;
use archon_memory::types::MemoryError;
use archon_pipeline::learning::schema::initialize_learning_schemas;
use archon_pipeline::learning::sona::{FeedbackInput, SonaConfig, SonaEngine};
use cozo::{DbInstance, ScriptMutability};

// ---------------------------------------------------------------------------
// Stub Embedder
// ---------------------------------------------------------------------------

struct StubEmbedder {
    dim: usize,
}

impl EmbeddingProvider for StubEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, MemoryError> {
        Ok(texts.iter().map(|_| vec![1.0_f32; self.dim]).collect())
    }

    fn dimensions(&self) -> usize {
        self.dim
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn mem_db() -> DbInstance {
    DbInstance::new("mem", "", "").unwrap()
}

/// Count rows in the trajectories relation.
fn count_trajectories(db: &DbInstance) -> i64 {
    let result = db
        .run_script(
            r#"
?[count(trajectory_id)] := *trajectories[trajectory_id, _, _, _, _, _, _, _, _, _, _, _, _]
"#,
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    result.rows[0][0].get_int().unwrap()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn provide_feedback_with_db_persists_to_cozo() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    let mut config = SonaConfig::default();
    config.db = Some(Arc::new(db.clone()));

    let mut engine = SonaEngine::new(config);
    let trajectory = engine.create_trajectory("test.route", "test-agent", "session-1");

    let input = FeedbackInput {
        trajectory_id: trajectory.trajectory_id.clone(),
        quality: 0.85,
        l_score: 0.9,
        success_rate: 1.0,
    };
    engine.provide_feedback(&input).unwrap();

    // Verify trajectory is accessible via get_trajectory with correct values.
    let stored = engine
        .get_trajectory(&trajectory.trajectory_id)
        .expect("trajectory should exist in hashmap after feedback");
    assert!(
        (stored.quality - 0.85).abs() < f64::EPSILON,
        "quality should be updated to 0.85"
    );
    let expected_reward = 0.85 * 0.9 * 1.0;
    assert!(
        (stored.reward - expected_reward).abs() < f64::EPSILON,
        "reward should be quality * l_score * success_rate"
    );

    // Verify exactly one row was persisted to CozoDB.
    assert_eq!(
        count_trajectories(&db),
        1,
        "cozo should have exactly 1 trajectory row"
    );

    // Verify quality and reward in CozoDB match.
    let result = db
        .run_script(
            r#"
?[quality, reward] := *trajectories[trajectory_id, _, _, _, _, _, _, quality, reward, _, _, _, _]
"#,
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert!(
        (result.rows[0][0].get_float().unwrap() - 0.85).abs() < f64::EPSILON,
        "cozo quality should be 0.85"
    );
    assert!(
        (result.rows[0][1].get_float().unwrap() - expected_reward).abs() < f64::EPSILON,
        "cozo reward should match expected"
    );
}

#[test]
fn provide_feedback_without_db_only_updates_hashmap() {
    // Create engine without any DB — hashmap-only storage.
    let config = SonaConfig::default();
    let mut engine = SonaEngine::new(config);
    let trajectory = engine.create_trajectory("test.route", "test-agent", "session-1");

    let input = FeedbackInput {
        trajectory_id: trajectory.trajectory_id.clone(),
        quality: 0.7,
        l_score: 0.8,
        success_rate: 0.9,
    };
    engine.provide_feedback(&input).unwrap();

    // Trajectory is accessible via get_trajectory.
    let stored = engine
        .get_trajectory(&trajectory.trajectory_id)
        .expect("trajectory should exist in hashmap after feedback");
    assert!(
        (stored.quality - 0.7).abs() < f64::EPSILON,
        "quality should be updated to 0.7"
    );
    let expected_reward = 0.7 * 0.8 * 0.9;
    assert!(
        (stored.reward - expected_reward).abs() < f64::EPSILON,
        "reward should be quality * l_score * success_rate"
    );

    // Separate empty DB check — confirms that without db, nothing is persisted.
    let empty_db = mem_db();
    initialize_learning_schemas(&empty_db).unwrap();
    assert_eq!(
        count_trajectories(&empty_db),
        0,
        "fresh DB should have zero trajectories"
    );
}

#[test]
fn provide_feedback_embeds_when_quality_first_set_nonzero() {
    let db = mem_db();
    initialize_learning_schemas(&db).unwrap();

    let mut config = SonaConfig::default();
    config.db = Some(Arc::new(db));
    config.embedding_provider = Some(Arc::new(StubEmbedder { dim: 4 }));
    config.gnn_input_dim = 4;

    let mut engine = SonaEngine::new(config);
    let trajectory = engine.create_trajectory("test.route", "test-agent", "session-1");

    // Newly created trajectory starts with quality=0 and empty embedding.
    assert!(
        trajectory.embedding.is_empty(),
        "fresh trajectory should have empty embedding"
    );
    assert!(
        (trajectory.quality - 0.0).abs() < f64::EPSILON,
        "fresh trajectory should have quality 0.0"
    );

    // Provide feedback with non-zero quality — should trigger compute + pad.
    let input = FeedbackInput {
        trajectory_id: trajectory.trajectory_id.clone(),
        quality: 0.8,
        l_score: 0.9,
        success_rate: 1.0,
    };
    engine.provide_feedback(&input).unwrap();

    // Verify embedding is now [1.0; 4] (stub embedder outputs all 1.0, dim=4).
    let stored = engine
        .get_trajectory(&trajectory.trajectory_id)
        .expect("trajectory should exist after feedback");
    assert_eq!(
        stored.embedding.len(),
        4,
        "embedding should be padded/truncated to gnn_input_dim=4"
    );
    for (i, &val) in stored.embedding.iter().enumerate() {
        assert!(
            (val - 1.0_f32).abs() < f32::EPSILON,
            "embedding[{i}] should be 1.0, got {val}"
        );
    }
    assert!(
        (stored.quality - 0.8).abs() < f64::EPSILON,
        "quality should be updated to 0.8"
    );
}

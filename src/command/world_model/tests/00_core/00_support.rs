use super::*;

fn seed_training_rows(root: &std::path::Path) {
    use archon_world_model::schema::{WorldActionKind, WorldTraceRow};

    let store = archon_world_model::storage::WorldModelStore::open(root).unwrap();
    let mut first = WorldTraceRow::new("session-1", WorldActionKind::ToolCall).with_row_id("r1");
    first.redacted_excerpt = Some("run cargo test".into());
    let mut second =
        WorldTraceRow::new("session-1", WorldActionKind::Verification).with_row_id("r2");
    second.redacted_excerpt = Some("cargo test failed".into());
    let mut third = WorldTraceRow::new("session-1", WorldActionKind::Retry).with_row_id("r3");
    third.redacted_excerpt = Some("fix test and rerun".into());

    store.persist_rows(&[first, second, third]).unwrap();
}

fn candidate_id_from(rendered: &str) -> String {
    rendered
        .lines()
        .find_map(|line| line.trim().strip_prefix("Candidate: "))
        .expect("train output should contain candidate id")
        .to_string()
}

fn passing_jepa_eval_record(candidate_id: &str) -> archon_world_model::jepa::JepaEvalRecord {
    archon_world_model::jepa::JepaEvalRecord {
        candidate_id: candidate_id.into(),
        mode: archon_world_model::jepa::PersistedEvalMode::Full,
        baseline_skipped: false,
        skipped_reason: None,
        corpus_fingerprint: None,
        config_fingerprint: "legacy".to_string(),
        eval_schema_version: 0,
        comparison: Some(archon_world_model::jepa::JepaRepresentationComparisonReport {
            candidate_id: candidate_id.into(),
            baseline_backend: "fastembed".into(),
            baseline_available: true,
            failure_reason: None,
            heldout_examples: 200,
            min_heldout_examples: 200,
            jepa_next_state_cosine_similarity: 0.90,
            baseline_next_state_cosine_similarity: 0.80,
            relative_improvement: 0.125,
            min_baseline_improvement: 0.05,
            brier_regressed: false,
            passed: true,
        }),
        collapse: archon_world_model::jepa::JepaCollapseReport {
            mean_latent_std: 0.06,
            effective_rank_ratio: 0.60,
            min_latent_std: 0.05,
            min_effective_rank_ratio: 0.50,
            passes: true,
        },
        horizon: archon_world_model::jepa::JepaHorizonReport {
            e_1: Some(0.10),
            e_3: Some(0.12),
            e_5: Some(0.15),
            tolerance: 0.02,
            passes: true,
        },
        gates: archon_world_model::jepa::JepaPromotionGateReport::from_parts(
            true, true, true, true, true, true,
        ),
        created_at: chrono::Utc::now(),
    }
}

fn prediction_id_from(rendered: &str) -> String {
    rendered
        .lines()
        .find_map(|line| line.trim().strip_prefix("Prediction id: "))
        .expect("prediction output should contain prediction id")
        .to_string()
}

fn test_config() -> archon_core::config::ArchonConfig {
    let mut config = archon_core::config::ArchonConfig::default();
    config.learning.world_model.embeddings.provider = "deterministic-hash".into();
    config
}

fn jepa_test_config() -> archon_core::config::ArchonConfig {
    let mut config = test_config();
    config.learning.world_model.model_kind = "jepa_transition".into();
    config.learning.world_model.state_dim = 8;
    config.learning.world_model.jepa.latent_dim = 8;
    config.learning.world_model.jepa.context_window_rows = 2;
    config.learning.world_model.jepa.target_window_rows = 1;
    config.learning.world_model.jepa.prediction_horizons = vec![1];
    config.learning.world_model.training.backend = "cpu".into();
    config
}

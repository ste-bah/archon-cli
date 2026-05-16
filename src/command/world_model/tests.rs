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
        comparison: archon_world_model::jepa::JepaRepresentationComparisonReport {
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
        },
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

#[test]
fn status_reports_cold_start_defaults() {
    let rendered = render_world_status_with_stats(
        &archon_core::config::ArchonConfig::default(),
        archon_world_model::ColdStartStats::default(),
    );

    assert!(rendered.contains("World Model Status"));
    assert!(rendered.contains("Enabled:            true"));
    assert!(rendered.contains("Training backend:   auto"));
    assert!(rendered.contains("Corpus rows:        0"));
    assert!(rendered.contains("cold_start"));
    assert!(rendered.contains("Active model:       none"));
    assert!(rendered.contains("JEPA status:        disabled"));
    assert!(rendered.contains("Selected backend:   cpu"));
    assert!(rendered.contains("Advisor status:     fail-open"));
    assert!(rendered.contains("cosine >= 0.95"));
}

#[test]
fn status_reports_ready_when_cold_start_thresholds_are_met() {
    let rendered = render_world_status_with_stats(
        &archon_core::config::ArchonConfig::default(),
        archon_world_model::ColdStartStats {
            rows: 1_000,
            sessions: 50,
            observed_days: 7,
        },
    );

    assert!(rendered.contains("Cold-start status:  ready"));
}

#[test]
fn ingest_requires_session_or_backfill() {
    assert!(validate_ingest_args(None, false).is_err());
    assert!(validate_ingest_args(Some("session-1"), true).is_err());
    assert!(validate_ingest_args(Some("session-1"), false).is_ok());
    assert!(validate_ingest_args(None, true).is_ok());
}

#[test]
fn finds_activity_paths_under_sessions_dir() {
    let temp = tempfile::tempdir().unwrap();
    let activity_dir = temp.path().join("s1").join("activity");
    std::fs::create_dir_all(&activity_dir).unwrap();
    std::fs::write(activity_dir.join("events.jsonl"), "").unwrap();

    let paths = activity_jsonl_paths_under(temp.path()).unwrap();

    assert_eq!(paths.len(), 1);
    assert!(paths[0].ends_with("events.jsonl"));
}

#[test]
fn retention_policy_converts_mb_to_bytes() {
    let config = archon_core::config::ArchonConfig::default();
    let policy = retention_policy(&config);

    assert_eq!(policy.jsonl_rotate_bytes, 500 * 1024 * 1024);
    assert_eq!(policy.raw_retention_days, 90);
}

#[test]
fn predict_next_fails_open_when_cold() {
    let temp = tempfile::tempdir().unwrap();
    let rendered = render_predict_next_with_state(
        &archon_core::config::ArchonConfig::default(),
        temp.path(),
        archon_world_model::ColdStartStats::default(),
        None,
        "s1",
        "a1",
        "run tests",
    );

    assert!(rendered.contains("Unavailable: ColdStart"));
    assert!(rendered.contains("Behavior: fail-open"));
}

#[test]
fn predict_next_fails_open_without_active_model() {
    let temp = tempfile::tempdir().unwrap();
    let rendered = render_predict_next_with_state(
        &archon_core::config::ArchonConfig::default(),
        temp.path(),
        archon_world_model::ColdStartStats {
            rows: 1_000,
            sessions: 50,
            observed_days: 7,
        },
        None,
        "s1",
        "a1",
        "run tests",
    );

    assert!(rendered.contains("Unavailable: CandidateOnly"));
    assert!(rendered.contains("Behavior: fail-open"));
}

#[test]
fn predict_next_uses_active_model_when_ready() {
    let temp = tempfile::tempdir().unwrap();
    seed_training_rows(temp.path());
    let trained = candidate::render_train(&test_config(), temp.path(), true, None).unwrap();
    let candidate_id = candidate_id_from(&trained);

    let rendered = render_predict_next_with_state(
        &test_config(),
        temp.path(),
        archon_world_model::ColdStartStats {
            rows: 1_000,
            sessions: 50,
            observed_days: 7,
        },
        Some(candidate_id.clone()),
        "s1",
        "a1",
        "run tests",
    );

    assert!(rendered.contains(&format!("Model: {candidate_id}")));
    assert!(rendered.contains("Inference: active_checkpoint"));
    assert!(rendered.contains("next-state dim="));

    let prediction_id = prediction_id_from(&rendered);
    let explained = actions::render_explain(temp.path(), &prediction_id);
    assert!(explained.contains(&format!("Prediction: {prediction_id}")));
    assert!(explained.contains("Predicted next state:"));
    assert!(explained.contains("Outcome: pending"));
    assert!(explained.contains("Evidence refs: runtime_action:a1"));
    let persisted = predict::load_prediction(temp.path(), &prediction_id)
        .unwrap()
        .expect("prediction should be persisted");
    assert!(
        persisted
            .guardrail_scores
            .expect("prediction should include auxiliary guardrail scores")
            .predicted_verification_needed
            .is_some()
    );
}

#[test]
fn record_outcome_links_actual_result_to_prediction() {
    let temp = tempfile::tempdir().unwrap();
    seed_training_rows(temp.path());
    let trained = candidate::render_train(&test_config(), temp.path(), true, None).unwrap();
    let candidate_id = candidate_id_from(&trained);
    let rendered = render_predict_next_with_state(
        &test_config(),
        temp.path(),
        archon_world_model::ColdStartStats {
            rows: 1_000,
            sessions: 50,
            observed_days: 7,
        },
        Some(candidate_id),
        "s1",
        "a1",
        "run tests",
    );
    let prediction_id = prediction_id_from(&rendered);

    let outcome = predict::render_record_outcome(
        &test_config(),
        temp.path(),
        &prediction_id,
        "tests passed after retry",
    )
    .unwrap();
    let explained = actions::render_explain(temp.path(), &prediction_id);

    assert!(outcome.contains("World Model Outcome"));
    assert!(outcome.contains("Actual outcome: tests passed after retry"));
    assert!(outcome.contains("Latent surprise:"));
    assert!(explained.contains("Actual outcome: tests passed after retry"));
    assert!(explained.contains("Latent surprise:"));
}

#[test]
#[cfg_attr(
    target_os = "macos",
    ignore = "macos-latest GitHub runner: trainer_tick returns Should train: false despite canonicalize() fix that works on developer M3 hardware. Suspected interaction with macos-15 sandbox-exec + tempfile + sqlite WAL. Re-enable once root cause identified."
)]
fn trainer_tick_writes_candidate_when_idle_and_triggered() {
    let temp = tempfile::tempdir().unwrap();
    seed_training_rows(temp.path());
    let mut config = test_config();
    config.learning.world_model.auto_trainer.first_run_threshold = 3;

    let rendered =
        candidate::render_trainer_tick(&config, temp.path(), Some(600_000), None, Some(90), false)
            .unwrap();

    assert!(
        rendered.contains("World Model Trainer Tick"),
        "trainer tick output missing header; full output:\n{rendered}"
    );
    assert!(
        rendered.contains("Should train: true"),
        "trainer decided NOT to train despite idle/battery/trigger conditions met; full output:\n{rendered}"
    );
    assert!(
        rendered.contains("Candidate: world-model-candidate-"),
        "trainer did not produce a candidate; full output:\n{rendered}"
    );
}

#[test]
fn jepa_trainer_tick_respects_recent_activity_gate() {
    let temp = tempfile::tempdir().unwrap();
    seed_training_rows(temp.path());
    let mut config = test_config();
    config.learning.world_model.jepa.enabled = true;
    config.learning.world_model.state_dim = 8;
    config.learning.world_model.jepa.latent_dim = 8;

    let rendered =
        candidate::render_trainer_tick(&config, temp.path(), Some(10_000), None, Some(90), false)
            .unwrap();

    assert!(rendered.contains("World Model JEPA Trainer Tick"));
    assert!(rendered.contains("Should train: false"));
    assert!(rendered.contains("Decision: RecentActivity"));
}

#[test]
fn train_writes_candidate_manifest_from_stored_rows() {
    let temp = tempfile::tempdir().unwrap();
    seed_training_rows(temp.path());

    let rendered = candidate::render_train(&test_config(), temp.path(), true, Some(1_000)).unwrap();

    let candidate_id = candidate_id_from(&rendered);
    assert!(rendered.contains("World Model Train"));
    assert!(rendered.contains("Examples: 2"));
    assert!(
        temp.path()
            .join("candidates")
            .join(format!("{candidate_id}.json"))
            .exists()
    );
}

#[test]
fn train_jepa_writes_candidate_manifest_from_stored_rows() {
    let temp = tempfile::tempdir().unwrap();
    seed_training_rows(temp.path());
    let mut config = test_config();
    config.learning.world_model.state_dim = 8;
    config.learning.world_model.jepa.latent_dim = 8;
    config.learning.world_model.jepa.context_window_rows = 2;
    config.learning.world_model.jepa.target_window_rows = 1;
    config.learning.world_model.jepa.prediction_horizons = vec![1];

    let rendered = candidate::render_train_jepa(&config, temp.path(), true, Some(1_000)).unwrap();

    let candidate_id = candidate_id_from(&rendered);
    assert!(rendered.contains("World Model JEPA Train"));
    assert!(rendered.contains("Model kind: jepa_transition"));
    assert!(rendered.contains("Requested backend: auto"));
    assert!(rendered.contains("Selected backend: cpu"));
    assert!(rendered.contains("Native encode: true"));
    assert!(rendered.contains("Latent dim: 8"));
    assert!(rendered.contains("Examples: 2"));
    assert!(
        temp.path()
            .join("jepa")
            .join("candidates")
            .join(format!("{candidate_id}.json"))
            .exists()
    );
    assert!(
        temp.path()
            .join("jepa")
            .join("candidates")
            .join(format!("{candidate_id}.safetensors"))
            .exists()
    );
}

#[test]
fn train_jepa_rejects_latent_dim_state_dim_mismatch() {
    let temp = tempfile::tempdir().unwrap();
    seed_training_rows(temp.path());
    let mut config = test_config();
    config.learning.world_model.state_dim = 16;
    config.learning.world_model.jepa.latent_dim = 8;

    let error = candidate::render_train_jepa(&config, temp.path(), true, None).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("must equal active transition state_dim")
    );
}

#[test]
fn inspect_jepa_reports_candidate_manifest() {
    let temp = tempfile::tempdir().unwrap();
    seed_training_rows(temp.path());
    let mut config = test_config();
    config.learning.world_model.state_dim = 8;
    config.learning.world_model.jepa.latent_dim = 8;
    config.learning.world_model.jepa.context_window_rows = 2;
    config.learning.world_model.jepa.target_window_rows = 1;
    config.learning.world_model.jepa.prediction_horizons = vec![1];
    let trained = candidate::render_train_jepa(&config, temp.path(), true, None).unwrap();
    let candidate_id = candidate_id_from(&trained);

    let rendered = candidate::render_inspect_jepa(temp.path(), &candidate_id).unwrap();

    assert!(rendered.contains("World Model JEPA Inspect"));
    assert!(rendered.contains("Model kind: jepa_transition"));
    assert!(rendered.contains("Stop gradient: true"));
    assert!(rendered.contains("Requested backend: auto"));
    assert!(rendered.contains("Selected backend: cpu"));
    assert!(rendered.contains("Host fallback count: 0"));
}

#[test]
fn compare_representations_persists_exploratory_report() {
    let temp = tempfile::tempdir().unwrap();
    seed_training_rows(temp.path());
    let mut config = test_config();
    config.learning.world_model.state_dim = 8;
    config.learning.world_model.jepa.latent_dim = 8;
    config.learning.world_model.jepa.context_window_rows = 2;
    config.learning.world_model.jepa.target_window_rows = 1;
    config.learning.world_model.jepa.prediction_horizons = vec![1];
    let trained = candidate::render_train_jepa(&config, temp.path(), true, None).unwrap();
    let candidate_id = candidate_id_from(&trained);

    let rendered = candidate::render_compare_representations(
        &config,
        temp.path(),
        "deterministic-hash",
        &candidate_id,
    )
    .unwrap();

    assert!(rendered.contains("World Model Representation Comparison"));
    assert!(rendered.contains("Promotion baseline fixed: fastembed"));
    assert!(
        temp.path()
            .join("jepa")
            .join("representation-comparisons")
            .join(format!("{candidate_id}.json"))
            .exists()
    );
}

#[test]
fn promote_jepa_requires_passing_eval_report() {
    let temp = tempfile::tempdir().unwrap();
    seed_training_rows(temp.path());
    let mut config = test_config();
    config.learning.world_model.state_dim = 8;
    config.learning.world_model.jepa.latent_dim = 8;
    config.learning.world_model.jepa.context_window_rows = 2;
    config.learning.world_model.jepa.target_window_rows = 1;
    config.learning.world_model.jepa.prediction_horizons = vec![1];
    let trained = candidate::render_train_jepa(&config, temp.path(), true, None).unwrap();
    let candidate_id = candidate_id_from(&trained);

    let missing = candidate::render_promote_jepa(temp.path(), &candidate_id).unwrap_err();
    assert!(missing.to_string().contains("has no eval report"));

    let registry = archon_world_model::registry::ModelRegistry::open(temp.path()).unwrap();
    let mut failing = passing_jepa_eval_record(&candidate_id);
    failing.gates.representation_baseline = false;
    failing.gates.passed = false;
    registry.write_jepa_eval_report(&failing).unwrap();
    let failed_gate = candidate::render_promote_jepa(temp.path(), &candidate_id).unwrap_err();
    assert!(failed_gate.to_string().contains("has not passed"));

    registry
        .write_jepa_eval_report(&passing_jepa_eval_record(&candidate_id))
        .unwrap();
    let promoted = candidate::render_promote_jepa(temp.path(), &candidate_id).unwrap();

    assert!(promoted.contains("Model kind: jepa_transition"));
    assert_eq!(
        registry.active_model_kind().unwrap().as_deref(),
        Some("jepa_transition")
    );
}

#[test]
fn predict_next_uses_active_jepa_model_when_configured() {
    let temp = tempfile::tempdir().unwrap();
    seed_training_rows(temp.path());
    let mut config = test_config();
    config.learning.world_model.model_kind = "jepa_transition".into();
    config.learning.world_model.state_dim = 8;
    config.learning.world_model.jepa.latent_dim = 8;
    config.learning.world_model.jepa.context_window_rows = 2;
    config.learning.world_model.jepa.target_window_rows = 1;
    config.learning.world_model.jepa.prediction_horizons = vec![1];
    let trained = candidate::render_train_jepa(&config, temp.path(), true, None).unwrap();
    let candidate_id = candidate_id_from(&trained);
    let registry = archon_world_model::registry::ModelRegistry::open(temp.path()).unwrap();
    registry
        .promote_model_kind(&candidate_id, "jepa_transition")
        .unwrap();

    let rendered = render_predict_next_with_state(
        &config,
        temp.path(),
        archon_world_model::ColdStartStats {
            rows: 1_000,
            sessions: 50,
            observed_days: 7,
        },
        Some(candidate_id.clone()),
        "s1",
        "a1",
        "run tests",
    );

    assert!(rendered.contains(&format!("Model: {candidate_id}")));
    assert!(rendered.contains("Model kind: jepa_transition"));
    assert!(rendered.contains("Representation: archon-jepa:"));
    let prediction_id = prediction_id_from(&rendered);
    let persisted = predict::load_prediction(temp.path(), &prediction_id)
        .unwrap()
        .expect("prediction should be persisted");
    assert!(
        persisted
            .guardrail_scores
            .expect("jepa prediction should include auxiliary guardrail scores")
            .predicted_verification_needed
            .is_some()
    );
}

#[test]
fn predict_next_fails_open_for_accelerator_jepa_without_native_runtime() {
    let temp = tempfile::tempdir().unwrap();
    seed_training_rows(temp.path());
    let mut config = test_config();
    config.learning.world_model.model_kind = "jepa_transition".into();
    config.learning.world_model.state_dim = 8;
    config.learning.world_model.jepa.latent_dim = 8;
    config.learning.world_model.jepa.context_window_rows = 2;
    config.learning.world_model.jepa.target_window_rows = 1;
    config.learning.world_model.jepa.prediction_horizons = vec![1];
    let trained = candidate::render_train_jepa(&config, temp.path(), true, None).unwrap();
    let candidate_id = candidate_id_from(&trained);
    let registry = archon_world_model::registry::ModelRegistry::open(temp.path()).unwrap();
    let path = temp
        .path()
        .join("jepa")
        .join("candidates")
        .join(format!("{candidate_id}.json"));
    let mut record: archon_world_model::registry::JepaCandidateRecord =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record.model.metadata.backend = archon_world_model::BackendKind::Cuda;
    record.model.metadata.backend_execution = archon_world_model::JepaBackendExecutionReport {
        requested_backend: archon_world_model::BackendKind::Cuda,
        selected_backend: archon_world_model::BackendKind::Cuda,
        framework: "candle".into(),
        device_name: Some("cuda:0".into()),
        commit_sha: "abc123".into(),
        feature_compiled: true,
        tensor_self_test_passed: true,
        hardware_validation_captured_at: Some(chrono::Utc::now()),
        validation_example_count: 512,
        native_encode: true,
        native_predictor_fit: true,
        native_auxiliary_fit: true,
        native_transition_fit: true,
        native_loss_eval: true,
        native_runtime_prediction: Some(true),
        host_fallback_count: 0,
        allowed_host_stage_count: 0,
        fallback_reason: None,
    };
    record.outcome.metadata = record.model.metadata.clone();
    std::fs::write(&path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    registry
        .promote_model_kind(&candidate_id, "jepa_transition")
        .unwrap();

    let rendered = render_predict_next_with_state(
        &config,
        temp.path(),
        archon_world_model::ColdStartStats {
            rows: 1_000,
            sessions: 50,
            observed_days: 7,
        },
        Some(candidate_id),
        "s1",
        "a1",
        "run tests",
    );

    assert!(rendered.contains("Unavailable: JepaBackendUnavailable"));
    assert!(rendered.contains("Behavior: fail-open"));
}

#[test]
fn predict_next_fails_open_when_jepa_pointer_missing() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = test_config();
    config.learning.world_model.model_kind = "jepa_transition".into();

    let rendered = render_predict_next_with_state(
        &config,
        temp.path(),
        archon_world_model::ColdStartStats {
            rows: 1_000,
            sessions: 50,
            observed_days: 7,
        },
        Some("missing-jepa".into()),
        "s1",
        "a1",
        "run tests",
    );

    assert!(rendered.contains("Unavailable: JepaCheckpointMissing"));
    assert!(rendered.contains("Behavior: fail-open"));
}

#[test]
fn eval_writes_report_and_keeps_unmet_gates_visible() {
    let temp = tempfile::tempdir().unwrap();
    seed_training_rows(temp.path());
    let trained = candidate::render_train(&test_config(), temp.path(), true, None).unwrap();
    let candidate_id = candidate_id_from(&trained);

    let rendered =
        candidate::render_eval(&test_config(), temp.path(), Some(&candidate_id)).unwrap();

    assert!(rendered.contains(&format!("Candidate: {candidate_id}")));
    assert!(rendered.contains("Next-state improvement:"));
    assert!(rendered.contains("Brier labels improved:"));
    assert!(rendered.contains("all primary gates are mandatory"));
    assert!(
        temp.path()
            .join("candidates")
            .join(format!("{candidate_id}.eval.json"))
            .exists()
    );
}

#[test]
fn promote_requires_passing_eval_report() {
    let temp = tempfile::tempdir().unwrap();
    seed_training_rows(temp.path());
    let trained = candidate::render_train(&test_config(), temp.path(), true, None).unwrap();
    let candidate_id = candidate_id_from(&trained);

    let missing = candidate::render_promote(temp.path(), &candidate_id).unwrap_err();
    assert!(missing.to_string().contains("has no eval report"));

    candidate::render_eval(&test_config(), temp.path(), Some(&candidate_id)).unwrap();
    let promoted = candidate::render_promote(temp.path(), &candidate_id).unwrap();

    assert!(promoted.contains("Validation metadata:"));
}

#[test]
fn score_actions_ranks_candidates_from_historical_rows() {
    let temp = tempfile::tempdir().unwrap();
    seed_training_rows(temp.path());
    let actions_path = temp.path().join("actions.json");
    std::fs::write(
        &actions_path,
        r#"[{"id":"verify-now","summary":"run cargo test"},{"id":"skip","summary":"continue without tests"}]"#,
    )
    .unwrap();

    let rendered =
        actions::render_score_actions(&test_config(), temp.path(), "finish feature", &actions_path)
            .unwrap();

    assert!(rendered.contains("World Model Action Scores"));
    assert!(rendered.contains("Candidate actions: 2"));
    assert!(rendered.contains("Calibration: similarity-based, not causal"));
    assert!(rendered.contains("Score record:"));
    assert!(temp.path().join("counterfactuals").exists());
}

#[test]
fn explain_reports_prediction_not_found_until_prediction_store_exists() {
    let temp = tempfile::tempdir().unwrap();

    let rendered = actions::render_explain(temp.path(), "prediction-1");

    assert!(rendered.contains("Prediction: prediction-1"));
    assert!(rendered.contains("Status: not_found"));
}

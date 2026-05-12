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

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
    assert!(rendered.contains("JEPA-inspired status:     disabled"));
    let expected_backend = if cfg!(feature = "cuda")
        && archon_world_model::backend::probe_backend(archon_world_model::BackendKind::Cuda)
            .available
    {
        "cuda"
    } else {
        "cpu"
    };
    assert!(rendered.contains(&format!("Selected backend:   {expected_backend}")));
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

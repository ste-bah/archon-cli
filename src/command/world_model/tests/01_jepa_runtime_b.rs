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
        native_runtime_prediction: Some(false),
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
        "session-1",
        "r1",
        "run tests",
    );

    assert!(rendered.contains("Unavailable: JepaBackendUnavailable"));
    assert!(rendered.contains("Behavior: fail-open"));
}

#[test]
fn predict_next_fails_open_when_jepa_pointer_missing() {
    let temp = tempfile::tempdir().unwrap();
    seed_training_rows(temp.path());
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
        "session-1",
        "r1",
        "run tests",
    );

    assert!(rendered.contains("Unavailable: JepaCheckpointMissing"));
    assert!(rendered.contains("Behavior: fail-open"));
}

#[test]
fn jepa_accelerator_latency_uses_runtime_measurement_not_outer_elapsed() {
    let mut config = test_config();
    config.learning.world_model.jepa.max_prediction_latency_ms = 50;
    config
        .learning
        .world_model
        .jepa
        .max_backend_prediction_latency_ms = 50;

    let (measured, cap) = predict::jepa_prediction_latency_budget(
        &config,
        archon_world_model::BackendKind::Cuda,
        12,
        250,
    );

    assert_eq!(measured, 12);
    assert_eq!(cap, 50);

    let (measured, cap) = predict::jepa_prediction_latency_budget(
        &config,
        archon_world_model::BackendKind::Cpu,
        12,
        250,
    );

    assert_eq!(measured, 250);
    assert_eq!(cap, 50);
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

/// `Eval { candidate_id: Option<String> }` used to reach a handler that just
/// errored "candidate id is required", so the optional argument was a lie and
/// the command could not be run without listing the store by hand.
#[test]
fn eval_without_an_id_scores_the_most_recent_candidate() {
    let temp = tempfile::tempdir().unwrap();
    seed_training_rows(temp.path());
    let trained = candidate::render_train(&test_config(), temp.path(), true, None).unwrap();
    let candidate_id = candidate_id_from(&trained);

    let rendered = candidate::render_eval(&test_config(), temp.path(), None).unwrap();

    assert!(rendered.contains(&format!("Candidate: {candidate_id}")));
    // The choice was implicit, so the report has to name it as such.
    assert!(rendered.contains("most recent"));
    assert!(
        temp.path()
            .join("candidates")
            .join(format!("{candidate_id}.eval.json"))
            .exists()
    );
}

/// With no candidates at all the message must point at the fix rather than
/// repeating that an id is required.
#[test]
fn eval_without_an_id_and_no_candidates_says_what_to_run() {
    let temp = tempfile::tempdir().unwrap();
    seed_training_rows(temp.path());

    let error = candidate::render_eval(&test_config(), temp.path(), None).unwrap_err();

    assert!(error.to_string().contains("no candidates to evaluate"));
    assert!(error.to_string().contains("world train --candidate"));
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

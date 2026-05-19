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

    // Check: no eval report yet → rejected before pre-promotion checks
    let missing =
        candidate::render_promote_jepa(temp.path(), &candidate_id, &config).unwrap_err();
    assert!(missing.to_string().contains("has no eval report"));

    // Build a properly-fingerprinted eval record (passes checks 1-7) but with
    // gates.passed=false so check 8 fires.
    let rows = archon_world_model::storage::WorldModelStore::open(temp.path())
        .unwrap()
        .load_rows()
        .unwrap();
    let corpus_fp =
        archon_world_model::jepa::JepaEvalPlanner::compute_corpus_fingerprint(&rows);
    let config_fp =
        candidate::compute_config_fingerprint(&config.learning.world_model.jepa);
    let schema_version = config
        .learning
        .world_model
        .jepa
        .eval_schema_version_or_default();

    // Create the representation-comparison report file required by check 4
    let comparisons_dir = temp
        .path()
        .join("jepa")
        .join("representation-comparisons");
    std::fs::create_dir_all(&comparisons_dir).unwrap();
    std::fs::write(
        comparisons_dir.join(format!("{candidate_id}.json")),
        b"{}",
    )
    .unwrap();

    let registry = archon_world_model::registry::ModelRegistry::open(temp.path()).unwrap();

    // Record that passes checks 1-7 but has gates.passed=false → check 8 rejects
    let mut failing = passing_jepa_eval_record(&candidate_id);
    failing.corpus_fingerprint = Some(corpus_fp.clone());
    failing.config_fingerprint = config_fp.clone();
    failing.eval_schema_version = schema_version;
    failing.gates.representation_baseline = false;
    failing.gates.passed = false;
    registry.write_jepa_eval_report(&failing).unwrap();
    let failed_gate =
        candidate::render_promote_jepa(temp.path(), &candidate_id, &config).unwrap_err();
    assert!(
        failed_gate.to_string().contains("has not passed"),
        "expected 'has not passed' in: {}",
        failed_gate
    );

    // Record that passes all 8 checks → promotion succeeds
    let mut passing = passing_jepa_eval_record(&candidate_id);
    passing.corpus_fingerprint = Some(corpus_fp);
    passing.config_fingerprint = config_fp;
    passing.eval_schema_version = schema_version;
    registry.write_jepa_eval_report(&passing).unwrap();
    let promoted =
        candidate::render_promote_jepa(temp.path(), &candidate_id, &config).unwrap();

    assert!(promoted.contains("Model kind: jepa_transition"));
    assert_eq!(
        registry.active_model_kind().unwrap().as_deref(),
        Some("jepa_transition")
    );
}

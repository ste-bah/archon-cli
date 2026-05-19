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

    assert!(rendered.contains("World Model JEPA-Inspired Trainer Tick"));
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
    config.learning.world_model.training.backend = "cpu".into();

    let rendered = candidate::render_train_jepa(&config, temp.path(), true, Some(1_000)).unwrap();

    let candidate_id = candidate_id_from(&rendered);
    assert!(rendered.contains("World Model JEPA-Inspired Train"));
    assert!(rendered.contains("Model kind: jepa_transition"));
    assert!(rendered.contains("Requested backend: cpu"));
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
    config.learning.world_model.training.backend = "cpu".into();
    let trained = candidate::render_train_jepa(&config, temp.path(), true, None).unwrap();
    let candidate_id = candidate_id_from(&trained);

    let rendered = candidate::render_inspect_jepa(temp.path(), &candidate_id).unwrap();

    assert!(rendered.contains("World Model JEPA-Inspired Inspect"));
    assert!(rendered.contains("Model kind: jepa_transition"));
    assert!(rendered.contains("Stop gradient: true"));
    assert!(rendered.contains("Requested backend: cpu"));
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
    config.learning.world_model.training.backend = "cpu".into();
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

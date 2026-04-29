//! GNN auto-trainer lifecycle integration test.
//!
//! Verifies: spawn → trigger → train → status → shutdown.

use std::sync::Arc;

use archon_pipeline::learning::gnn::auto_trainer::{AutoTrainer, AutoTrainerConfig};
use archon_pipeline::learning::gnn::cache::CacheConfig;
use archon_pipeline::learning::gnn::loss::TrajectoryWithFeedback;
use archon_pipeline::learning::gnn::trainer::TrainingConfig;
use archon_pipeline::learning::gnn::weights::WeightStore;
use archon_pipeline::learning::gnn::{GnnConfig, GnnEnhancer};
use archon_pipeline::learning::schema;

fn make_samples(n: usize, dim: usize) -> Vec<TrajectoryWithFeedback> {
    let mut samples = Vec::with_capacity(n);
    for i in 0..n {
        let embedding: Vec<f32> = (0..dim)
            .map(|j| ((i * 7 + j * 13) as f32).sin() * 0.5)
            .collect();
        let quality = if i % 2 == 0 { 0.9 } else { 0.1 };
        samples.push(TrajectoryWithFeedback {
            trajectory_id: format!("traj_{i}"),
            embedding,
            quality,
        });
    }
    samples
}

#[tokio::test]
async fn lifecycle_spawn_train_shutdown() {
    let db = Arc::new(cozo::DbInstance::new("mem", "", "").unwrap());
    schema::initialize_learning_schemas(&db).expect("schema init");

    let ws = Arc::new(WeightStore::new(Arc::clone(&db)));
    let gnn_cfg = GnnConfig {
        use_layer_norm: false,
        use_residual: false,
        ..GnnConfig::default()
    };
    let enhancer = Arc::new(GnnEnhancer::with_in_memory_weights(
        gnn_cfg,
        CacheConfig::default(),
        42,
    ));

    let samples = make_samples(30, 1536);
    let provider_samples = samples.clone();
    let sample_provider: Arc<dyn Fn() -> Vec<TrajectoryWithFeedback> + Send + Sync> =
        Arc::new(move || provider_samples.clone());

    let config = AutoTrainerConfig {
        enabled: true,
        min_throttle_ms: 0,
        trigger_new_memories: 5,
        trigger_corrections: 5,
        first_run_threshold: 5,
        max_runtime_ms: 30_000,
        tick_interval_ms: 100,
        ..AutoTrainerConfig::default()
    };

    let trainer = AutoTrainer::new(config);
    trainer.record_memories(10);

    let train_cfg = TrainingConfig {
        max_epochs: 3,
        max_runtime_ms: 25_000,
        ..TrainingConfig::default()
    };

    trainer.spawn(
        Arc::clone(&enhancer),
        Arc::clone(&ws),
        train_cfg,
        sample_provider,
    );

    // Wait for training to complete (poll status)
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(30);
    loop {
        let status = trainer.status();
        if status.training_count > 0 || start.elapsed() > timeout {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    trainer.shutdown();

    let status = trainer.status();
    assert!(
        status.training_count > 0,
        "Training should have run at least once, got count={}",
        status.training_count
    );
    assert!(status.last_outcome.is_some(), "Should have a last_outcome");
    let outcome = status.last_outcome.as_ref().unwrap();
    assert!(
        outcome.epochs_completed > 0,
        "Should have completed at least 1 epoch"
    );
    assert!(!outcome.final_loss.is_nan(), "Final loss should not be NaN");
    assert!(
        ws.current_version() > 0,
        "Weight version should be > 0 after training"
    );
}

#[tokio::test]
async fn disabled_auto_trainer_does_not_run() {
    let config = AutoTrainerConfig {
        enabled: false,
        trigger_new_memories: 1,
        first_run_threshold: 1,
        tick_interval_ms: 100,
        ..AutoTrainerConfig::default()
    };

    let trainer = AutoTrainer::new(config);
    let enhancer = Arc::new(GnnEnhancer::with_in_memory_weights(
        GnnConfig::default(),
        CacheConfig::default(),
        0,
    ));
    let ws = Arc::new(WeightStore::with_in_memory());
    trainer.spawn(enhancer, ws, TrainingConfig::default(), Arc::new(|| vec![]));

    let status = trainer.status();
    assert!(!status.enabled);
    assert_eq!(status.training_count, 0);

    trainer.shutdown();
}

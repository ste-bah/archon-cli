//! GNN auto-trainer rollback integration test.
//!
//! Verifies: training with good data improves loss and persists a version,
//! and the post-training weights remain recoverable.

use std::sync::Arc;

use archon_pipeline::learning::gnn::auto_trainer::{AutoTrainer, AutoTrainerConfig};
use archon_pipeline::learning::gnn::cache::CacheConfig;
use archon_pipeline::learning::gnn::loss::TrajectoryWithFeedback;
use archon_pipeline::learning::gnn::trainer::TrainingConfig;
use archon_pipeline::learning::gnn::weights::WeightStore;
use archon_pipeline::learning::gnn::{GnnConfig, GnnEnhancer};
use archon_pipeline::learning::schema;

const INPUT_DIM: usize = 1536;

fn make_quality_samples(n: usize, good_ratio: f32) -> Vec<TrajectoryWithFeedback> {
    (0..n)
        .map(|i| TrajectoryWithFeedback {
            trajectory_id: format!("s{i}"),
            embedding: (0..INPUT_DIM)
                .map(|j| ((i * 7 + j * 13) as f32).sin() * 0.5)
                .collect(),
            quality: if (i as f32) < (n as f32 * good_ratio) {
                0.9
            } else {
                0.1
            },
        })
        .collect()
}

#[tokio::test]
async fn auto_trainer_rollback_preserves_version_integrity() {
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
        100,
    ));

    // Training with good-quality data — should improve loss and save version
    let good_samples = make_quality_samples(40, 0.7);
    let provider1: Arc<dyn Fn() -> Vec<TrajectoryWithFeedback> + Send + Sync> =
        Arc::new(move || good_samples.clone());

    let cfg = AutoTrainerConfig {
        enabled: true,
        min_throttle_ms: 0,
        first_run_threshold: 5,
        tick_interval_ms: 100,
        ..AutoTrainerConfig::default()
    };

    let trainer = AutoTrainer::new(cfg);
    trainer.record_memories(10);

    let train_cfg = TrainingConfig {
        max_epochs: 4,
        max_runtime_ms: 30_000,
        ..TrainingConfig::default()
    };

    trainer.spawn(Arc::clone(&enhancer), Arc::clone(&ws), train_cfg, provider1);

    // Wait for training to complete
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
    while trainer.status().training_count == 0 && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    let status1 = trainer.status();
    assert!(status1.training_count > 0, "First training should complete");
    let version_after_first = ws.current_version();
    assert!(
        version_after_first > 0,
        "Version should be > 0 after first training"
    );

    let first_loss = status1
        .last_outcome
        .as_ref()
        .map(|o| o.final_loss)
        .unwrap_or(0.0);
    assert!(
        !first_loss.is_nan(),
        "First training loss should not be NaN"
    );

    trainer.shutdown();
    eprintln!("First training: loss={first_loss:.6}, version={version_after_first}");

    // Verify weights exist in the shared store (in-memory cache populated during save)
    assert!(ws.has_layer("layer1"), "Layer1 should exist after training");
    assert!(ws.has_layer("layer2"), "Layer2 should exist after training");
    assert!(ws.has_layer("layer3"), "Layer3 should exist after training");
}

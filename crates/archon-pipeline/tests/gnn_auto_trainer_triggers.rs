//! GNN auto-trainer trigger integration tests.
//!
//! Verifies each trigger path end-to-end: first run, memory accumulation,
//! correction spike. Runs under tokio runtime.

use std::sync::Arc;
use std::time::{Duration, Instant};

use archon_pipeline::learning::gnn::auto_trainer::{AutoTrainer, AutoTrainerConfig};
use archon_pipeline::learning::gnn::cache::CacheConfig;
use archon_pipeline::learning::gnn::loss::TrajectoryWithFeedback;
use archon_pipeline::learning::gnn::trainer::TrainingConfig;
use archon_pipeline::learning::gnn::weights::WeightStore;
use archon_pipeline::learning::gnn::{GnnConfig, GnnEnhancer};
use archon_pipeline::learning::schema;

const INPUT_DIM: usize = 1536;

fn make_samples(n: usize) -> Vec<TrajectoryWithFeedback> {
    (0..n)
        .map(|i| TrajectoryWithFeedback {
            trajectory_id: format!("t{i}"),
            embedding: (0..INPUT_DIM)
                .map(|j| ((i * 7 + j * 13) as f32).sin() * 0.5)
                .collect(),
            quality: if i % 2 == 0 { 0.9 } else { 0.1 },
        })
        .collect()
}

fn setup() -> (Arc<GnnEnhancer>, Arc<WeightStore>) {
    let db = Arc::new(cozo::DbInstance::new("mem", "", "").unwrap());
    schema::initialize_learning_schemas(&db).expect("schema init");
    let ws = Arc::new(WeightStore::new(db));
    let enhancer = Arc::new(GnnEnhancer::with_in_memory_weights(
        GnnConfig {
            use_layer_norm: false,
            use_residual: false,
            ..GnnConfig::default()
        },
        CacheConfig::default(),
        42,
    ));
    (enhancer, ws)
}

fn short_train_cfg() -> TrainingConfig {
    TrainingConfig {
        max_epochs: 2,
        max_runtime_ms: 20_000,
        ..TrainingConfig::default()
    }
}

#[tokio::test]
async fn first_run_triggers_when_memories_exceed_threshold() {
    let (enhancer, ws) = setup();
    let samples = make_samples(15);
    let provider: Arc<dyn Fn() -> Vec<TrajectoryWithFeedback> + Send + Sync> =
        Arc::new(move || samples.clone());

    let cfg = AutoTrainerConfig {
        enabled: true,
        min_throttle_ms: 0,
        first_run_threshold: 10,
        tick_interval_ms: 50,
        ..AutoTrainerConfig::default()
    };

    let trainer = AutoTrainer::new(cfg);
    trainer.record_memories(12);

    trainer.spawn(enhancer, ws, short_train_cfg(), provider);

    let deadline = Instant::now() + Duration::from_secs(20);
    while trainer.status().training_count == 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let status = trainer.status();
    trainer.shutdown();
    assert!(status.training_count > 0, "First run should have triggered");
}

#[tokio::test]
async fn memory_accumulation_triggers_after_first_run() {
    let (enhancer, ws) = setup();
    let samples = make_samples(20);
    let provider: Arc<dyn Fn() -> Vec<TrajectoryWithFeedback> + Send + Sync> =
        Arc::new(move || samples.clone());

    let cfg = AutoTrainerConfig {
        enabled: true,
        min_throttle_ms: 0,
        first_run_threshold: 5,
        trigger_new_memories: 15,
        tick_interval_ms: 100,
        ..AutoTrainerConfig::default()
    };

    let trainer = AutoTrainer::new(cfg);
    trainer.record_memories(10);

    trainer.spawn(
        Arc::clone(&enhancer),
        Arc::clone(&ws),
        short_train_cfg(),
        provider,
    );

    let deadline = Instant::now() + Duration::from_secs(30);
    while trainer.status().training_count == 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(
        trainer.status().training_count > 0,
        "First run should trigger"
    );

    // Wait for training_in_progress to clear so memories_at_last_train
    // is updated before we record more memories.
    while trainer.status().training_in_progress && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Accumulate more memories — should trigger second run
    trainer.record_memories(20);

    while trainer.status().training_count < 2 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    trainer.shutdown();
    assert!(
        trainer.status().training_count >= 2,
        "Memory accumulation should trigger second run"
    );
}

#[tokio::test]
async fn correction_trigger_fires() {
    let (enhancer, ws) = setup();
    let samples = make_samples(15);
    let provider: Arc<dyn Fn() -> Vec<TrajectoryWithFeedback> + Send + Sync> =
        Arc::new(move || samples.clone());

    let cfg = AutoTrainerConfig {
        enabled: true,
        min_throttle_ms: 0,
        trigger_corrections: 8,
        first_run_threshold: 100,
        trigger_new_memories: 100,
        tick_interval_ms: 100,
        ..AutoTrainerConfig::default()
    };

    let trainer = AutoTrainer::new(cfg);
    trainer.record_memories(5);

    trainer.spawn(
        Arc::clone(&enhancer),
        Arc::clone(&ws),
        short_train_cfg(),
        provider,
    );

    // No training should fire yet
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        trainer.status().training_count,
        0,
        "Should not trigger on memories alone"
    );

    // Trigger via corrections
    trainer.record_corrections(10);

    let deadline = Instant::now() + Duration::from_secs(15);
    while trainer.status().training_count == 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let status = trainer.status();
    trainer.shutdown();
    assert!(
        status.training_count > 0,
        "Corrections should trigger training"
    );
}

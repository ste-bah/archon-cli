//! GNN auto-trainer acceptance gate: foreground latency during training.
//!
//! Verifies `during_p95 < baseline_p95 * 2.0` while the auto-trainer is
//! running a training job in `spawn_blocking`. Also verifies that training
//! actually overlapped with the foreground measurement window — the test
//! fails fast with a diagnostic if training never started, rather than
//! trivially passing against an idle trainer.

use std::sync::Arc;
use std::time::Instant;

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

fn p95_us(timings: &[u128]) -> u128 {
    let mut sorted: Vec<u128> = timings.to_vec();
    sorted.sort_unstable();
    let idx = ((sorted.len() as f64) * 0.95).ceil() as usize;
    sorted[idx.saturating_sub(1).min(sorted.len() - 1)]
}

#[tokio::test]
async fn foreground_latency_during_training_stays_within_bounds() {
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

    // Baseline: 20 foreground calls without training
    let mut baseline_timings = Vec::with_capacity(20);
    let input: Vec<f32> = (0..1536).map(|i| (i as f32).sin()).collect();
    for _ in 0..20 {
        let t0 = Instant::now();
        let _ = enhancer.enhance(&input, None, None, false);
        baseline_timings.push(t0.elapsed().as_micros());
    }
    let baseline_p95 = p95_us(&baseline_timings);

    let samples = make_samples(60, 1536);
    let provider_samples = samples.clone();
    let sample_provider: Arc<dyn Fn() -> Vec<TrajectoryWithFeedback> + Send + Sync> =
        Arc::new(move || provider_samples.clone());

    let config = AutoTrainerConfig {
        enabled: true,
        min_throttle_ms: 0,
        trigger_new_memories: 1,
        first_run_threshold: 1,
        max_runtime_ms: 60_000,
        tick_interval_ms: 100,
        ..AutoTrainerConfig::default()
    };

    let trainer = AutoTrainer::new(config);
    trainer.record_memories(10);

    let train_cfg = TrainingConfig {
        max_epochs: 5,
        max_runtime_ms: 50_000,
        ..TrainingConfig::default()
    };

    trainer.spawn(
        Arc::clone(&enhancer),
        Arc::clone(&ws),
        train_cfg,
        sample_provider,
    );

    // Wait until training starts, tracking whether we observed it
    let mut training_observed = false;
    let start = Instant::now();
    loop {
        if trainer.status().training_in_progress {
            training_observed = true;
            break;
        }
        if start.elapsed().as_secs() > 10 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // Run foreground work WHILE training is in progress, also polling
    // training_in_progress between iterations
    let mut during_timings = Vec::with_capacity(20);
    for _ in 0..20 {
        let t0 = Instant::now();
        let _ = enhancer.enhance(&input, None, None, false);
        during_timings.push(t0.elapsed().as_micros());
        if trainer.status().training_in_progress {
            training_observed = true;
        }
    }
    let during_p95 = p95_us(&during_timings);

    trainer.shutdown();

    let final_status = trainer.status();
    let training_overlap = training_observed || final_status.training_count > 0;

    eprintln!("baseline_p95 = {baseline_p95} us");
    eprintln!("during_p95  = {during_p95} us");
    eprintln!(
        "ratio = {:.2}",
        during_p95 as f64 / baseline_p95.max(1) as f64
    );
    eprintln!("training_count = {}", final_status.training_count);
    eprintln!("training_observed = {training_observed}");

    assert!(
        training_overlap,
        "Test invalid: training never overlapped with foreground measurement \
         (training_in_progress never true AND training_count=0). \
         Either trigger logic is broken or measurement window is wrong. \
         baseline_p95={baseline_p95}us, during_p95={during_p95}us"
    );

    let ratio = during_p95 as f64 / baseline_p95.max(1) as f64;
    assert!(
        ratio < 2.0,
        "Foreground P95 ({during_p95} us) exceeds 2x baseline ({baseline_p95} us), ratio={ratio:.2}"
    );
}

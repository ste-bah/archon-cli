//! GNN rollback — NaN guard safety and weight version integrity.
//!
//! Verifies:
//! - NaN guard prevents weight corruption under extreme learning rates
//! - Weight version persists correctly through save/load cycles
//! - Post-training weights remain finite

use std::sync::Arc;

use archon_pipeline::learning::gnn::cache::CacheConfig;
use archon_pipeline::learning::gnn::loss::TrajectoryWithFeedback;
use archon_pipeline::learning::gnn::trainer::{GnnTrainer, TrainingConfig};
use archon_pipeline::learning::gnn::weights::{Initialization, WeightStore};
use archon_pipeline::learning::gnn::{GnnConfig, GnnEnhancer};
use archon_pipeline::learning::schema;

// The enhancer hardcodes intermediate_dims:
//   intermediate_dim1 = 1536 * 2 / 3 = 1024
//   intermediate_dim2 = 1536 * 5 / 6 = 1280
// Test weights MUST match these dimensions.
const H1: usize = 1024;
const H2: usize = 1280;

fn make_samples(n: usize) -> Vec<TrajectoryWithFeedback> {
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let embedding: Vec<f32> = (0..16).map(|j| (i * 17 + j * 13) as f32 * 0.1).collect();
        let quality = if i % 2 == 0 { 0.9 } else { 0.1 };
        out.push(TrajectoryWithFeedback {
            trajectory_id: format!("s{i}"),
            embedding,
            quality,
        });
    }
    out
}

fn mem_db() -> cozo::DbInstance {
    cozo::DbInstance::new("mem", "", "").unwrap()
}

#[test]
fn nan_guard_keeps_weights_finite_under_extreme_lr() {
    let db = Arc::new(mem_db());
    schema::initialize_learning_schemas(&db).expect("schema init");

    let dim: usize = 16;

    let store = WeightStore::new(Arc::clone(&db));
    store.initialize("layer1", dim, H1, Initialization::He, 100);
    store.initialize("layer2", H1, H2, Initialization::He, 101);
    store.initialize("layer3", H2, dim, Initialization::He, 102);
    let v_before = store.save_all().expect("save initial weights");

    let mut gnn_cfg = GnnConfig::default();
    gnn_cfg.input_dim = dim;
    gnn_cfg.output_dim = dim;

    let enhancer = GnnEnhancer::with_in_memory_weights(gnn_cfg, CacheConfig::default(), 100);
    // Override enhancer weights from CozoDB store
    {
        let l1_w = store.get_weights("layer1");
        let l1_b = store.get_bias("layer1");
        let l2_w = store.get_weights("layer2");
        let l2_b = store.get_bias("layer2");
        let l3_w = store.get_weights("layer3");
        let l3_b = store.get_bias("layer3");
        enhancer.set_weights(
            archon_pipeline::learning::gnn::LayerWeights {
                w: (*l1_w).clone(),
                bias: (*l1_b).clone(),
            },
            archon_pipeline::learning::gnn::LayerWeights {
                w: (*l2_w).clone(),
                bias: (*l2_b).clone(),
            },
            archon_pipeline::learning::gnn::LayerWeights {
                w: (*l3_w).clone(),
                bias: (*l3_b).clone(),
            },
        );
    }

    let test_input: Vec<f32> = vec![0.5; dim];
    let pre = enhancer.enhance(&test_input, None, None, false);
    assert!(
        pre.enhanced.iter().all(|v| v.is_finite()),
        "pre-training must be finite"
    );

    let samples = make_samples(30);
    let training_cfg = TrainingConfig {
        learning_rate: 1e10,
        max_epochs: 2,
        batch_size: 8,
        max_triplets_per_run: 32,
        max_runtime_ms: 30_000,
        early_stopping_patience: 10,
        validation_split: 0.2,
        ..TrainingConfig::default()
    };

    let mut trainer = GnnTrainer::new(training_cfg, Some(Arc::new(store)));
    let _outcome = trainer.train(&enhancer, &samples, None);

    // Core safety property: training must not crash even with extreme lr.
    // The NaN guard in optimizer + forward_pass prevents weight corruption.
    // Outcome may be rollback, cancellation, or just no improvement — all are safe.

    // Verify weights are still finite post-training
    let (l1, l2, l3) = enhancer.get_weights();
    for (name, layer) in [("layer1", &l1), ("layer2", &l2), ("layer3", &l3)] {
        for (i, row) in layer.w.iter().enumerate() {
            for (j, &v) in row.iter().enumerate() {
                assert!(
                    v.is_finite(),
                    "{name}[{i}][{j}] = {v} is not finite after extreme-lr training"
                );
            }
        }
        for (i, &b) in layer.bias.iter().enumerate() {
            assert!(b.is_finite(), "{name} bias[{i}] = {b} is not finite");
        }
    }

    // Version should be preserved (either rollback restored, or no save occurred)
    let store2 = WeightStore::new(Arc::clone(&db));
    assert!(store2.current_version() >= v_before);
    store2
        .load_version(v_before)
        .expect("can still load pre-training version");
}

#[test]
fn cozodb_weight_version_survives_training() {
    let db = Arc::new(mem_db());
    schema::initialize_learning_schemas(&db).expect("schema init");

    let dim: usize = 16;

    let store = WeightStore::new(Arc::clone(&db));
    store.initialize("layer1", dim, H1, Initialization::He, 200);
    store.initialize("layer2", H1, H2, Initialization::He, 201);
    store.initialize("layer3", H2, dim, Initialization::He, 202);
    let v_before = store.save_all().expect("save");

    let mut gnn_cfg = GnnConfig::default();
    gnn_cfg.input_dim = dim;
    gnn_cfg.output_dim = dim;
    let enhancer = GnnEnhancer::with_in_memory_weights(gnn_cfg, CacheConfig::default(), 200);

    // Override enhancer weights from CozoDB
    {
        let l1_w = store.get_weights("layer1");
        let l1_b = store.get_bias("layer1");
        let l2_w = store.get_weights("layer2");
        let l2_b = store.get_bias("layer2");
        let l3_w = store.get_weights("layer3");
        let l3_b = store.get_bias("layer3");
        enhancer.set_weights(
            archon_pipeline::learning::gnn::LayerWeights {
                w: (*l1_w).clone(),
                bias: (*l1_b).clone(),
            },
            archon_pipeline::learning::gnn::LayerWeights {
                w: (*l2_w).clone(),
                bias: (*l2_b).clone(),
            },
            archon_pipeline::learning::gnn::LayerWeights {
                w: (*l3_w).clone(),
                bias: (*l3_b).clone(),
            },
        );
    }

    let samples = make_samples(30);

    // Use a normal learning rate — this tests the persistence flow, not NaN
    let training_cfg = TrainingConfig {
        learning_rate: 0.001,
        max_epochs: 1,
        batch_size: 4,
        max_triplets_per_run: 16,
        max_runtime_ms: 30_000,
        ..TrainingConfig::default()
    };

    let mut trainer = GnnTrainer::new(training_cfg, Some(Arc::new(store)));
    let outcome = trainer.train(&enhancer, &samples, None);

    // Training must complete without panic
    assert!(outcome.epochs_completed >= 1);

    // A fresh store can still load the pre-training version
    let store2 = WeightStore::new(Arc::clone(&db));
    store2
        .load_version(v_before)
        .expect("pre-training version must be loadable");
    assert!(store2.has_layer("layer1"));
}

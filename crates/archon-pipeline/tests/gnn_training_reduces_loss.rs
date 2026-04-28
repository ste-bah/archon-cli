//! GNN training reduces loss — PR 2 acceptance gate (cannot be gamed).
//!
//! Three independent properties, ALL must hold:
//! 1. Loss reduction: final train loss < initial * 0.8 (>=20% reduction)
//! 2. No NaN/Inf: every epoch, every layer has finite positive weight norm
//! 3. No overfit divergence: val_loss <= train_loss * 1.5 for >=70% of epochs

use archon_pipeline::learning::gnn::loss::TrajectoryWithFeedback;
use archon_pipeline::learning::gnn::trainer::{GnnTrainer, TrainingConfig};
use archon_pipeline::learning::gnn::cache::CacheConfig;
use archon_pipeline::learning::gnn::{GnnConfig, GnnEnhancer};

const DIM: usize = 16;

/// Generate samples with random embeddings but structured quality.
/// Quality labels encode hidden structure the GNN must learn:
/// samples with quality >= 0.75 should map close together,
/// samples with quality <= 0.25 should map far from high-quality ones.
fn make_samples(n: usize) -> Vec<TrajectoryWithFeedback> {
    // Use a simple deterministic pseudo-random with enough diversity
    let mut out = Vec::with_capacity(n);
    let mut s: u64 = 12345;
    for i in 0..n {
        let embedding: Vec<f32> = (0..DIM)
            .map(|_| {
                s = s.wrapping_mul(1103515245).wrapping_add(12345);
                let bits = ((s >> 16) as u32) & 0x7fffff;
                bits as f32 / 8_388_607.0 * 2.0 - 1.0
            })
            .collect();
        // Quality based on index parity — creates learnable structure
        let quality = if i % 2 == 0 { 0.9 } else { 0.1 };
        out.push(TrajectoryWithFeedback {
            trajectory_id: format!("s{i}"),
            embedding,
            quality,
        });
    }
    out
}

#[test]
fn training_reduces_loss_and_passes_three_property_gate() {
    let mut gnn_cfg = GnnConfig::default();
    gnn_cfg.input_dim = DIM;
    gnn_cfg.output_dim = DIM;
    // Disable layer norm to prevent mode collapse from excessive normalization
    gnn_cfg.use_layer_norm = false;
    gnn_cfg.use_residual = false;
    let enhancer = GnnEnhancer::with_in_memory_weights(gnn_cfg, CacheConfig::default(), 42);

    let samples = make_samples(200);

    let training_cfg = TrainingConfig {
        max_epochs: 20,
        batch_size: 32,
        learning_rate: 0.005,
        max_triplets_per_run: 256,
        max_runtime_ms: 120_000,
        early_stopping_patience: 10,
        validation_split: 0.2,
        ..TrainingConfig::default()
    };

    let mut trainer = GnnTrainer::new(training_cfg, None);
    let outcome = trainer.train(&enhancer, &samples, None);

    eprintln!(
        "Training outcome: epochs={}, initial_loss={:.6}, final_loss={:.6}, best={:.6}, val={:?}",
        outcome.epochs_completed,
        outcome.initial_loss,
        outcome.final_loss,
        outcome.best_loss,
        outcome.validation_loss,
    );
    for m in &outcome.epoch_metrics {
        eprintln!(
            "  epoch {}: train={:.6} val={:?}",
            m.epoch, m.train_loss, m.val_loss,
        );
    }

    // Must have completed at least 1 epoch
    assert!(outcome.epochs_completed >= 1, "must complete at least 1 epoch");

    // ---------------------------------------------------------------
    // Property 1: Loss reduction >= 20%
    // ---------------------------------------------------------------
    assert!(
        outcome.initial_loss > 0.0,
        "initial loss must be > 0 (got {})",
        outcome.initial_loss
    );
    let reduction_ratio = outcome.final_loss / outcome.initial_loss;
    assert!(
        reduction_ratio < 0.8,
        "loss must reduce >=20%: initial={:.6}, final={:.6}, ratio={:.4}",
        outcome.initial_loss,
        outcome.final_loss,
        reduction_ratio,
    );

    // ---------------------------------------------------------------
    // Property 2: No NaN/Inf across entire run
    // ---------------------------------------------------------------
    let metrics = &outcome.epoch_metrics;
    assert!(!metrics.is_empty(), "must have per-epoch metrics");

    for m in metrics {
        for (layer_id, norm, has_nan) in &m.layer_norms {
            assert!(!has_nan, "epoch {} layer {layer_id}: has NaN/Inf", m.epoch);
            assert!(
                norm.is_finite() && *norm > 0.0,
                "epoch {} layer {layer_id}: weight_norm={norm} (must be finite > 0)",
                m.epoch,
            );
        }
        assert!(
            m.train_loss.is_finite() && m.train_loss >= 0.0,
            "epoch {}: train_loss={} must be finite >= 0",
            m.epoch,
            m.train_loss,
        );
    }

    // ---------------------------------------------------------------
    // Property 3: No overfit divergence (>=70% of epochs pass)
    // ---------------------------------------------------------------
    let valid_epochs: Vec<_> = metrics
        .iter()
        .filter(|m| m.val_loss.is_some())
        .collect();
    assert!(
        valid_epochs.len() >= 3,
        "need at least 3 epochs with validation loss, got {}",
        valid_epochs.len(),
    );

    let ok_count = valid_epochs
        .iter()
        .filter(|m| {
            let val = m.val_loss.unwrap();
            val <= m.train_loss * 1.5
        })
        .count();
    let required = (valid_epochs.len() as f32 * 0.7).ceil() as usize;

    assert!(
        ok_count >= required,
        "overfit divergence: only {ok_count}/{n} epochs satisfy val_loss <= 1.5x train_loss (need {required})",
        n = valid_epochs.len(),
    );
}

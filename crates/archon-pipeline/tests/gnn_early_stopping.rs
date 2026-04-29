//! Verifies early stopping patience kicks in and best weights are restored.
//!
//! v0.1.27 hygiene — closes PR 2 audit finding that early_stopping_patience
//! was defined in TrainingConfig but ignored by the epoch loop.

use archon_pipeline::learning::gnn::cache::CacheConfig;
use archon_pipeline::learning::gnn::loss::TrajectoryWithFeedback;
use archon_pipeline::learning::gnn::trainer::{GnnTrainer, TrainingConfig, TrainingOutcome};
use archon_pipeline::learning::gnn::{GnnConfig, GnnEnhancer};

fn make_samples(n: usize, dim: usize, label_noise: bool) -> Vec<TrajectoryWithFeedback> {
    let mut samples = Vec::with_capacity(n);
    for i in 0..n {
        let embedding: Vec<f32> = (0..dim)
            .map(|j| ((i.wrapping_mul(7).wrapping_add(j * 13)) as f32).sin() * 0.5)
            .collect();
        // With label_noise, half the samples get random labels → hard to fit,
        // causing validation loss to plateau after an early minimum
        let quality = if label_noise && i % 3 == 0 {
            // Noisy label — prevents clean convergence
            if (i / 3) % 2 == 0 { 0.15 } else { 0.85 }
        } else if i % 2 == 0 {
            0.9
        } else {
            0.1
        };
        samples.push(TrajectoryWithFeedback {
            trajectory_id: format!("traj_{i}"),
            embedding,
            quality,
        });
    }
    samples
}

fn make_enhancer() -> GnnEnhancer {
    GnnEnhancer::with_in_memory_weights(
        GnnConfig {
            use_layer_norm: false,
            use_residual: false,
            ..GnnConfig::default()
        },
        CacheConfig::default(),
        42,
    )
}

fn train(
    cfg: TrainingConfig,
    samples: &[TrajectoryWithFeedback],
) -> (GnnEnhancer, TrainingOutcome) {
    let enhancer = make_enhancer();
    let mut trainer = GnnTrainer::new(cfg, None);
    let outcome = trainer.train(&enhancer, samples, None);
    (enhancer, outcome)
}

#[test]
fn training_stops_early_when_validation_plateaus() {
    // 32 synthetic samples with label noise, 20 max epochs, patience=3.
    // High learning rate causes fast initial fit then overfit → plateau.
    let samples = make_samples(32, 16, true);

    let cfg = TrainingConfig {
        max_epochs: 20,
        early_stopping_patience: 3,
        validation_split: 0.25,
        learning_rate: 0.05,
        batch_size: 8,
        max_triplets_per_run: 64,
        max_runtime_ms: 30_000,
        min_improvement: 0.0005,
        ..TrainingConfig::default()
    };

    let (_enhancer, outcome) = train(cfg, &samples);

    assert!(
        outcome.stopped_early,
        "Expected early stopping to fire (patience=3, epochs_completed={}, best_epoch={})",
        outcome.epochs_completed, outcome.best_epoch,
    );
    assert!(
        outcome.best_epoch < outcome.epochs_completed,
        "best_epoch ({}) should be before epochs_completed ({})",
        outcome.best_epoch,
        outcome.epochs_completed,
    );
    assert!(
        outcome.epochs_completed - outcome.best_epoch <= 4,
        "gap between best_epoch ({}) and epochs_completed ({}) should be <= patience+1 (4)",
        outcome.best_epoch,
        outcome.epochs_completed,
    );
    assert!(outcome.best_val_loss > 0.0);
    assert!(!outcome.final_loss.is_nan());
    assert!(!outcome.best_loss.is_nan());
}

#[test]
fn early_stopping_restores_best_weights() {
    // Train with early stopping enabled. After training completes,
    // verify final_loss is close to best_train_loss — proving weights were
    // restored to best-epoch state, not left at the degraded final state.
    let samples = make_samples(32, 16, true);

    let cfg = TrainingConfig {
        max_epochs: 20,
        early_stopping_patience: 3,
        validation_split: 0.25,
        learning_rate: 0.05,
        batch_size: 8,
        max_triplets_per_run: 64,
        max_runtime_ms: 30_000,
        min_improvement: 0.0005,
        ..TrainingConfig::default()
    };

    let min_improvement = cfg.min_improvement;
    let (enhancer, outcome) = train(cfg, &samples);

    assert!(outcome.stopped_early);

    // Compare final_loss against best_train_loss (not best_val_loss) since
    // final_loss is computed on training triplets. If weights were restored,
    // these should be very close.
    let loss_gap = (outcome.final_loss - outcome.best_train_loss).abs();
    assert!(
        loss_gap < min_improvement * 5.0,
        "Weight restoration failed: final_loss ({:.6}) diverges from best_train_loss ({:.6}), gap={:.6}",
        outcome.final_loss,
        outcome.best_train_loss,
        loss_gap,
    );

    // Verify weights are sane (no NaN/Inf after restore)
    let (l1, l2, l3) = enhancer.get_weights();
    for (name, lw) in [("layer1", &l1), ("layer2", &l2), ("layer3", &l3)] {
        for row in &lw.w {
            for &v in row {
                assert!(
                    !v.is_nan() && !v.is_infinite(),
                    "NaN/Inf in {name} weights after early-stop restore"
                );
            }
        }
        for &v in &lw.bias {
            assert!(
                !v.is_nan() && !v.is_infinite(),
                "NaN/Inf in {name} bias after early-stop restore"
            );
        }
    }
}

#[test]
fn early_stopping_disabled_when_patience_zero() {
    // patience = 0 means feature disabled — trainer runs all max_epochs
    // even when validation loss regresses.
    let samples = make_samples(32, 16, true);

    let cfg = TrainingConfig {
        max_epochs: 5,
        early_stopping_patience: 0,
        validation_split: 0.25,
        learning_rate: 0.05,
        batch_size: 8,
        max_triplets_per_run: 64,
        max_runtime_ms: 30_000,
        ..TrainingConfig::default()
    };

    let max_epochs_expected = cfg.max_epochs;
    let (_enhancer, outcome) = train(cfg, &samples);

    assert!(
        !outcome.stopped_early,
        "patience=0 should disable early stopping"
    );
    assert_eq!(
        outcome.epochs_completed, max_epochs_expected,
        "patience=0 should run all {} epochs, got {}",
        max_epochs_expected, outcome.epochs_completed,
    );
}

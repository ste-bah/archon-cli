use std::sync::Arc;

use super::super::LayerWeights;
use super::super::ewc::EwcRegularizer;
use super::super::loss::ContrastiveLossConfig;
use super::super::optimizer::{AdamConfig, AdamOptimizer};
use super::super::triplets_loss::TripletLossConfig;
use super::super::weights::WeightStore;

/// Configuration for a synchronous training run.
#[derive(Debug, Clone)]
pub struct TrainingConfig {
    pub learning_rate: f32,
    pub batch_size: usize,
    pub max_epochs: usize,
    pub early_stopping_patience: usize,
    pub validation_split: f32,
    pub ewc_lambda: f32,
    pub margin: f32,
    pub triplet_loss_coefficient: f32,
    pub max_gradient_norm: f32,
    pub shuffle: bool,
    pub min_improvement: f32,
    pub max_triplets_per_run: usize,
    pub max_runtime_ms: u64,
}

impl Default for TrainingConfig {
    fn default() -> Self {
        Self {
            learning_rate: 0.001,
            batch_size: 32,
            max_epochs: 10,
            early_stopping_patience: 3,
            validation_split: 0.2,
            ewc_lambda: 0.1,
            margin: 0.5,
            triplet_loss_coefficient: 0.1,
            max_gradient_norm: 1.0,
            shuffle: true,
            min_improvement: 0.001,
            max_triplets_per_run: 256,
            max_runtime_ms: 300_000,
        }
    }
}

// ---------------------------------------------------------------------------
// Per-epoch metrics (public — consumed by integration tests)
// ---------------------------------------------------------------------------

/// Per-epoch metrics captured during training.
///
/// Used by the `gnn_training_reduces_loss` acceptance gate to verify
/// NaN-free training, loss reduction, and no overfit divergence.
#[derive(Debug, Clone)]
pub struct EpochMetrics {
    pub epoch: usize,
    pub train_loss: f32,
    pub val_loss: Option<f32>,
    pub loss_quality: f32,
    pub loss_ewc: f32,
    pub loss_triplet: f32,
    /// (layer_id, l2_norm_of_weights, has_nan_or_inf)
    pub layer_norms: Vec<(String, f32, bool)>,
}

// ---------------------------------------------------------------------------
// TrainingOutcome
// ---------------------------------------------------------------------------

/// Result of a completed training run.
#[derive(Debug, Clone)]
pub struct TrainingOutcome {
    pub epochs_completed: usize,
    pub batches_processed: usize,
    pub samples_processed: usize,
    pub initial_loss: f32,
    pub final_loss: f32,
    pub best_loss: f32,
    pub validation_loss: Option<f32>,
    pub stopped_early: bool,
    pub timed_out: bool,
    pub cancelled: bool,
    /// Epoch that produced best_val_loss (0-indexed).
    pub best_epoch: usize,
    /// Best validation loss observed across all epochs.
    pub best_val_loss: f32,
    /// Training loss at best_epoch (for weight-restoration verification).
    pub best_train_loss: f32,
    /// Per-epoch metrics collected during training.
    pub epoch_metrics: Vec<EpochMetrics>,
}

// ---------------------------------------------------------------------------
// Internal batch result
// ---------------------------------------------------------------------------

pub(super) struct BatchResult {
    pub(super) loss: f32,
    pub(super) grads: Vec<(Vec<Vec<f32>>, Vec<f32>)>,
}

/// Internal epoch return - train_loss + optional val_loss.
pub(super) struct EpochResult {
    pub(super) train_loss: f32,
    pub(super) val_loss: Option<f32>,
    pub(super) loss_quality: f32,
    pub(super) loss_ewc: f32,
    pub(super) loss_triplet: f32,
}

/// Synchronous GNN trainer.
///
/// Runs forward → loss → backward → Adam step in a standard loop.
/// Cancellation and timeout are checked at batch boundaries.
pub struct GnnTrainer {
    pub(super) config: TrainingConfig,
    pub(super) optimizer: AdamOptimizer,
    pub(super) ewc: EwcRegularizer,
    pub(super) loss_config: ContrastiveLossConfig,
    pub(super) triplet_loss_config: TripletLossConfig,
    pub(super) weight_store: Option<Arc<WeightStore>>,
}

impl GnnTrainer {
    /// Create a new trainer.
    ///
    /// `weight_store` is optional — when provided, weights are persisted after
    /// each successful training run (loss decreased, no NaN).
    pub fn new(config: TrainingConfig, weight_store: Option<Arc<WeightStore>>) -> Self {
        let adam_config = AdamConfig {
            learning_rate: config.learning_rate,
            ..AdamConfig::default()
        };
        let dummy: Vec<&LayerWeights> = vec![];
        let optimizer = AdamOptimizer::new(adam_config, &dummy);

        let ewc = EwcRegularizer::new(config.ewc_lambda);
        let loss_config = ContrastiveLossConfig {
            margin: config.margin,
            ..ContrastiveLossConfig::default()
        };
        let triplet_loss_config = TripletLossConfig::default();

        Self {
            config,
            optimizer,
            ewc,
            loss_config,
            triplet_loss_config,
            weight_store,
        }
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    pub fn config(&self) -> &TrainingConfig {
        &self.config
    }

    pub fn learning_rate(&self) -> f32 {
        self.optimizer.learning_rate()
    }

    pub fn set_learning_rate(&mut self, lr: f32) {
        self.optimizer.set_learning_rate(lr);
    }

    pub fn ewc(&self) -> &EwcRegularizer {
        &self.ewc
    }
}

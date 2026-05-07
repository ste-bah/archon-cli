//! Synchronous GNN training loop with Adam, EWC, early stopping, and timeout.
//!
//! PR 2 implementation — single-threaded, synchronous training. PR 3 wraps
//! this in a tokio background task.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tracing::{info, warn};

use super::backprop;
use super::ewc::EwcRegularizer;
use super::loss::{self, ContrastiveLossConfig, TrajectoryWithFeedback};
use super::math::ActivationType;
use super::optimizer::{AdamConfig, AdamOptimizer};
use super::triplets_loss::{self, TripletBatch, TripletLossConfig};
use super::weights::WeightStore;
use super::{GnnEnhancer, LayerWeights};

// ---------------------------------------------------------------------------
// TrainingConfig
// ---------------------------------------------------------------------------

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

struct BatchResult {
    loss: f32,
    grads: Vec<(Vec<Vec<f32>>, Vec<f32>)>,
}

/// Internal epoch return — train_loss + optional val_loss.
struct EpochResult {
    train_loss: f32,
    val_loss: Option<f32>,
    loss_quality: f32,
    loss_ewc: f32,
    loss_triplet: f32,
}

// ---------------------------------------------------------------------------
// GnnTrainer
// ---------------------------------------------------------------------------

/// Synchronous GNN trainer.
///
/// Runs forward → loss → backward → Adam step in a standard loop.
/// Cancellation and timeout are checked at batch boundaries.
pub struct GnnTrainer {
    config: TrainingConfig,
    optimizer: AdamOptimizer,
    ewc: EwcRegularizer,
    loss_config: ContrastiveLossConfig,
    triplet_loss_config: TripletLossConfig,
    weight_store: Option<Arc<WeightStore>>,
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

    /// Run the full training loop.
    ///
    /// `cancel` — optional atomic bool checked at batch boundaries.
    pub fn train(
        &mut self,
        enhancer: &GnnEnhancer,
        samples: &[TrajectoryWithFeedback],
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> TrainingOutcome {
        self.train_with_triplets(enhancer, samples, &TripletBatch::default(), cancel)
    }

    /// Run the full training loop with optional hydrated meaning triplets.
    ///
    /// The empty-triplet path is identical to [`Self::train`]. Hydrated
    /// triplets add a conservative auxiliary metric-learning term.
    pub fn train_with_triplets(
        &mut self,
        enhancer: &GnnEnhancer,
        samples: &[TrajectoryWithFeedback],
        triplet_batch: &TripletBatch,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> TrainingOutcome {
        let start_time = Instant::now();
        let mut samples_processed = 0usize;
        let mut batches_processed = 0usize;
        let mut timed_out = false;
        let mut cancelled = false;
        let mut epoch_metrics: Vec<EpochMetrics> = Vec::new();

        // Build triplets (indices into samples)
        let triplets = loss::build_triplets(samples, &self.loss_config);
        if triplets.len() < 2 && triplet_batch.triplets.is_empty() {
            warn!(
                "Not enough triplets for training (trajectory={}, meaning={})",
                triplets.len(),
                triplet_batch.triplets.len()
            );
            return TrainingOutcome {
                epochs_completed: 0,
                batches_processed: 0,
                samples_processed: 0,
                initial_loss: 0.0,
                final_loss: 0.0,
                best_loss: 0.0,
                validation_loss: None,
                stopped_early: false,
                timed_out: false,
                cancelled: false,
                best_epoch: 0,
                best_val_loss: 0.0,
                best_train_loss: 0.0,
                epoch_metrics,
            };
        }

        // Truncate to max_triplets_per_run
        let triplets = if triplets.len() > self.config.max_triplets_per_run {
            triplets[..self.config.max_triplets_per_run].to_vec()
        } else {
            triplets
        };

        // Train/validation split
        let (train_triplets, val_triplets) = if triplets.is_empty() {
            (&[][..], &[][..])
        } else {
            let split_idx = ((1.0 - self.config.validation_split) * triplets.len() as f32) as usize;
            let split_idx = split_idx.max(1).min(triplets.len().saturating_sub(1));
            triplets.split_at(split_idx)
        };

        // Initialize optimizer with current layer shapes
        let (l1, l2, l3) = enhancer.get_weights();
        self.optimizer = AdamOptimizer::new(
            AdamConfig {
                learning_rate: self.config.learning_rate,
                ..AdamConfig::default()
            },
            &[&l1, &l2, &l3],
        );

        // Compute initial loss — bust cache so embeddings are fresh
        enhancer.clear_cache();
        let all_embeddings = Self::forward_all(enhancer, samples);
        let initial_quality_loss = self.compute_triplet_loss(train_triplets, &all_embeddings);
        let initial_triplet_loss = self.compute_meaning_triplet_loss(enhancer, triplet_batch);
        let initial_loss =
            initial_quality_loss + self.config.triplet_loss_coefficient * initial_triplet_loss;
        let mut best_loss = f32::MAX;

        // Record pre-training weight version
        let weight_version_before = self
            .weight_store
            .as_ref()
            .map(|ws| ws.current_version())
            .unwrap_or(0);

        let mut no_improvement_epochs = 0usize;
        let mut epochs_completed = 0usize;
        let mut best_epoch_weights: Option<(LayerWeights, LayerWeights, LayerWeights)> = None;
        let mut best_epoch = 0usize;
        let mut best_train_loss = 0.0_f32;

        for epoch in 0..self.config.max_epochs {
            // Timeout check
            if start_time.elapsed().as_millis() as u64 > self.config.max_runtime_ms {
                timed_out = true;
                break;
            }

            // Cancellation check
            if cancel.map(|c| c.load(std::sync::atomic::Ordering::Relaxed)) == Some(true) {
                cancelled = true;
                break;
            }

            // Snapshot weights BEFORE this epoch — if this epoch improves the
            // loss, we want the pre-epoch weights so best_train_loss (computed
            // from pre-epoch embeddings) matches the restored state.
            let pre_epoch_weights = enhancer.get_weights();

            // Refresh embeddings after previous epoch's weight updates — bust cache
            enhancer.clear_cache();
            let embeddings = Self::forward_all(enhancer, samples);

            let epoch_result = self.train_epoch(
                enhancer,
                samples,
                train_triplets,
                &embeddings,
                val_triplets,
                triplet_batch,
                &mut batches_processed,
                &mut samples_processed,
                cancel,
            );
            let cancelled_after_epoch =
                cancel.map(|c| c.load(std::sync::atomic::Ordering::Relaxed)) == Some(true);

            // Collect per-epoch layer-norm metrics
            let layer_norms = compute_layer_norms(enhancer);
            epoch_metrics.push(EpochMetrics {
                epoch,
                train_loss: epoch_result.train_loss,
                val_loss: epoch_result.val_loss,
                loss_quality: epoch_result.loss_quality,
                loss_ewc: epoch_result.loss_ewc,
                loss_triplet: epoch_result.loss_triplet,
                layer_norms,
            });
            info!(
                epoch,
                loss_quality = epoch_result.loss_quality,
                loss_ewc = epoch_result.loss_ewc,
                loss_triplet = epoch_result.loss_triplet,
                "trainer.epoch"
            );

            epochs_completed += 1;

            if cancelled_after_epoch {
                cancelled = true;
                break;
            }

            // Early stopping check
            let patience = self.config.early_stopping_patience;
            let mut improved = false;
            if let Some(val_loss) = epoch_result.val_loss {
                if val_loss < best_loss - self.config.min_improvement {
                    best_loss = val_loss;
                    no_improvement_epochs = 0;
                    improved = true;
                } else if patience > 0 {
                    no_improvement_epochs += 1;
                    if no_improvement_epochs >= patience {
                        break;
                    }
                }
            } else if epoch_result.train_loss < best_loss - self.config.min_improvement {
                best_loss = epoch_result.train_loss;
                no_improvement_epochs = 0;
                improved = true;
            } else if patience > 0 {
                no_improvement_epochs += 1;
                if no_improvement_epochs >= patience {
                    break;
                }
            }

            if improved {
                best_epoch_weights = Some(pre_epoch_weights);
                best_epoch = epoch;
                best_train_loss = epoch_result.train_loss;
            }
        }

        if cancelled {
            enhancer.clear_cache();
            let best_loss = if best_loss == f32::MAX {
                initial_loss
            } else {
                best_loss
            };
            return TrainingOutcome {
                epochs_completed,
                batches_processed,
                samples_processed,
                initial_loss,
                final_loss: initial_loss,
                best_loss,
                validation_loss: None,
                stopped_early: false,
                timed_out,
                cancelled: true,
                best_epoch,
                best_val_loss: best_loss,
                best_train_loss,
                epoch_metrics,
            };
        }

        let stopped_early = self.config.early_stopping_patience > 0
            && no_improvement_epochs >= self.config.early_stopping_patience;

        // Restore best-epoch weights when early stopping fired
        if stopped_early && let Some((l1, l2, l3)) = best_epoch_weights.take() {
            enhancer.set_weights(l1, l2, l3);
            enhancer.clear_cache();
        }

        enhancer.clear_cache();
        let final_embeddings = Self::forward_all(enhancer, samples);
        let final_quality_loss = self.compute_triplet_loss(train_triplets, &final_embeddings);
        let final_triplet_loss = self.compute_meaning_triplet_loss(enhancer, triplet_batch);
        let final_loss =
            final_quality_loss + self.config.triplet_loss_coefficient * final_triplet_loss;

        // Post-training: persist or rollback.
        // Check weight sanity first — NaN/Inf weights always trigger rollback.
        let weight_norms = compute_layer_norms(enhancer);
        let has_nan_weights = weight_norms.iter().any(|(_, _, nan)| *nan);
        let rolled_back =
            if final_loss > initial_loss * 1.1 || final_loss.is_nan() || has_nan_weights {
                warn!(
                    "Training degraded loss ({} → {}), rolling back to version {}",
                    initial_loss, final_loss, weight_version_before
                );
                if let Some(ref ws) = self.weight_store
                    && weight_version_before > 0
                {
                    let _ = ws.load_version(weight_version_before);
                }
                true
            } else if final_loss < initial_loss {
                if let Some(ref ws) = self.weight_store {
                    let (l1, l2, l3) = enhancer.get_weights();
                    ws.set_weights("layer1", l1.w, l1.bias);
                    ws.set_weights("layer2", l2.w, l2.bias);
                    ws.set_weights("layer3", l3.w, l3.bias);
                    match ws.save_all() {
                        Ok(new_version) => {
                            info!(
                                "Saved weight version {} (loss: {} → {})",
                                new_version, initial_loss, final_loss
                            );
                        }
                        Err(e) => warn!("Failed to save weights: {}", e),
                    }
                }
                false
            } else {
                false
            };

        TrainingOutcome {
            epochs_completed,
            batches_processed,
            samples_processed,
            initial_loss,
            final_loss,
            best_loss,
            validation_loss: if !val_triplets.is_empty() {
                Some(self.compute_triplet_loss(val_triplets, &final_embeddings))
            } else {
                None
            },
            stopped_early,
            timed_out,
            cancelled: cancelled || rolled_back,
            best_epoch,
            best_val_loss: best_loss,
            best_train_loss,
            epoch_metrics,
        }
    }

    // -----------------------------------------------------------------------
    // Private: forward / epoch / batch
    // -----------------------------------------------------------------------

    /// Forward-pass all samples, collecting enhanced embeddings.
    fn forward_all(enhancer: &GnnEnhancer, samples: &[TrajectoryWithFeedback]) -> Vec<Vec<f32>> {
        samples
            .iter()
            .map(|s| {
                let fwd = enhancer.enhance(&s.embedding, None, None, false);
                fwd.enhanced
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn train_epoch(
        &mut self,
        enhancer: &GnnEnhancer,
        samples: &[TrajectoryWithFeedback],
        train: &[loss::Triplet],
        embeddings: &[Vec<f32>],
        val: &[loss::Triplet],
        triplet_batch: &TripletBatch,
        batches_processed: &mut usize,
        samples_processed: &mut usize,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> EpochResult {
        let mut total_quality_loss = 0.0_f32;
        let mut quality_batch_count = 0usize;
        let mut triplet_loss = 0.0_f32;

        for batch in train.chunks(self.config.batch_size) {
            if cancel.map(|c| c.load(std::sync::atomic::Ordering::Relaxed)) == Some(true) {
                break;
            }

            let result = self.train_batch(enhancer, samples, batch, embeddings);
            total_quality_loss += result.loss;
            quality_batch_count += 1;

            // Apply gradients
            self.apply_gradients(enhancer, &result.grads);

            *batches_processed += 1;
            *samples_processed += batch.len();
        }

        if !triplet_batch.triplets.is_empty()
            && cancel.map(|c| c.load(std::sync::atomic::Ordering::Relaxed)) != Some(true)
        {
            let result = self.train_meaning_triplet_batch(enhancer, triplet_batch);
            triplet_loss = result.loss;
            let scaled = scale_grads(&result.grads, self.config.triplet_loss_coefficient);
            self.apply_gradients(enhancer, &scaled);
            *batches_processed += 1;
            *samples_processed += triplet_batch.triplets.len();
        }

        let loss_quality = if quality_batch_count > 0 {
            total_quality_loss / quality_batch_count as f32
        } else {
            0.0
        };
        let loss_ewc = self.current_ewc_loss(enhancer);
        let train_loss =
            loss_quality + loss_ewc + self.config.triplet_loss_coefficient * triplet_loss;

        let val_loss = if !val.is_empty() {
            Some(self.compute_triplet_loss(val, embeddings))
        } else {
            None
        };

        EpochResult {
            train_loss,
            val_loss,
            loss_quality,
            loss_ewc,
            loss_triplet: triplet_loss,
        }
    }

    #[allow(clippy::type_complexity)]
    fn train_batch(
        &self,
        enhancer: &GnnEnhancer,
        samples: &[TrajectoryWithFeedback],
        batch: &[loss::Triplet],
        embeddings: &[Vec<f32>],
    ) -> BatchResult {
        let mut total_loss = 0.0_f32;
        let mut accumulated_grads: Option<Vec<(Vec<Vec<f32>>, Vec<f32>)>> = None;

        // Collect unique indices in this batch for activation-collected forward pass
        let mut unique_indices: Vec<usize> = Vec::new();
        {
            let mut seen = std::collections::HashSet::new();
            for t in batch {
                if seen.insert(t.anchor) {
                    unique_indices.push(t.anchor);
                }
                if seen.insert(t.positive) {
                    unique_indices.push(t.positive);
                }
                if seen.insert(t.negative) {
                    unique_indices.push(t.negative);
                }
            }
        }

        // Forward pass with activation collection for unique samples
        let mut activations: HashMap<usize, Vec<super::LayerActivationCache>> = HashMap::new();
        for &idx in &unique_indices {
            let fwd = enhancer.enhance(
                &samples[idx].embedding,
                None,
                None,
                true, // collect activations
            );
            activations.insert(idx, fwd.activation_cache);
        }

        for triplet in batch {
            let emb_a = &embeddings[triplet.anchor];
            let emb_p = &embeddings[triplet.positive];
            let emb_n = &embeddings[triplet.negative];

            let loss_result = loss::compute_loss(emb_a, emb_p, emb_n, self.config.margin);

            total_loss += loss_result.loss;
            if loss_result.loss <= 0.0 {
                continue;
            }

            // Backprop through GNN for anchor embedding
            if let Some(caches) = activations.get(&triplet.anchor)
                && caches.len() == 3
            {
                let (l1, l2, l3) = enhancer.get_weights();
                let grads = backprop::full_backward(
                    caches,
                    [&l1, &l2, &l3],
                    &loss_result.grad_anchor,
                    [
                        ActivationType::LeakyRelu,
                        ActivationType::LeakyRelu,
                        ActivationType::Tanh,
                    ],
                );

                let layer_grads: Vec<(Vec<Vec<f32>>, Vec<f32>)> =
                    grads.into_iter().map(|g| (g.dw, g.db)).collect();

                // Accumulate
                match &mut accumulated_grads {
                    Some(acc) => {
                        for (i, (dw, db)) in layer_grads.iter().enumerate() {
                            for (row_a, row_g) in acc[i].0.iter_mut().zip(dw.iter()) {
                                for (a, g) in row_a.iter_mut().zip(row_g.iter()) {
                                    *a += *g;
                                }
                            }
                            for (a, g) in acc[i].1.iter_mut().zip(db.iter()) {
                                *a += *g;
                            }
                        }
                    }
                    None => {
                        accumulated_grads = Some(layer_grads);
                    }
                }
            }
        }

        let batch_size = batch.len() as f32;
        let avg_loss = if batch.is_empty() {
            0.0
        } else {
            total_loss / batch_size
        };

        // Average gradients
        let grads = accumulated_grads
            .map(|acc| {
                acc.into_iter()
                    .map(|(dw, db)| {
                        let dw: Vec<Vec<f32>> = dw
                            .into_iter()
                            .map(|row| row.into_iter().map(|v| v / batch_size).collect())
                            .collect();
                        let db: Vec<f32> = db.into_iter().map(|v| v / batch_size).collect();
                        (dw, db)
                    })
                    .collect()
            })
            .unwrap_or_else(|| zero_grads(enhancer));

        BatchResult {
            loss: avg_loss,
            grads,
        }
    }

    fn apply_gradients(&mut self, enhancer: &GnnEnhancer, grads: &[(Vec<Vec<f32>>, Vec<f32>)]) {
        let mut clipped = grads.to_vec();
        for (dw, db) in &mut clipped {
            backprop::clip_gradient_matrix(dw, self.config.max_gradient_norm);
            backprop::clip_gradients(db, self.config.max_gradient_norm);
        }

        // Add EWC penalty gradients
        if self.ewc.is_initialized() {
            let (l1, l2, l3) = enhancer.get_weights();
            let ewc_layers = [
                self.ewc.penalty_gradient("gnn_embed", &l1.w),
                self.ewc.penalty_gradient("gnn_hidden", &l2.w),
                self.ewc.penalty_gradient("gnn_output", &l3.w),
            ];
            for (i, ewc_grad) in ewc_layers.iter().enumerate() {
                if let Some((dw, _db)) = clipped.get_mut(i) {
                    for (row_dw, row_ewc) in dw.iter_mut().zip(ewc_grad.iter()) {
                        for (d, e) in row_dw.iter_mut().zip(row_ewc.iter()) {
                            *d += *e;
                        }
                    }
                }
            }
        }

        // Apply Adam step
        let (l1, l2, l3) = enhancer.get_weights();
        let mut layers = vec![l1.clone(), l2.clone(), l3.clone()];
        self.optimizer.step(&mut layers, &clipped);
        let l1 = layers.remove(0);
        let l2 = layers.remove(0);
        let l3 = layers.remove(0);
        enhancer.set_weights(l1, l2, l3);
    }

    fn compute_triplet_loss(&self, triplets: &[loss::Triplet], embeddings: &[Vec<f32>]) -> f32 {
        if triplets.is_empty() {
            return 0.0;
        }
        let total: f32 = triplets
            .iter()
            .map(|t| {
                loss::compute_loss(
                    &embeddings[t.anchor],
                    &embeddings[t.positive],
                    &embeddings[t.negative],
                    self.config.margin,
                )
                .loss
            })
            .sum();
        total / triplets.len() as f32
    }

    fn train_meaning_triplet_batch(
        &self,
        enhancer: &GnnEnhancer,
        batch: &TripletBatch,
    ) -> BatchResult {
        if batch.triplets.is_empty() {
            return BatchResult {
                loss: 0.0,
                grads: zero_grads(enhancer),
            };
        }

        let (l1, l2, l3) = enhancer.get_weights();
        let mut total_loss = 0.0_f32;
        let mut accumulated_grads: Option<Vec<(Vec<Vec<f32>>, Vec<f32>)>> = None;

        for triplet in &batch.triplets {
            let anchor = enhancer.enhance(&triplet.anchor, None, None, true);
            let positive = enhancer.enhance(&triplet.positive, None, None, true);
            let negative = enhancer.enhance(&triplet.negative, None, None, true);

            let loss_result = triplets_loss::triplet_loss_gradient(
                &anchor.enhanced,
                &positive.enhanced,
                &negative.enhanced,
                &self.triplet_loss_config,
            );
            total_loss += loss_result.loss;
            if loss_result.loss <= 0.0 {
                continue;
            }

            accumulate_embedding_grads(
                &mut accumulated_grads,
                &anchor.activation_cache,
                [&l1, &l2, &l3],
                &pad_gradient(loss_result.grad_anchor, anchor.enhanced.len()),
            );
            accumulate_embedding_grads(
                &mut accumulated_grads,
                &positive.activation_cache,
                [&l1, &l2, &l3],
                &pad_gradient(loss_result.grad_positive, positive.enhanced.len()),
            );
            accumulate_embedding_grads(
                &mut accumulated_grads,
                &negative.activation_cache,
                [&l1, &l2, &l3],
                &pad_gradient(loss_result.grad_negative, negative.enhanced.len()),
            );
        }

        let batch_size = batch.triplets.len() as f32;
        let grads = accumulated_grads
            .map(|acc| average_grads(acc, batch_size))
            .unwrap_or_else(|| zero_grads(enhancer));

        BatchResult {
            loss: total_loss / batch_size,
            grads,
        }
    }

    fn compute_meaning_triplet_loss(&self, enhancer: &GnnEnhancer, batch: &TripletBatch) -> f32 {
        if batch.triplets.is_empty() {
            return 0.0;
        }
        let total = batch
            .triplets
            .iter()
            .map(|triplet| {
                let anchor = enhancer.enhance(&triplet.anchor, None, None, false);
                let positive = enhancer.enhance(&triplet.positive, None, None, false);
                let negative = enhancer.enhance(&triplet.negative, None, None, false);
                triplets_loss::triplet_loss(
                    &anchor.enhanced,
                    &positive.enhanced,
                    &negative.enhanced,
                    &self.triplet_loss_config,
                )
            })
            .sum::<f32>();
        total / batch.triplets.len() as f32
    }

    fn current_ewc_loss(&self, enhancer: &GnnEnhancer) -> f32 {
        let (l1, l2, l3) = enhancer.get_weights();
        let mut weights = HashMap::new();
        weights.insert("gnn_embed".to_string(), l1.w);
        weights.insert("gnn_hidden".to_string(), l2.w);
        weights.insert("gnn_output".to_string(), l3.w);
        self.ewc.penalty(&weights)
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute per-layer L2 weight norms and NaN/Inf status.
fn compute_layer_norms(enhancer: &GnnEnhancer) -> Vec<(String, f32, bool)> {
    let (l1, l2, l3) = enhancer.get_weights();
    let layers = [("layer1", &l1), ("layer2", &l2), ("layer3", &l3)];
    layers
        .iter()
        .map(|(name, lw)| {
            let mut sum_sq = 0.0f64;
            let mut has_nan = false;
            for row in &lw.w {
                for &v in row {
                    if v.is_nan() || v.is_infinite() {
                        has_nan = true;
                    }
                    sum_sq += (v as f64) * (v as f64);
                }
            }
            for &v in &lw.bias {
                if v.is_nan() || v.is_infinite() {
                    has_nan = true;
                }
                sum_sq += (v as f64) * (v as f64);
            }
            (name.to_string(), (sum_sq.sqrt()) as f32, has_nan)
        })
        .collect()
}

fn accumulate_embedding_grads(
    accumulated: &mut Option<Vec<(Vec<Vec<f32>>, Vec<f32>)>>,
    caches: &[super::LayerActivationCache],
    weights: [&LayerWeights; 3],
    grad: &[f32],
) {
    if caches.len() != 3 {
        return;
    }
    let grads = backprop::full_backward(
        caches,
        weights,
        grad,
        [
            ActivationType::LeakyRelu,
            ActivationType::LeakyRelu,
            ActivationType::Tanh,
        ],
    );
    let layer_grads: Vec<(Vec<Vec<f32>>, Vec<f32>)> =
        grads.into_iter().map(|grad| (grad.dw, grad.db)).collect();
    match accumulated {
        Some(acc) => add_grads_in_place(acc, &layer_grads),
        None => *accumulated = Some(layer_grads),
    }
}

fn add_grads_in_place(
    acc: &mut [(Vec<Vec<f32>>, Vec<f32>)],
    layer_grads: &[(Vec<Vec<f32>>, Vec<f32>)],
) {
    for (i, (dw, db)) in layer_grads.iter().enumerate() {
        for (row_a, row_g) in acc[i].0.iter_mut().zip(dw.iter()) {
            for (a, g) in row_a.iter_mut().zip(row_g.iter()) {
                *a += *g;
            }
        }
        for (a, g) in acc[i].1.iter_mut().zip(db.iter()) {
            *a += *g;
        }
    }
}

fn average_grads(
    grads: Vec<(Vec<Vec<f32>>, Vec<f32>)>,
    divisor: f32,
) -> Vec<(Vec<Vec<f32>>, Vec<f32>)> {
    if divisor <= 0.0 {
        return grads;
    }
    grads
        .into_iter()
        .map(|(dw, db)| {
            let dw = dw
                .into_iter()
                .map(|row| row.into_iter().map(|value| value / divisor).collect())
                .collect();
            let db = db.into_iter().map(|value| value / divisor).collect();
            (dw, db)
        })
        .collect()
}

fn scale_grads(grads: &[(Vec<Vec<f32>>, Vec<f32>)], scale: f32) -> Vec<(Vec<Vec<f32>>, Vec<f32>)> {
    grads
        .iter()
        .map(|(dw, db)| {
            let dw = dw
                .iter()
                .map(|row| row.iter().map(|value| value * scale).collect())
                .collect();
            let db = db.iter().map(|value| value * scale).collect();
            (dw, db)
        })
        .collect()
}

fn pad_gradient(mut grad: Vec<f32>, dim: usize) -> Vec<f32> {
    grad.resize(dim, 0.0);
    grad
}

fn zero_grads(enhancer: &GnnEnhancer) -> Vec<(Vec<Vec<f32>>, Vec<f32>)> {
    let (l1, l2, l3) = enhancer.get_weights();
    vec![
        (
            vec![vec![0.0; l1.w[0].len()]; l1.w.len()],
            vec![0.0; l1.bias.len()],
        ),
        (
            vec![vec![0.0; l2.w[0].len()]; l2.w.len()],
            vec![0.0; l2.bias.len()],
        ),
        (
            vec![vec![0.0; l3.w[0].len()]; l3.w.len()],
            vec![0.0; l3.bias.len()],
        ),
    ]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::gnn::{CacheConfig, GnnConfig};

    fn make_sample(id: &str, embedding: Vec<f32>, quality: f32) -> TrajectoryWithFeedback {
        TrajectoryWithFeedback {
            trajectory_id: id.to_string(),
            embedding,
            quality,
        }
    }

    fn test_enhancer() -> GnnEnhancer {
        GnnEnhancer::with_in_memory_weights(GnnConfig::default(), CacheConfig::default(), 42)
    }

    #[test]
    fn test_trainer_requires_enough_samples() {
        let mut trainer = GnnTrainer::new(TrainingConfig::default(), None);
        let enhancer = test_enhancer();
        let samples = vec![
            make_sample("a", vec![0.1; 4], 0.9),
            make_sample("b", vec![0.2; 4], 0.1),
        ];
        let outcome = trainer.train(&enhancer, &samples, None);
        assert_eq!(outcome.epochs_completed, 0);
    }

    #[test]
    fn test_trainer_runs_with_valid_samples() {
        let mut trainer = GnnTrainer::new(
            TrainingConfig {
                max_epochs: 1,
                batch_size: 4,
                max_triplets_per_run: 16,
                max_runtime_ms: 30_000,
                ..TrainingConfig::default()
            },
            None,
        );
        let enhancer = test_enhancer();
        let samples: Vec<TrajectoryWithFeedback> = (0..10)
            .map(|i| {
                let q = if i % 2 == 0 { 0.9 } else { 0.1 };
                make_sample(&format!("t{}", i), vec![i as f32 * 0.1; 4], q)
            })
            .collect();

        let outcome = trainer.train(&enhancer, &samples, None);
        assert!(outcome.epochs_completed >= 1);
        assert!(outcome.batches_processed > 0);
    }

    #[test]
    fn trainer_handles_empty_triplet_batch() {
        let mut trainer = GnnTrainer::new(
            TrainingConfig {
                max_epochs: 1,
                batch_size: 4,
                max_triplets_per_run: 16,
                max_runtime_ms: 30_000,
                ..TrainingConfig::default()
            },
            None,
        );
        let enhancer = test_enhancer();
        let samples: Vec<TrajectoryWithFeedback> = (0..10)
            .map(|i| {
                let q = if i % 2 == 0 { 0.9 } else { 0.1 };
                make_sample(&format!("t{}", i), vec![i as f32 * 0.1; 4], q)
            })
            .collect();

        let outcome =
            trainer.train_with_triplets(&enhancer, &samples, &TripletBatch::default(), None);

        assert!(outcome.epochs_completed >= 1);
        assert!(
            outcome
                .epoch_metrics
                .iter()
                .all(|epoch| epoch.loss_triplet == 0.0)
        );
    }

    #[test]
    fn test_trainer_cancellation() {
        let mut trainer = GnnTrainer::new(
            TrainingConfig {
                max_epochs: 10,
                batch_size: 2,
                max_triplets_per_run: 16,
                max_runtime_ms: 30_000,
                ..TrainingConfig::default()
            },
            None,
        );
        let enhancer = test_enhancer();
        let samples: Vec<TrajectoryWithFeedback> = (0..10)
            .map(|i| {
                let q = if i % 2 == 0 { 0.9 } else { 0.1 };
                make_sample(&format!("t{}", i), vec![i as f32 * 0.1; 4], q)
            })
            .collect();

        let cancel = std::sync::atomic::AtomicBool::new(true);
        let outcome = trainer.train(&enhancer, &samples, Some(&cancel));
        assert!(outcome.cancelled);
    }

    #[test]
    fn test_training_config_defaults() {
        let cfg = TrainingConfig::default();
        assert!((cfg.learning_rate - 0.001).abs() < 1e-6);
        assert_eq!(cfg.max_epochs, 10);
        assert_eq!(cfg.batch_size, 32);
    }

    #[test]
    fn test_training_outcome_fields() {
        let outcome = TrainingOutcome {
            epochs_completed: 5,
            batches_processed: 20,
            samples_processed: 640,
            initial_loss: 0.5,
            final_loss: 0.3,
            best_loss: 0.25,
            validation_loss: Some(0.28),
            stopped_early: false,
            timed_out: false,
            cancelled: false,
            best_epoch: 4,
            best_val_loss: 0.25,
            best_train_loss: 0.26,
            epoch_metrics: vec![],
        };
        assert!(outcome.final_loss < outcome.initial_loss);
        assert!(!outcome.cancelled);
    }
}

use std::time::Instant;

use tracing::{info, warn};

use super::super::loss::{self, TrajectoryWithFeedback};
use super::super::optimizer::{AdamConfig, AdamOptimizer};
use super::super::triplets_loss::TripletBatch;
use super::super::{GnnEnhancer, LayerWeights};
use super::gradients::scale_grads;
use super::metrics::compute_layer_norms;
use super::types::{EpochMetrics, EpochResult, GnnTrainer, TrainingDataSources, TrainingOutcome};

impl GnnTrainer {
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
        let mut sona_samples_processed = 0usize;
        let mut meaning_triplets_processed = 0usize;
        let mut batches_processed = 0usize;
        let mut timed_out = false;
        let mut cancelled = false;
        let mut epoch_metrics: Vec<EpochMetrics> = Vec::new();

        // Build triplets (indices into samples)
        let triplets = loss::build_triplets(samples, &self.loss_config);
        let data_sources = TrainingDataSources {
            sona_trajectories: samples.len(),
            sona_triplets: triplets.len(),
            meaning_triplets: triplet_batch.triplets.len(),
        };
        if let Some(reason) = data_sources.no_data_reason() {
            warn!(
                reason,
                sona_trajectories = data_sources.sona_trajectories,
                sona_triplets = data_sources.sona_triplets,
                meaning_triplets = data_sources.meaning_triplets,
                "Not enough training data"
            );
            return TrainingOutcome {
                epochs_completed: 0,
                batches_processed: 0,
                samples_processed: 0,
                sona_samples_processed: 0,
                meaning_triplets_processed: 0,
                data_sources,
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
                &mut sona_samples_processed,
                &mut meaning_triplets_processed,
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
                sona_samples_processed,
                meaning_triplets_processed,
                data_sources,
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
            sona_samples_processed,
            meaning_triplets_processed,
            data_sources,
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
    pub(super) fn forward_all(
        enhancer: &GnnEnhancer,
        samples: &[TrajectoryWithFeedback],
    ) -> Vec<Vec<f32>> {
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
        sona_samples_processed: &mut usize,
        meaning_triplets_processed: &mut usize,
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
            *sona_samples_processed += batch.len();
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
            *meaning_triplets_processed += triplet_batch.triplets.len();
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
}

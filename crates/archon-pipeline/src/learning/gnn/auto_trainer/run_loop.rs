use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::super::GnnEnhancer;
use super::super::loss::TrajectoryWithFeedback;
use super::super::trainer::{GnnTrainer, TrainingConfig};
use super::super::triplets_loss::TripletBatch;
use super::super::weights::WeightStore;
use super::types::{AutoTrainer, AutoTrainerConfig, TrainerState};

impl AutoTrainer {
    pub(super) async fn run_loop(
        config: AutoTrainerConfig,
        state: Arc<TrainerState>,
        enhancer: Arc<GnnEnhancer>,
        weight_store: Arc<WeightStore>,
        train_cfg: TrainingConfig,
        sample_provider: Arc<dyn Fn() -> Vec<TrajectoryWithFeedback> + Send + Sync>,
        triplet_provider: Arc<dyn Fn() -> TripletBatch + Send + Sync>,
        cancel: CancellationToken,
        training_cancel: Arc<AtomicBool>,
    ) {
        let mut tick = tokio::time::interval(Duration::from_millis(config.tick_interval_ms));
        // Skip the initial immediate tick — wait for the first interval
        tick.tick().await;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("AutoTrainer: cancellation received, exiting loop");
                    break;
                }
                _ = tick.tick() => {
                    if !Self::check_triggers(&config, &state) {
                        continue;
                    }

                    let mem_since = state.total_memories.load(Ordering::Relaxed)
                        .saturating_sub(state.memories_at_last_train.load(Ordering::Relaxed));
                    let corr_since = state.total_corrections.load(Ordering::Relaxed)
                        .saturating_sub(state.corrections_at_last_train.load(Ordering::Relaxed));
                    info!(
                        mem_since,
                        corr_since,
                        "AutoTrainer: triggers fired, starting training"
                    );

                    state.training_in_progress.store(true, Ordering::Relaxed);

                    let state2 = Arc::clone(&state);
                    let enhancer2 = Arc::clone(&enhancer);
                    let ws2 = Arc::clone(&weight_store);
                    let train_cfg2 = train_cfg.clone();
                    let provider2 = Arc::clone(&sample_provider);
                    let triplet_provider2 = Arc::clone(&triplet_provider);
                    let training_cancel2 = Arc::clone(&training_cancel);

                    let samples = provider2();
                    let triplet_batch = triplet_provider2();
                    info!(
                        count = triplet_batch.triplets.len(),
                        "auto_trainer.triplet_load"
                    );

                    let outcome = archon_observability::spawn_blocking_named(
                        "gnn-auto-trainer-train",
                        move || {
                            let mut trainer = GnnTrainer::new(train_cfg2, Some(ws2));
                            trainer.train_with_triplets(
                                &enhancer2,
                                &samples,
                                &triplet_batch,
                                Some(training_cancel2.as_ref()),
                            )
                        },
                    )
                    .await;

                    match outcome {
                        Ok(outcome) => {
                            let no_data_reason =
                                outcome.data_sources.no_data_reason().map(str::to_string);
                            if let Some(reason) = no_data_reason {
                                warn!(
                                    reason = %reason,
                                    sona_trajectories = outcome.data_sources.sona_trajectories,
                                    sona_triplets = outcome.data_sources.sona_triplets,
                                    meaning_triplets = outcome.data_sources.meaning_triplets,
                                    "auto_trainer.no_data"
                                );
                                *state2.last_no_data_reason.write().unwrap() = Some(reason);
                                state2.no_data_count.fetch_add(1, Ordering::Relaxed);
                            } else {
                                info!(
                                    epochs = outcome.epochs_completed,
                                    initial = outcome.initial_loss,
                                    final_loss = outcome.final_loss,
                                    best = outcome.best_loss,
                                    sona_trajectories = outcome.data_sources.sona_trajectories,
                                    sona_triplets = outcome.data_sources.sona_triplets,
                                    meaning_triplets = outcome.data_sources.meaning_triplets,
                                    "AutoTrainer: training run complete"
                                );
                                *state2.last_no_data_reason.write().unwrap() = None;
                                state2.training_count.fetch_add(1, Ordering::Relaxed);
                                *state2.last_successful_train_time.write().unwrap() =
                                    Some(Instant::now());
                            }
                            *state2.last_outcome.write().unwrap() = Some(outcome);
                        }
                        Err(e) => {
                            warn!("AutoTrainer: spawn_blocking join error: {}", e);
                        }
                    }

                    state2.training_in_progress.store(false, Ordering::Relaxed);
                    state2.memories_at_last_train.store(
                        state2.total_memories.load(Ordering::Relaxed),
                        Ordering::Relaxed,
                    );
                    state2.corrections_at_last_train.store(
                        state2.total_corrections.load(Ordering::Relaxed),
                        Ordering::Relaxed,
                    );
                    *state2.last_train_time.write().unwrap() = Some(Instant::now());
                }
            }
        }
    }
}

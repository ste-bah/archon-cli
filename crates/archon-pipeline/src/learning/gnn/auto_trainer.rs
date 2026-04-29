//! Auto-trainer — background tokio task wrapping synchronous GNN training.
//!
//! PR 3 delivers a configurable background loop that checks triggers
//! (memory count, correction count, time elapsed, first run) and delegates
//! the sync [`GnnTrainer`] to [`tokio::task::spawn_blocking`].

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::GnnEnhancer;
use super::loss::TrajectoryWithFeedback;
use super::trainer::{GnnTrainer, TrainingConfig, TrainingOutcome};
use super::weights::WeightStore;

// ---------------------------------------------------------------------------
// AutoTrainerConfig
// ---------------------------------------------------------------------------

/// Configuration for the GNN auto-trainer background task.
///
/// Mirrors the root TS `background-trainer.ts` trigger/throttle model.
#[derive(Debug, Clone)]
pub struct AutoTrainerConfig {
    /// If false, [`AutoTrainer::spawn`] returns immediately.
    pub enabled: bool,
    /// Minimum time between training runs (throttle).
    pub min_throttle_ms: u64,
    /// Number of new memories since last train that triggers a run.
    pub trigger_new_memories: u64,
    /// Time since last train that triggers a run regardless of other triggers.
    pub trigger_elapsed_ms: u64,
    /// Number of corrections since last train that triggers a run.
    pub trigger_corrections: u64,
    /// Number of stored memories needed for the first training run.
    pub first_run_threshold: u64,
    /// Max wall-clock time per training run (passed to [`GnnTrainer`]).
    pub max_runtime_ms: u64,
    /// Background loop check interval.
    pub tick_interval_ms: u64,
}

impl Default for AutoTrainerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_throttle_ms: 3_600_000, // 1 hour
            trigger_new_memories: 50,
            trigger_elapsed_ms: 21_600_000, // 6 hours
            trigger_corrections: 5,
            first_run_threshold: 100,
            max_runtime_ms: 300_000,  // 5 minutes
            tick_interval_ms: 60_000, // 1 minute
        }
    }
}

// ---------------------------------------------------------------------------
// TrainerState
// ---------------------------------------------------------------------------

/// Shared state for the auto-trainer, readable via [`AutoTrainer::status`].
pub struct TrainerState {
    /// Total memories stored (cumulative, never reset).
    pub total_memories: AtomicU64,
    /// Total corrections recorded (cumulative, never reset).
    pub total_corrections: AtomicU64,
    /// `total_memories` at the time of the last completed training run.
    pub memories_at_last_train: AtomicU64,
    /// `total_corrections` at the time of the last completed training run.
    pub corrections_at_last_train: AtomicU64,
    /// How many training runs have completed.
    pub training_count: AtomicU64,
    /// When the last training run completed (None = never).
    pub last_train_time: RwLock<Option<Instant>>,
    /// Outcome of the most recent training run.
    pub last_outcome: RwLock<Option<TrainingOutcome>>,
    /// Set while a training run is executing in spawn_blocking.
    pub training_in_progress: AtomicBool,
}

impl Default for TrainerState {
    fn default() -> Self {
        Self {
            total_memories: AtomicU64::new(0),
            total_corrections: AtomicU64::new(0),
            memories_at_last_train: AtomicU64::new(0),
            corrections_at_last_train: AtomicU64::new(0),
            training_count: AtomicU64::new(0),
            last_train_time: RwLock::new(None),
            last_outcome: RwLock::new(None),
            training_in_progress: AtomicBool::new(false),
        }
    }
}

// ---------------------------------------------------------------------------
// AutoTrainerStatus — immutable snapshot for /learning-status
// ---------------------------------------------------------------------------

/// Immutable snapshot of auto-trainer state.
#[derive(Debug, Clone)]
pub struct AutoTrainerStatus {
    pub enabled: bool,
    pub training_count: u64,
    pub total_memories: u64,
    pub total_corrections: u64,
    pub memories_since_last_train: u64,
    pub corrections_since_last_train: u64,
    pub seconds_since_last_train: Option<u64>,
    pub training_in_progress: bool,
    pub last_outcome: Option<TrainingOutcome>,
}

// ---------------------------------------------------------------------------
// AutoTrainer
// ---------------------------------------------------------------------------

/// Background GNN auto-trainer.
///
/// Spawning launches a tokio task that polls trigger conditions every
/// `tick_interval_ms` and runs synchronous training via `spawn_blocking`.
///
/// # Lifecycle
///
/// ```text
/// AutoTrainer::new(config)
///   .spawn(enhancer, weight_store, train_cfg, sample_provider)
///     -> tokio::spawn(run_loop)
///       -> tick -> check_triggers() -> spawn_blocking(trainer.train())
///   .record_memory() / .record_correction()  (from LearningIntegration)
///   .shutdown()  (cancel token)
///   .status()    (for /learning-status)
/// ```
pub struct AutoTrainer {
    config: AutoTrainerConfig,
    state: Arc<TrainerState>,
    cancel: CancellationToken,
    handle: std::sync::Mutex<Option<JoinHandle<()>>>,
}

impl AutoTrainer {
    /// Create a new auto-trainer. Does not start the background task.
    pub fn new(config: AutoTrainerConfig) -> Self {
        Self {
            config,
            state: Arc::new(TrainerState::default()),
            cancel: CancellationToken::new(),
            handle: std::sync::Mutex::new(None),
        }
    }

    /// Launch the background training loop.
    ///
    /// If `config.enabled` is false, logs and returns without spawning.
    ///
    /// `sample_provider` is called on the async runtime thread before
    /// `spawn_blocking` offloads the sync training — it should return
    /// quickly (just query the DB, no heavy computation).
    pub fn spawn(
        &self,
        enhancer: Arc<GnnEnhancer>,
        weight_store: Arc<WeightStore>,
        train_cfg: TrainingConfig,
        sample_provider: Arc<dyn Fn() -> Vec<TrajectoryWithFeedback> + Send + Sync>,
    ) {
        if !self.config.enabled {
            info!("AutoTrainer: disabled, background task not started");
            return;
        }

        let state = Arc::clone(&self.state);
        let config = self.config.clone();
        let cancel = self.cancel.clone();

        let handle = tokio::spawn(async move {
            Self::run_loop(
                config,
                state,
                enhancer,
                weight_store,
                train_cfg,
                sample_provider,
                cancel,
            )
            .await;
        });

        self.handle.lock().unwrap().replace(handle);
        info!("AutoTrainer: background task spawned");
    }

    /// Record a new stored memory. Thread-safe, lock-free.
    pub fn record_memory(&self) {
        self.state.total_memories.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a new correction. Thread-safe, lock-free.
    pub fn record_correction(&self) {
        self.state.total_corrections.fetch_add(1, Ordering::Relaxed);
    }

    /// Convenience: record N memories at once.
    pub fn record_memories(&self, count: u64) {
        self.state
            .total_memories
            .fetch_add(count, Ordering::Relaxed);
    }

    /// Convenience: record N corrections at once.
    pub fn record_corrections(&self, count: u64) {
        self.state
            .total_corrections
            .fetch_add(count, Ordering::Relaxed);
    }

    /// Snapshot current state for `/learning-status`.
    pub fn status(&self) -> AutoTrainerStatus {
        let total = self.state.total_memories.load(Ordering::Relaxed);
        let at_last = self.state.memories_at_last_train.load(Ordering::Relaxed);
        let corr = self.state.total_corrections.load(Ordering::Relaxed);
        let corr_at_last = self.state.corrections_at_last_train.load(Ordering::Relaxed);
        let last_time = self.state.last_train_time.read().unwrap();

        AutoTrainerStatus {
            enabled: self.config.enabled,
            training_count: self.state.training_count.load(Ordering::Relaxed),
            total_memories: total,
            total_corrections: corr,
            memories_since_last_train: total.saturating_sub(at_last),
            corrections_since_last_train: corr.saturating_sub(corr_at_last),
            seconds_since_last_train: last_time.map(|t| t.elapsed().as_secs()),
            training_in_progress: self.state.training_in_progress.load(Ordering::Relaxed),
            last_outcome: self.state.last_outcome.read().unwrap().clone(),
        }
    }

    /// Signal cancellation and try to join the background task.
    ///
    /// Does not block — if a training run is in progress inside
    /// `spawn_blocking`, it will complete on its own (bounded by
    /// `max_runtime_ms`).
    pub fn shutdown(&self) {
        self.cancel.cancel();
        if let Some(handle) = self.handle.lock().unwrap().take() {
            // Non-blocking: the handle completes when the loop exits.
            // spawn_blocking runs to completion independently.
            if !handle.is_finished() {
                info!("AutoTrainer: shutdown signalled, background task still finishing");
            }
        }
    }

    // -------------------------------------------------------------------
    // Private: run loop
    // -------------------------------------------------------------------

    async fn run_loop(
        config: AutoTrainerConfig,
        state: Arc<TrainerState>,
        enhancer: Arc<GnnEnhancer>,
        weight_store: Arc<WeightStore>,
        train_cfg: TrainingConfig,
        sample_provider: Arc<dyn Fn() -> Vec<TrajectoryWithFeedback> + Send + Sync>,
        cancel: CancellationToken,
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

                    let samples = provider2();

                    let outcome = tokio::task::spawn_blocking(move || {
                        let cancel_flag = AtomicBool::new(false);
                        let mut trainer = GnnTrainer::new(train_cfg2, Some(ws2));
                        trainer.train(&enhancer2, &samples, Some(&cancel_flag))
                    })
                    .await;

                    match outcome {
                        Ok(outcome) => {
                            info!(
                                epochs = outcome.epochs_completed,
                                initial = outcome.initial_loss,
                                final_loss = outcome.final_loss,
                                best = outcome.best_loss,
                                "AutoTrainer: training run complete"
                            );
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
                    state2.training_count.fetch_add(1, Ordering::Relaxed);
                    *state2.last_train_time.write().unwrap() = Some(Instant::now());
                }
            }
        }
    }

    // -------------------------------------------------------------------
    // Private: trigger checking
    // -------------------------------------------------------------------

    fn check_triggers(config: &AutoTrainerConfig, state: &TrainerState) -> bool {
        let total = state.total_memories.load(Ordering::Relaxed);
        let memories_since =
            total.saturating_sub(state.memories_at_last_train.load(Ordering::Relaxed));
        let corr_total = state.total_corrections.load(Ordering::Relaxed);
        let corr_since =
            corr_total.saturating_sub(state.corrections_at_last_train.load(Ordering::Relaxed));
        let training_count = state.training_count.load(Ordering::Relaxed);

        if state.training_in_progress.load(Ordering::Relaxed) {
            return false;
        }

        // Throttle: enforce minimum time between runs
        if let Some(last) = *state.last_train_time.read().unwrap() {
            if (last.elapsed().as_millis() as u64) < config.min_throttle_ms {
                return false;
            }
        }

        // First run
        if training_count == 0 && total >= config.first_run_threshold {
            return true;
        }

        // Memory accumulation
        if memories_since >= config.trigger_new_memories {
            return true;
        }

        // Correction spike
        if corr_since >= config.trigger_corrections {
            return true;
        }

        // Time-based
        if let Some(last) = *state.last_train_time.read().unwrap() {
            if (last.elapsed().as_millis() as u64) >= config.trigger_elapsed_ms {
                return true;
            }
        }

        false
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_fires_when_threshold_met() {
        let config = AutoTrainerConfig {
            first_run_threshold: 10,
            ..Default::default()
        };
        let state = TrainerState::default();
        state.total_memories.store(10, Ordering::Relaxed);

        assert!(AutoTrainer::check_triggers(&config, &state));
    }

    #[test]
    fn first_run_does_not_fire_below_threshold() {
        let config = AutoTrainerConfig {
            first_run_threshold: 10,
            ..Default::default()
        };
        let state = TrainerState::default();
        state.total_memories.store(9, Ordering::Relaxed);

        assert!(!AutoTrainer::check_triggers(&config, &state));
    }

    #[test]
    fn memory_trigger_fires_after_threshold() {
        let config = AutoTrainerConfig {
            trigger_new_memories: 50,
            ..Default::default()
        };
        let state = TrainerState::default();
        state.total_memories.store(60, Ordering::Relaxed);
        state.memories_at_last_train.store(5, Ordering::Relaxed);
        // Need at least one prior train so first-run doesn't interfere
        state.training_count.store(1, Ordering::Relaxed);
        // Must pass throttle — set last_train_time far enough in the past
        *state.last_train_time.write().unwrap() =
            Some(Instant::now() - Duration::from_millis(config.min_throttle_ms + 1000));

        assert!(AutoTrainer::check_triggers(&config, &state));
    }

    #[test]
    fn memory_trigger_does_not_fire_below_threshold() {
        let config = AutoTrainerConfig {
            trigger_new_memories: 50,
            ..Default::default()
        };
        let state = TrainerState::default();
        state.total_memories.store(60, Ordering::Relaxed);
        state.memories_at_last_train.store(20, Ordering::Relaxed);
        state.training_count.store(1, Ordering::Relaxed);
        *state.last_train_time.write().unwrap() =
            Some(Instant::now() - Duration::from_millis(config.min_throttle_ms + 1000));

        assert!(!AutoTrainer::check_triggers(&config, &state));
    }

    #[test]
    fn correction_trigger_fires() {
        let config = AutoTrainerConfig {
            trigger_corrections: 5,
            ..Default::default()
        };
        let state = TrainerState::default();
        state.total_corrections.store(10, Ordering::Relaxed);
        state.training_count.store(1, Ordering::Relaxed);
        *state.last_train_time.write().unwrap() =
            Some(Instant::now() - Duration::from_millis(config.min_throttle_ms + 1000));

        assert!(AutoTrainer::check_triggers(&config, &state));
    }

    #[test]
    fn throttle_blocks_when_too_soon() {
        let config = AutoTrainerConfig {
            min_throttle_ms: 3_600_000,
            trigger_new_memories: 50,
            ..Default::default()
        };
        let state = TrainerState::default();
        state.total_memories.store(200, Ordering::Relaxed);
        state.training_count.store(1, Ordering::Relaxed);
        // Last train was just now — within throttle window
        *state.last_train_time.write().unwrap() = Some(Instant::now());

        assert!(!AutoTrainer::check_triggers(&config, &state));
    }

    #[test]
    fn training_in_progress_blocks_trigger() {
        let config = AutoTrainerConfig {
            trigger_new_memories: 50,
            ..Default::default()
        };
        let state = TrainerState::default();
        state.total_memories.store(200, Ordering::Relaxed);
        state.training_in_progress.store(true, Ordering::Relaxed);

        assert!(!AutoTrainer::check_triggers(&config, &state));
    }

    #[test]
    fn record_memory_increments_counter() {
        let trainer = AutoTrainer::new(AutoTrainerConfig::default());
        trainer.record_memory();
        trainer.record_memories(5);
        assert_eq!(trainer.state.total_memories.load(Ordering::Relaxed), 6);
    }

    #[test]
    fn record_correction_increments_counter() {
        let trainer = AutoTrainer::new(AutoTrainerConfig::default());
        trainer.record_correction();
        trainer.record_corrections(3);
        assert_eq!(trainer.state.total_corrections.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn status_reflects_current_state() {
        let config = AutoTrainerConfig {
            enabled: true,
            ..Default::default()
        };
        let trainer = AutoTrainer::new(config);
        trainer.record_memories(42);
        trainer.record_corrections(3);

        let status = trainer.status();
        assert!(status.enabled);
        assert_eq!(status.total_memories, 42);
        assert_eq!(status.total_corrections, 3);
        assert_eq!(status.training_count, 0);
        assert!(!status.training_in_progress);
    }

    #[test]
    fn spawn_respects_disabled() {
        let config = AutoTrainerConfig {
            enabled: false,
            ..Default::default()
        };
        let trainer = AutoTrainer::new(config);
        let enhancer = Arc::new(GnnEnhancer::with_in_memory_weights(
            Default::default(),
            Default::default(),
            0,
        ));
        let ws = Arc::new(WeightStore::with_in_memory());

        trainer.spawn(enhancer, ws, TrainingConfig::default(), Arc::new(|| vec![]));

        // Handle should not be stored when disabled
        assert!(trainer.handle.lock().unwrap().is_none());
    }
}

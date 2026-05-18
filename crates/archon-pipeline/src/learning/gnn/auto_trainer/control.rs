use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio_util::sync::CancellationToken;
use tracing::info;

use super::types::{AutoTrainer, AutoTrainerConfig, AutoTrainerStatus, TrainerState};

impl AutoTrainer {
    /// Create a new auto-trainer. Does not start the background task.
    pub fn new(config: AutoTrainerConfig) -> Self {
        Self {
            config,
            state: Arc::new(TrainerState::default()),
            cancel: CancellationToken::new(),
            training_cancel: Arc::new(AtomicBool::new(false)),
            handle: std::sync::Mutex::new(None),
        }
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
        let last_attempt_time = self.state.last_train_time.read().unwrap();
        let last_successful_train_time = self.state.last_successful_train_time.read().unwrap();

        AutoTrainerStatus {
            enabled: self.config.enabled,
            first_run_threshold: self.config.first_run_threshold,
            trigger_new_memories: self.config.trigger_new_memories,
            trigger_corrections: self.config.trigger_corrections,
            trigger_elapsed_ms: self.config.trigger_elapsed_ms,
            min_throttle_ms: self.config.min_throttle_ms,
            training_count: self.state.training_count.load(Ordering::Relaxed),
            no_data_count: self.state.no_data_count.load(Ordering::Relaxed),
            total_memories: total,
            total_corrections: corr,
            memories_since_last_train: total.saturating_sub(at_last),
            corrections_since_last_train: corr.saturating_sub(corr_at_last),
            seconds_since_last_train: last_successful_train_time.map(|t| t.elapsed().as_secs()),
            seconds_since_last_attempt: last_attempt_time.map(|t| t.elapsed().as_secs()),
            training_in_progress: self.state.training_in_progress.load(Ordering::Relaxed),
            last_outcome: self.state.last_outcome.read().unwrap().clone(),
            last_no_data_reason: self.state.last_no_data_reason.read().unwrap().clone(),
        }
    }

    /// Expose immutable runtime configuration for status surfaces.
    pub fn config(&self) -> &AutoTrainerConfig {
        &self.config
    }

    /// Signal cancellation and try to join the background task.
    ///
    /// Does not block. If a training run is in progress inside
    /// `spawn_blocking`, it receives a cancellation flag so it can stop at the
    /// next trainer checkpoint instead of keeping Tokio's blocking pool alive
    /// until the full training budget elapses.
    pub fn shutdown(&self) {
        self.cancel.cancel();
        self.training_cancel.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.lock().unwrap().take() {
            // Non-blocking: the handle completes when the loop exits.
            // The blocking trainer gets its own cancellation flag above.
            if !handle.is_finished() {
                info!("AutoTrainer: shutdown signalled, background task still finishing");
            }
        }
    }
}

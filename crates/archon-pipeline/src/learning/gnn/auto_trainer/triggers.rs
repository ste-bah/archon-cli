use std::sync::atomic::Ordering;

use tracing::{info, trace};

use super::types::{AutoTrainer, AutoTrainerConfig, TrainerState};

impl AutoTrainer {
    pub(crate) fn check_triggers(config: &AutoTrainerConfig, state: &TrainerState) -> bool {
        let total = state.total_memories.load(Ordering::Relaxed);
        let memories_since =
            total.saturating_sub(state.memories_at_last_train.load(Ordering::Relaxed));
        let corr_total = state.total_corrections.load(Ordering::Relaxed);
        let corr_since =
            corr_total.saturating_sub(state.corrections_at_last_train.load(Ordering::Relaxed));
        let training_count = state.training_count.load(Ordering::Relaxed);

        if !config.enabled {
            trace!(reason = "disabled", "autotrainer.skip");
            return false;
        }

        if state.training_in_progress.load(Ordering::Relaxed) {
            trace!(reason = "training_in_progress", "autotrainer.skip");
            return false;
        }

        // Throttle: enforce minimum time between runs
        if let Some(last) = *state.last_train_time.read().unwrap()
            && (last.elapsed().as_millis() as u64) < config.min_throttle_ms
        {
            trace!(
                reason = "throttled",
                elapsed_ms = last.elapsed().as_millis() as u64,
                throttle_ms = config.min_throttle_ms,
                "autotrainer.skip"
            );
            return false;
        }

        // First run
        if training_count == 0 {
            if config.trigger_corrections > 0 && corr_since >= config.trigger_corrections {
                info!(
                    trigger = "corrections",
                    corrections_since = corr_since,
                    trigger_corrections = config.trigger_corrections,
                    "autotrainer.train"
                );
                return true;
            }

            if total < config.first_run_threshold {
                trace!(
                    reason = "below_first_run_threshold",
                    total_memories = total,
                    first_run_threshold = config.first_run_threshold,
                    corrections_since = corr_since,
                    trigger_corrections = config.trigger_corrections,
                    "autotrainer.skip"
                );
                return false;
            }
            info!(
                trigger = "first_run",
                total_memories = total,
                first_run_threshold = config.first_run_threshold,
                "autotrainer.train"
            );
            return true;
        }

        // Memory accumulation
        if memories_since >= config.trigger_new_memories {
            info!(
                trigger = "new_memories",
                memories_since,
                trigger_new_memories = config.trigger_new_memories,
                "autotrainer.train"
            );
            return true;
        }
        trace!(
            reason = "below_new_memory_threshold",
            memories_since,
            trigger_new_memories = config.trigger_new_memories,
            "autotrainer.skip"
        );

        // Correction spike
        if corr_since >= config.trigger_corrections {
            info!(
                trigger = "corrections",
                corrections_since = corr_since,
                trigger_corrections = config.trigger_corrections,
                "autotrainer.train"
            );
            return true;
        }
        trace!(
            reason = "below_correction_threshold",
            corrections_since = corr_since,
            trigger_corrections = config.trigger_corrections,
            "autotrainer.skip"
        );

        // Time-based
        if let Some(last) = *state.last_train_time.read().unwrap()
            && (last.elapsed().as_millis() as u64) >= config.trigger_elapsed_ms
        {
            info!(
                trigger = "elapsed",
                elapsed_ms = last.elapsed().as_millis() as u64,
                trigger_elapsed_ms = config.trigger_elapsed_ms,
                "autotrainer.train"
            );
            return true;
        }
        if let Some(last) = *state.last_train_time.read().unwrap() {
            trace!(
                reason = "below_elapsed_threshold",
                elapsed_ms = last.elapsed().as_millis() as u64,
                trigger_elapsed_ms = config.trigger_elapsed_ms,
                "autotrainer.skip"
            );
        } else {
            trace!(reason = "elapsed_gate_unavailable", "autotrainer.skip");
        }

        false
    }
}

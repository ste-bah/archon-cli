use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::*;
use crate::learning::gnn::GnnEnhancer;
use crate::learning::gnn::trainer::TrainingConfig;
use crate::learning::gnn::weights::WeightStore;

#[test]
fn default_config_is_enabled() {
    assert!(AutoTrainerConfig::default().enabled);
}

#[test]
fn default_config_thresholds_are_tuned() {
    let config = AutoTrainerConfig::default();
    assert_eq!(config.first_run_threshold, 30);
    assert_eq!(config.trigger_new_memories, 20);
    assert_eq!(config.trigger_corrections, 3);
    assert_eq!(config.min_throttle_ms, 3_600_000);
    assert_eq!(config.max_runtime_ms, 300_000);
}

#[test]
fn should_train_skips_below_new_first_run_threshold() {
    let config = AutoTrainerConfig::default();
    let state = TrainerState::default();
    state.total_memories.store(29, Ordering::Relaxed);
    assert!(!AutoTrainer::check_triggers(&config, &state));

    state.total_memories.store(30, Ordering::Relaxed);
    assert!(AutoTrainer::check_triggers(&config, &state));
}

#[tracing_test::traced_test]
#[test]
fn should_train_logs_skip_reason_at_trace() {
    let config = AutoTrainerConfig::default();
    let state = TrainerState::default();
    state.total_memories.store(29, Ordering::Relaxed);

    assert!(!AutoTrainer::check_triggers(&config, &state));
    assert!(logs_contain("autotrainer.skip"));
    assert!(logs_contain("below_first_run_threshold"));
}

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
fn correction_trigger_can_start_first_run() {
    let config = AutoTrainerConfig {
        first_run_threshold: 100,
        trigger_corrections: 5,
        ..Default::default()
    };
    let state = TrainerState::default();
    state.total_memories.store(2, Ordering::Relaxed);
    state.total_corrections.store(5, Ordering::Relaxed);

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

    trainer.spawn(
        enhancer,
        ws,
        TrainingConfig::default(),
        Arc::new(std::vec::Vec::new),
    );

    // Handle should not be stored when disabled
    assert!(trainer.handle.lock().unwrap().is_none());
}

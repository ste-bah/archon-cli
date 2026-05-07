use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::super::trainer::TrainingOutcome;

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
            // Enabled by default. The 1h throttle and 5min runtime cap below
            // remain the compute safety rails; operators can still opt out
            // explicitly with learning.gnn.auto_trainer.enabled=false.
            enabled: true,
            min_throttle_ms: 3_600_000, // 1 hour
            max_runtime_ms: 300_000,    // 5 minutes
            // Tuned for the first 2-3 normal sessions instead of a dozen+.
            // Correction threshold matches governed-learning proposal clusters.
            first_run_threshold: 30,
            trigger_new_memories: 20,
            trigger_corrections: 3,
            trigger_elapsed_ms: 21_600_000, // 6 hours
            tick_interval_ms: 60_000,       // 1 minute
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
    pub first_run_threshold: u64,
    pub trigger_new_memories: u64,
    pub trigger_corrections: u64,
    pub trigger_elapsed_ms: u64,
    pub min_throttle_ms: u64,
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
    pub(super) config: AutoTrainerConfig,
    pub(super) state: Arc<TrainerState>,
    pub(super) cancel: CancellationToken,
    pub(super) training_cancel: Arc<AtomicBool>,
    pub(super) handle: Mutex<Option<JoinHandle<()>>>,
}

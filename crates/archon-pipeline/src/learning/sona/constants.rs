// ---------------------------------------------------------------------------
// Constants (matching TypeScript)
// ---------------------------------------------------------------------------

pub const DEFAULT_LEARNING_RATE: f64 = 0.01;
pub const DEFAULT_REGULARIZATION: f64 = 0.1;
pub const DEFAULT_DRIFT_ALERT_THRESHOLD: f64 = 0.3;
pub const DEFAULT_DRIFT_REJECT_THRESHOLD: f64 = 0.5;
pub const WEIGHT_MIN: f64 = -1.0;
pub const WEIGHT_MAX: f64 = 1.0;
pub const DEFAULT_MAX_CHECKPOINTS: usize = 10;
pub const FISHER_DECAY_RATE: f64 = 0.9;
pub const MAX_OBSERVATION_LEN: usize = 10_000;

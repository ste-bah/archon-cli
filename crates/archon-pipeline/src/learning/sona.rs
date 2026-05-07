//! SONA Engine - trajectory tracking, feedback, weight updates, drift detection.
//!
//! Implements REQ-LEARN-001 (trajectory tracking) and REQ-LEARN-002 (weight management).
//! Port of sona-engine.ts, sona-types.ts, sona-utils.ts, step-capture-service.ts.

mod config;
mod constants;
mod engine;
mod errors;
mod helpers;
mod math;
mod step_capture;
mod types;

pub use config::SonaConfig;
pub use constants::{
    DEFAULT_DRIFT_ALERT_THRESHOLD, DEFAULT_DRIFT_REJECT_THRESHOLD, DEFAULT_LEARNING_RATE,
    DEFAULT_MAX_CHECKPOINTS, DEFAULT_REGULARIZATION, FISHER_DECAY_RATE, MAX_OBSERVATION_LEN,
    WEIGHT_MAX, WEIGHT_MIN,
};
pub use engine::SonaEngine;
pub use errors::SonaError;
pub use helpers::pad_or_truncate;
pub use math::{
    calculate_gradient, calculate_reward, calculate_weight_update, cosine_similarity,
    crc32_checksum, update_fisher_information,
};
pub use step_capture::StepCaptureService;
pub use types::{
    DriftReport, DriftStatus, FeedbackInput, Trajectory, TrajectoryStep, WeightCheckpoint,
    WeightUpdateResult,
};

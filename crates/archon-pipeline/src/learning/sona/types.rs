use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A trajectory recording an agent's execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trajectory {
    pub trajectory_id: String,
    pub route: String,
    pub agent_key: String,
    pub session_id: String,
    pub patterns: Vec<String>,
    pub context: Vec<String>,
    pub embedding: Vec<f32>,
    pub quality: f64,
    pub reward: f64,
    pub feedback_score: f64,
    pub weights_path: String,
    pub created_at: u64,
    pub updated_at: u64,
}

/// A single step within a trajectory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryStep {
    pub step_id: String,
    pub trajectory_id: String,
    pub step_index: usize,
    pub action: String,
    pub observation: String,
    pub reward: f64,
    pub timestamp: u64,
}

/// Input for providing feedback on a trajectory.
#[derive(Debug, Clone)]
pub struct FeedbackInput {
    pub trajectory_id: String,
    pub quality: f64,
    pub l_score: f64,
    pub success_rate: f64,
}

/// Result of a weight update operation.
#[derive(Debug, Clone)]
pub struct WeightUpdateResult {
    pub route: String,
    pub pattern_id: String,
    pub old_weight: f64,
    pub new_weight: f64,
    pub gradient: f64,
}

/// Report from drift detection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftReport {
    pub status: DriftStatus,
    pub divergence: f64,
    pub threshold_used: f64,
}

/// Drift classification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DriftStatus {
    Normal,
    Alert,
    Reject,
}

/// Checkpoint of engine state for rollback.
#[derive(Debug, Clone)]
pub struct WeightCheckpoint {
    pub weights: HashMap<String, HashMap<String, f64>>,
    pub fisher: HashMap<String, HashMap<String, f64>>,
    pub timestamp: u64,
}

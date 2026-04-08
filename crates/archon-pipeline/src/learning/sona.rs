//! SONA Engine — trajectory tracking, feedback, weight updates, drift detection.
//!
//! Implements REQ-LEARN-001 (trajectory tracking) and REQ-LEARN-002 (weight management).
//! Port of sona-engine.ts, sona-types.ts, sona-utils.ts, step-capture-service.ts.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

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

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the SONA Engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SonaConfig {
    pub learning_rate: f64,
    pub regularization: f64,
    pub drift_alert_threshold: f64,
    pub drift_reject_threshold: f64,
    pub max_checkpoints: usize,
    pub fisher_decay_rate: f64,
}

impl Default for SonaConfig {
    fn default() -> Self {
        Self {
            learning_rate: DEFAULT_LEARNING_RATE,
            regularization: DEFAULT_REGULARIZATION,
            drift_alert_threshold: DEFAULT_DRIFT_ALERT_THRESHOLD,
            drift_reject_threshold: DEFAULT_DRIFT_REJECT_THRESHOLD,
            max_checkpoints: DEFAULT_MAX_CHECKPOINTS,
            fisher_decay_rate: FISHER_DECAY_RATE,
        }
    }
}

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
    pub quality: f64,
    pub reward: f64,
    pub feedback_score: f64,
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

/// SONA-specific errors.
#[derive(Debug, Clone)]
pub enum SonaError {
    TrajectoryValidation(String),
    WeightUpdate(String),
    DriftExceeded(String),
    FeedbackValidation(String),
    WeightPersistence(String),
    Checkpoint(String),
    RollbackLoop(String),
}

impl std::fmt::Display for SonaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TrajectoryValidation(s) => write!(f, "trajectory validation: {}", s),
            Self::WeightUpdate(s) => write!(f, "weight update: {}", s),
            Self::DriftExceeded(s) => write!(f, "drift exceeded: {}", s),
            Self::FeedbackValidation(s) => write!(f, "feedback validation: {}", s),
            Self::WeightPersistence(s) => write!(f, "weight persistence: {}", s),
            Self::Checkpoint(s) => write!(f, "checkpoint: {}", s),
            Self::RollbackLoop(s) => write!(f, "rollback loop: {}", s),
        }
    }
}

impl std::error::Error for SonaError {}

/// Checkpoint of engine state for rollback.
#[derive(Debug, Clone)]
pub struct WeightCheckpoint {
    pub weights: HashMap<String, HashMap<String, f64>>,
    pub fisher: HashMap<String, HashMap<String, f64>>,
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// Math utilities (matching sona-utils.ts)
// ---------------------------------------------------------------------------

/// reward = quality * l_score * success_rate
pub fn calculate_reward(quality: f64, l_score: f64, success_rate: f64) -> f64 {
    quality * l_score * success_rate
}

/// gradient = (reward - 0.5) * similarity
pub fn calculate_gradient(reward: f64, similarity: f64) -> f64 {
    (reward - 0.5) * similarity
}

/// weight_change = learning_rate * gradient / (1 + regularization * importance)
/// Result clamped to [WEIGHT_MIN, WEIGHT_MAX].
pub fn calculate_weight_update(
    gradient: f64,
    learning_rate: f64,
    regularization: f64,
    importance: f64,
) -> f64 {
    let change = learning_rate * gradient / (1.0 + regularization * importance);
    change.clamp(WEIGHT_MIN, WEIGHT_MAX)
}

/// fisher_update = decay * old_importance + (1 - decay) * gradient^2
pub fn update_fisher_information(old_importance: f64, gradient: f64, decay: f64) -> f64 {
    decay * old_importance + (1.0 - decay) * gradient * gradient
}

/// Cosine similarity between two vectors. Returns 0.0 for zero-norm vectors.
pub fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();

    if norm_a < 1e-12 || norm_b < 1e-12 {
        return 0.0;
    }

    dot / (norm_a * norm_b)
}

// ---------------------------------------------------------------------------
// CRC32 (polynomial 0xEDB88320)
// ---------------------------------------------------------------------------

/// Compute CRC32 checksum using polynomial 0xEDB88320.
pub fn crc32_checksum(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    crc ^ 0xFFFF_FFFF
}

// ---------------------------------------------------------------------------
// SONA Engine
// ---------------------------------------------------------------------------

/// The core SONA learning engine.
///
/// Manages trajectories, weights, Fisher information, drift detection,
/// and checkpoints. Called by both coding and research pipelines.
pub struct SonaEngine {
    config: SonaConfig,
    /// Trajectories by ID.
    trajectories: HashMap<String, Trajectory>,
    /// Weights: route -> pattern_id -> weight value.
    weights: HashMap<String, HashMap<String, f64>>,
    /// Fisher information: route -> pattern_id -> importance value.
    fisher: HashMap<String, HashMap<String, f64>>,
    /// Checkpoint stack for rollback.
    checkpoints: Vec<WeightCheckpoint>,
}

impl SonaEngine {
    pub fn new(config: SonaConfig) -> Self {
        Self {
            config,
            trajectories: HashMap::new(),
            weights: HashMap::new(),
            fisher: HashMap::new(),
            checkpoints: Vec::new(),
        }
    }

    /// Create a new trajectory for an agent execution.
    pub fn create_trajectory(
        &mut self,
        route: &str,
        agent_key: &str,
        session_id: &str,
    ) -> Trajectory {
        let now = epoch_secs();
        let traj = Trajectory {
            trajectory_id: uuid::Uuid::new_v4().to_string(),
            route: route.to_string(),
            agent_key: agent_key.to_string(),
            session_id: session_id.to_string(),
            quality: 0.0,
            reward: 0.0,
            feedback_score: 0.0,
            created_at: now,
            updated_at: now,
        };
        self.trajectories
            .insert(traj.trajectory_id.clone(), traj.clone());
        traj
    }

    /// Provide feedback on a trajectory, triggering a weight update.
    pub fn provide_feedback(
        &mut self,
        input: &FeedbackInput,
    ) -> Result<WeightUpdateResult, SonaError> {
        let traj = self
            .trajectories
            .get_mut(&input.trajectory_id)
            .ok_or_else(|| {
                SonaError::FeedbackValidation(format!(
                    "trajectory '{}' not found",
                    input.trajectory_id
                ))
            })?;

        let reward = calculate_reward(input.quality, input.l_score, input.success_rate);
        traj.quality = input.quality;
        traj.reward = reward;
        traj.feedback_score = input.l_score;
        traj.updated_at = epoch_secs();

        let route = traj.route.clone();
        let pattern_id = "default".to_string();

        // Calculate gradient (using reward and similarity=1.0 for default pattern)
        let gradient = calculate_gradient(reward, 1.0);

        // Get current Fisher importance
        let importance = self
            .fisher
            .get(&route)
            .and_then(|m| m.get(&pattern_id))
            .copied()
            .unwrap_or(0.0);

        // Calculate weight change
        let weight_change = calculate_weight_update(
            gradient,
            self.config.learning_rate,
            self.config.regularization,
            importance,
        );

        // Apply weight update
        let route_weights = self.weights.entry(route.clone()).or_default();
        let old_weight = *route_weights.get(&pattern_id).unwrap_or(&0.0);
        let new_weight = (old_weight + weight_change).clamp(WEIGHT_MIN, WEIGHT_MAX);
        route_weights.insert(pattern_id.clone(), new_weight);

        // Update Fisher information
        let route_fisher = self.fisher.entry(route.clone()).or_default();
        let old_fisher = *route_fisher.get(&pattern_id).unwrap_or(&0.0);
        let new_fisher =
            update_fisher_information(old_fisher, gradient, self.config.fisher_decay_rate);
        route_fisher.insert(pattern_id.clone(), new_fisher);

        Ok(WeightUpdateResult {
            route,
            pattern_id,
            old_weight,
            new_weight,
            gradient,
        })
    }

    /// Get a trajectory by ID.
    pub fn get_trajectory(&self, trajectory_id: &str) -> Option<&Trajectory> {
        self.trajectories.get(trajectory_id)
    }

    /// Get a weight value for a route/pattern.
    pub fn get_weight(&self, route: &str, pattern_id: &str) -> f64 {
        self.weights
            .get(route)
            .and_then(|m| m.get(pattern_id))
            .copied()
            .unwrap_or(0.0)
    }

    /// Check drift between old and new weight vectors.
    pub fn check_drift(&self, old_weights: &[f64], new_weights: &[f64]) -> DriftReport {
        let sim = cosine_similarity(old_weights, new_weights);
        let divergence = 1.0 - sim;

        let status = if divergence >= self.config.drift_reject_threshold {
            DriftStatus::Reject
        } else if divergence >= self.config.drift_alert_threshold {
            DriftStatus::Alert
        } else {
            DriftStatus::Normal
        };

        DriftReport {
            status,
            divergence,
            threshold_used: if divergence >= self.config.drift_reject_threshold {
                self.config.drift_reject_threshold
            } else {
                self.config.drift_alert_threshold
            },
        }
    }

    /// Save current weights and Fisher information as a checkpoint.
    pub fn save_checkpoint(&mut self) {
        let checkpoint = WeightCheckpoint {
            weights: self.weights.clone(),
            fisher: self.fisher.clone(),
            timestamp: epoch_secs(),
        };

        self.checkpoints.push(checkpoint);

        // Enforce max checkpoints
        while self.checkpoints.len() > self.config.max_checkpoints {
            self.checkpoints.remove(0);
        }
    }

    /// Rollback to the most recent checkpoint. Returns true if rollback occurred.
    pub fn rollback(&mut self) -> bool {
        if let Some(checkpoint) = self.checkpoints.pop() {
            self.weights = checkpoint.weights;
            self.fisher = checkpoint.fisher;
            true
        } else {
            false
        }
    }

    /// Number of stored checkpoints.
    pub fn checkpoint_count(&self) -> usize {
        self.checkpoints.len()
    }

    /// Serialize weights to binary with CRC32 appended.
    pub fn serialize_weights(&self) -> Vec<u8> {
        let json = serde_json::to_vec(&self.weights).unwrap_or_default();
        let crc = crc32_checksum(&json);
        let mut data = json;
        data.extend_from_slice(&crc.to_le_bytes());
        data
    }

    /// Deserialize weights from binary, validating CRC32.
    /// Returns Err on CRC mismatch (EC-PIPE-013).
    pub fn deserialize_weights(
        data: &[u8],
    ) -> Result<HashMap<String, HashMap<String, f64>>, SonaError> {
        if data.len() < 4 {
            return Err(SonaError::WeightPersistence(
                "data too short for CRC32".into(),
            ));
        }

        let json_data = &data[..data.len() - 4];
        let stored_crc = u32::from_le_bytes([
            data[data.len() - 4],
            data[data.len() - 3],
            data[data.len() - 2],
            data[data.len() - 1],
        ]);

        let computed_crc = crc32_checksum(json_data);
        if stored_crc != computed_crc {
            return Err(SonaError::WeightPersistence(format!(
                "CRC32 mismatch: stored={:#010x}, computed={:#010x}",
                stored_crc, computed_crc
            )));
        }

        serde_json::from_slice(json_data)
            .map_err(|e| SonaError::WeightPersistence(format!("deserialization failed: {}", e)))
    }
}

// ---------------------------------------------------------------------------
// Step Capture Service
// ---------------------------------------------------------------------------

/// Per-trajectory step buffer for recording agent reasoning steps.
pub struct StepCaptureService {
    buffers: HashMap<String, Vec<TrajectoryStep>>,
}

impl StepCaptureService {
    pub fn new() -> Self {
        Self {
            buffers: HashMap::new(),
        }
    }

    /// Begin capturing steps for a trajectory.
    pub fn begin_capture(&mut self, trajectory_id: &str) {
        self.buffers.insert(trajectory_id.to_string(), Vec::new());
    }

    /// Record a step in the trajectory buffer.
    pub fn capture_step(
        &mut self,
        trajectory_id: &str,
        action: &str,
        observation: &str,
        reward: f64,
    ) {
        let buffer = self.buffers.entry(trajectory_id.to_string()).or_default();

        let step_index = buffer.len();

        // Truncate large observations
        let obs = if observation.len() > MAX_OBSERVATION_LEN {
            &observation[..MAX_OBSERVATION_LEN]
        } else {
            observation
        };

        buffer.push(TrajectoryStep {
            step_id: uuid::Uuid::new_v4().to_string(),
            trajectory_id: trajectory_id.to_string(),
            step_index,
            action: action.to_string(),
            observation: obs.to_string(),
            reward,
            timestamp: epoch_secs(),
        });
    }

    /// End capture and return all steps. Clears the buffer.
    pub fn end_capture(&mut self, trajectory_id: &str) -> Vec<TrajectoryStep> {
        self.buffers.remove(trajectory_id).unwrap_or_default()
    }
}

impl Default for StepCaptureService {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

use super::config::SonaConfig;
use super::constants::{WEIGHT_MAX, WEIGHT_MIN};
use super::errors::SonaError;
use super::helpers::{epoch_secs, pad_or_truncate};
use super::math::{
    calculate_gradient, calculate_reward, calculate_weight_update, cosine_similarity,
    crc32_checksum, update_fisher_information,
};
use super::types::{
    DriftReport, DriftStatus, FeedbackInput, Trajectory, WeightCheckpoint, WeightUpdateResult,
};
use crate::learning::trajectory_store;
use archon_memory::embedding::EmbeddingProvider;
use cozo::DbInstance;
use std::collections::HashMap;
use std::sync::Arc;

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
    /// Optional CozoDB handle for persisting trajectories.
    db: Option<Arc<DbInstance>>,
    /// Optional embedding provider for computing trajectory embeddings.
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
}

impl SonaEngine {
    pub fn new(config: SonaConfig) -> Self {
        let db = config.db.clone();
        let embedding_provider = config.embedding_provider.clone();
        Self {
            config,
            trajectories: HashMap::new(),
            weights: HashMap::new(),
            fisher: HashMap::new(),
            checkpoints: Vec::new(),
            db,
            embedding_provider,
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
            patterns: vec![],
            context: vec![],
            embedding: vec![],
            quality: 0.0,
            reward: 0.0,
            feedback_score: 0.0,
            weights_path: String::new(),
            created_at: now,
            updated_at: now,
        };
        self.trajectories
            .insert(traj.trajectory_id.clone(), traj.clone());

        if let Some(ref db) = self.db {
            let _ = trajectory_store::store_trajectory(db, &traj);
        }

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

        // Embed at first non-zero quality so the trainer sees real vectors.
        if traj.embedding.is_empty()
            && input.quality > 0.0
            && let Some(ref provider) = self.embedding_provider
        {
            let text = format!(
                "route:{}\nagent:{}\npatterns:{}\ncontext:{}",
                traj.route,
                traj.agent_key,
                traj.patterns.join(","),
                traj.context.join("\n"),
            );
            match provider.embed(&[text]) {
                Ok(embeddings) => {
                    if let Some(emb) = embeddings.into_iter().next() {
                        traj.embedding = pad_or_truncate(emb, self.config.gnn_input_dim);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        trajectory_id = %traj.trajectory_id,
                        error = %e,
                        "embedding failed for trajectory"
                    );
                }
            }
        }

        // Persist updated trajectory to CozoDB when available.
        if let Some(ref db) = self.db {
            let _ = trajectory_store::store_trajectory(db, traj);
        }

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

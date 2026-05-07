use super::constants::{
    DEFAULT_DRIFT_ALERT_THRESHOLD, DEFAULT_DRIFT_REJECT_THRESHOLD, DEFAULT_LEARNING_RATE,
    DEFAULT_MAX_CHECKPOINTS, DEFAULT_REGULARIZATION, FISHER_DECAY_RATE,
};
use archon_memory::embedding::EmbeddingProvider;
use cozo::DbInstance;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the SONA Engine.
#[derive(Clone, Serialize, Deserialize)]
pub struct SonaConfig {
    pub learning_rate: f64,
    pub regularization: f64,
    pub drift_alert_threshold: f64,
    pub drift_reject_threshold: f64,
    pub max_checkpoints: usize,
    pub fisher_decay_rate: f64,
    /// Target embedding dimensionality for GNN input (pad/truncate to this).
    pub gnn_input_dim: usize,
    /// Optional CozoDB handle for persisting trajectories.
    #[serde(skip)]
    pub db: Option<Arc<DbInstance>>,
    /// Optional embedding provider for computing trajectory embeddings.
    #[serde(skip)]
    pub embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
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
            gnn_input_dim: 1536,
            db: None,
            embedding_provider: None,
        }
    }
}

// Manual Debug impl - EmbeddingProvider is not Debug.
impl std::fmt::Debug for SonaConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SonaConfig")
            .field("learning_rate", &self.learning_rate)
            .field("regularization", &self.regularization)
            .field("drift_alert_threshold", &self.drift_alert_threshold)
            .field("drift_reject_threshold", &self.drift_reject_threshold)
            .field("max_checkpoints", &self.max_checkpoints)
            .field("fisher_decay_rate", &self.fisher_decay_rate)
            .field("gnn_input_dim", &self.gnn_input_dim)
            .field("db", &self.db.as_ref().map(|_| "Arc<DbInstance>"))
            .field(
                "embedding_provider",
                &self
                    .embedding_provider
                    .as_ref()
                    .map(|_| "Arc<dyn EmbeddingProvider>"),
            )
            .finish()
    }
}

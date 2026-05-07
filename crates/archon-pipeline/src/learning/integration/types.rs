// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the main learning integration layer.
#[derive(Debug, Clone)]
pub struct LearningIntegrationConfig {
    pub track_trajectories: bool,
    pub auto_feedback: bool,
    pub quality_threshold: f64,
    pub route_prefix: String,
    pub enable_hyperedges: bool,
    pub hyperedge_threshold: f64,
}

impl Default for LearningIntegrationConfig {
    fn default() -> Self {
        Self {
            track_trajectories: true,
            auto_feedback: true,
            quality_threshold: 0.8,
            route_prefix: "pipeline/".to_string(),
            enable_hyperedges: false,
            hyperedge_threshold: 0.85,
        }
    }
}

// ---------------------------------------------------------------------------
// LearningContext
// ---------------------------------------------------------------------------

/// Context returned to the pipeline when an agent starts or queries learning.
#[derive(Debug, Clone, Default)]
pub struct LearningContext {
    pub sona_context: String,
    pub reasoning_context: String,
    pub desc_episodes: Vec<String>,
    pub reflexion: Option<String>,
}

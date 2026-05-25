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
    pub autonomous_behaviour_apply: bool,
    pub autonomous_max_risk: archon_learning::models::RiskLevel,
    pub autonomous_min_evidence: usize,
    pub autonomous_max_recent_incidents: usize,
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
            autonomous_behaviour_apply: false,
            autonomous_max_risk: archon_learning::models::RiskLevel::Low,
            autonomous_min_evidence: 3,
            autonomous_max_recent_incidents: 4,
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

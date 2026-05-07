use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default quality degradation threshold for `QualityMonitor`.
pub const DEFAULT_QUALITY_THRESHOLD: f64 = 0.5;

/// Default maximum number of episodes returned by `InjectionFilter`.
pub const DEFAULT_MAX_INJECTION: usize = 3;

/// Default minimum quality score for `InjectionFilter`.
pub const DEFAULT_MIN_INJECTION_QUALITY: f64 = 0.3;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// A DESC episode recording a past pipeline execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DescEpisode {
    pub episode_id: String,
    pub session_id: String,
    pub task_type: String,
    pub description: String,
    pub solution: String,
    pub outcome: String,
    pub quality_score: f64,
    pub reward: f64,
    pub tags: Vec<String>,
    pub trajectory_id: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Query parameters for finding episodes.
#[derive(Debug, Clone)]
pub struct EpisodeQuery {
    pub task_type: Option<String>,
    pub min_quality: Option<f64>,
    pub limit: usize,
}

impl Default for EpisodeQuery {
    fn default() -> Self {
        Self {
            task_type: None,
            min_quality: None,
            limit: 10,
        }
    }
}

/// Result of injection filtering - episodes ranked for prompt injection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InjectionResult {
    pub episodes: Vec<RankedEpisode>,
    pub injection_confidence: f64,
}

/// An episode ranked for injection with a combined score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedEpisode {
    pub episode: DescEpisode,
    /// Combined score: quality_score * similarity.
    pub score: f64,
}

/// Quality monitoring report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityReport {
    pub mean_quality: f64,
    pub min_quality: f64,
    pub max_quality: f64,
    pub total_episodes: usize,
    pub degradation_detected: bool,
    pub degradation_threshold: f64,
}

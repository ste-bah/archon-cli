//! PhD-quality scoring calculator for research agent outputs.
//!
//! Scores research agent outputs across 5 weighted dimensions to produce
//! scores in the 0.30–0.95 range. Replicates the TypeScript
//! `PhDQualityCalculator` scoring logic.

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Contextual metadata about the agent being scored.
#[derive(Clone, Debug)]
pub struct QualityContext {
    pub agent_key: String,
    pub phase: u8,
    pub expected_min_length: Option<usize>,
    pub is_writing_agent: bool,
    pub is_critical_agent: bool,
}

/// Per-dimension score breakdown.
#[derive(Clone, Debug)]
pub struct QualityBreakdown {
    /// Max 0.25
    pub content_depth: f64,
    /// Max 0.20
    pub structural_quality: f64,
    /// Max 0.25
    pub research_rigor: f64,
    /// Max 0.20
    pub completeness: f64,
    /// Max 0.10
    pub format_quality: f64,
}

/// Final quality assessment with breakdown, tier, and summary.
#[derive(Clone, Debug)]
pub struct QualityAssessment {
    /// 0.0–0.95
    pub score: f64,
    pub breakdown: QualityBreakdown,
    pub tier: QualityTier,
    pub summary: String,
}

/// Tier classification based on final score.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QualityTier {
    /// >= 0.85
    Excellent,
    /// >= 0.70
    Good,
    /// >= 0.50
    Adequate,
    /// < 0.50
    Poor,
}

impl std::fmt::Display for QualityTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QualityTier::Excellent => write!(f, "Excellent"),
            QualityTier::Good => write!(f, "Good"),
            QualityTier::Adequate => write!(f, "Adequate"),
            QualityTier::Poor => write!(f, "Poor"),
        }
    }
}

mod score;
mod tables;

pub use score::PhDQualityCalculator;

#[cfg(test)]
mod tests;

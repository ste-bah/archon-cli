//! 9-axis game-theory fingerprint types.
//!
//! The fingerprint is the primary output of Tier 1 classification.
//! It captures the multi-dimensional game structure of a strategic situation.

use serde::{Deserialize, Serialize};

/// Verdict for a single classification axis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AxisVerdict {
    /// The classification value (e.g. "non-cooperative", "zero-sum").
    pub value: String,
    /// Confidence level: "high", "medium", "low".
    pub confidence: String,
    /// Brief rationale for the classification.
    pub rationale: String,
}

impl AxisVerdict {
    pub fn new(
        value: impl Into<String>,
        confidence: impl Into<String>,
        rationale: impl Into<String>,
    ) -> Self {
        Self {
            value: value.into(),
            confidence: confidence.into(),
            rationale: rationale.into(),
        }
    }
}

/// Detection of a hidden game embedded within the stated situation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiddenGameDetection {
    /// Name of the detected hidden game.
    pub game_name: String,
    /// Confidence in the detection.
    pub confidence: String,
    /// Description of the hidden game and its relationship to the stated situation.
    pub description: String,
}

/// A noted ambiguity in the situation classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmbiguityNote {
    /// Which axis or aspect is ambiguous.
    pub axis: String,
    /// Description of the ambiguity.
    pub note: String,
}

/// Complete 9-axis game-theory fingerprint produced by Tier 1 classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameTheoryFingerprint {
    pub run_id: String,
    pub cooperation: AxisVerdict,
    pub payoff_sum: AxisVerdict,
    pub symmetry: AxisVerdict,
    pub timing: AxisVerdict,
    pub perfect_info: AxisVerdict,
    pub complete_info: AxisVerdict,
    pub cardinality: AxisVerdict,
    pub strategy_space: AxisVerdict,
    pub horizon: AxisVerdict,
    pub primary_family: String,
    pub nearest_classic: Option<String>,
    pub shadow_games: Vec<String>,
    pub hidden_game_scan: Option<HiddenGameDetection>,
    pub ambiguities: Vec<AmbiguityNote>,
    pub created_at: String,
}

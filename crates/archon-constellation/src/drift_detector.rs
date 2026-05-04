use serde::{Deserialize, Serialize};

use crate::errors::Result;
use crate::score::{ScoreResult, score_text};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriftReport {
    pub target: String,
    pub centroid_id: String,
    pub version: u32,
    pub similarity: f64,
    pub threshold: f64,
    pub drifted: bool,
    pub reason: String,
}

pub fn detect_drift(
    db: &cozo::DbInstance,
    target: &str,
    text: &str,
    threshold: f64,
) -> Result<DriftReport> {
    let score = score_text(db, target, text)?;
    Ok(report_from_score(score, threshold))
}

fn report_from_score(score: ScoreResult, threshold: f64) -> DriftReport {
    let drifted = score.similarity < threshold;
    let reason = if drifted {
        format!(
            "similarity {:.3} is below threshold {:.3}",
            score.similarity, threshold
        )
    } else {
        format!(
            "similarity {:.3} meets threshold {:.3}",
            score.similarity, threshold
        )
    };
    DriftReport {
        target: score.target,
        centroid_id: score.centroid_id,
        version: score.version,
        similarity: score.similarity,
        threshold,
        drifted,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_marks_drift_below_threshold() {
        let score = ScoreResult {
            target: "project".into(),
            centroid_id: "c1".into(),
            version: 1,
            similarity: 0.2,
            distance: 0.8,
            sample_count: 2,
        };
        assert!(report_from_score(score, 0.5).drifted);
    }
}

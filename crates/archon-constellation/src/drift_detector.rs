use serde::{Deserialize, Serialize};

use crate::bootstrap::{BootstrapSource, bootstrap_centroid, default_bootstrap_source};
use crate::errors::{ConstellationError, Result};
use crate::score::{ScoreResult, score_text};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriftStatus {
    Ready,
    BootstrapPending,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriftReport {
    pub target: String,
    pub centroid_id: String,
    pub version: u32,
    pub status: DriftStatus,
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
    detect_drift_inner(db, target, text, threshold, None)
}

pub fn detect_drift_with_bootstrap_source(
    db: &cozo::DbInstance,
    target: &str,
    text: &str,
    threshold: f64,
    source: BootstrapSource<'_>,
) -> Result<DriftReport> {
    detect_drift_inner(db, target, text, threshold, Some(source))
}

fn detect_drift_inner(
    db: &cozo::DbInstance,
    target: &str,
    text: &str,
    threshold: f64,
    source: Option<BootstrapSource<'_>>,
) -> Result<DriftReport> {
    crate::ensure_schema(db)?;
    match score_text(db, target, text) {
        Ok(score) => Ok(report_from_score(score, threshold)),
        Err(ConstellationError::MissingCentroid(_)) => {
            let source = match source {
                Some(source) => source,
                None => default_bootstrap_source(db, target)?,
            };
            match bootstrap_centroid(db, target, source)? {
                Some(_) => {
                    let score = score_text(db, target, text)?;
                    Ok(report_from_score(score, threshold))
                }
                None => Ok(DriftReport::bootstrap_pending(target, threshold)),
            }
        }
        Err(ConstellationError::InvalidTarget(target)) => {
            Err(ConstellationError::UnknownTarget(target))
        }
        Err(other) => Err(other),
    }
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
        status: DriftStatus::Ready,
        similarity: score.similarity,
        threshold,
        drifted,
        reason,
    }
}

impl DriftReport {
    pub fn bootstrap_pending(target: &str, threshold: f64) -> Self {
        Self {
            target: target.to_string(),
            centroid_id: String::new(),
            version: 0,
            status: DriftStatus::BootstrapPending,
            similarity: 0.0,
            threshold,
            drifted: false,
            reason: format!(
                "bootstrap pending: at least {} representative texts are required",
                crate::MIN_BOOTSTRAP_TEXTS
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BootstrapSource, list_centroids};

    fn db() -> cozo::DbInstance {
        cozo::DbInstance::new("mem", "", Default::default()).unwrap()
    }

    fn texts(count: usize) -> Vec<String> {
        (0..count)
            .map(|idx| format!("memory anchor phrase {idx} with acceptance evidence"))
            .collect()
    }

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

    #[test]
    fn detect_drift_auto_bootstraps_on_first_call() {
        let db = db();
        let texts = texts(5);

        let report = detect_drift_with_bootstrap_source(
            &db,
            "memory",
            "memory anchor phrase with acceptance evidence",
            0.45,
            BootstrapSource::Inline(&texts),
        )
        .unwrap();

        assert_eq!(report.status, DriftStatus::Ready);
        assert_eq!(list_centroids(&db).unwrap().len(), 1);
    }

    #[test]
    fn detect_drift_returns_bootstrap_pending_when_source_too_thin() {
        let db = db();
        let texts = texts(2);

        let report = detect_drift_with_bootstrap_source(
            &db,
            "memory",
            "memory anchor phrase with acceptance evidence",
            0.45,
            BootstrapSource::Inline(&texts),
        )
        .unwrap();

        assert_eq!(report.status, DriftStatus::BootstrapPending);
        assert!(list_centroids(&db).unwrap().is_empty());
    }

    #[test]
    fn detect_drift_uses_existing_centroid_on_subsequent_calls() {
        let db = db();
        let texts = texts(5);

        detect_drift_with_bootstrap_source(
            &db,
            "memory",
            "memory anchor phrase with acceptance evidence",
            0.45,
            BootstrapSource::Inline(&texts),
        )
        .unwrap();
        detect_drift_with_bootstrap_source(
            &db,
            "memory",
            "memory anchor phrase with acceptance evidence",
            0.45,
            BootstrapSource::Inline(&texts),
        )
        .unwrap();

        assert_eq!(list_centroids(&db).unwrap().len(), 1);
    }
}

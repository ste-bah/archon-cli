//! Constellation profiles built from accepted meaning samples.

pub mod bootstrap;
pub mod centroid_builder;
pub mod drift_detector;
pub mod errors;
pub mod schema;
pub mod score;
pub mod store;

use archon_meaning::{MeaningLabel, MeaningSample};
use cozo::DbInstance;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use bootstrap::{BootstrapSource, MIN_BOOTSTRAP_TEXTS, bootstrap_centroid};
pub use drift_detector::{
    DriftReport, DriftStatus, detect_drift, detect_drift_with_bootstrap_source,
};
pub use errors::{ConstellationError, Result};
pub use score::{ScoreResult, score_text};
pub use store::{ConstellationCentroid, LEXICAL_CENTROID_FEATURE_SPACE, list_centroids};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildReport {
    pub target: String,
    pub centroid_id: Option<String>,
    pub version: Option<u32>,
    pub samples_seen: usize,
    pub sample_count: usize,
    pub vector_rows: usize,
}

pub fn ensure_schema(db: &DbInstance) -> Result<()> {
    schema::ensure_schema(db)
}

pub fn build_constellation(db: &DbInstance, target: &str) -> Result<BuildReport> {
    let target = validate_target(target)?;
    ensure_schema(db)?;
    archon_meaning::ensure_schema(db)?;
    let samples = target_positive_samples(db, target)?;
    let samples_seen = archon_meaning::list_samples(db)?.len();
    if samples.is_empty() {
        return Ok(BuildReport {
            target: target.to_string(),
            centroid_id: None,
            version: None,
            samples_seen,
            sample_count: 0,
            vector_rows: store::count_vectors(db)?,
        });
    }
    let texts: Vec<String> = samples.iter().map(|sample| sample.text.clone()).collect();
    let vector = centroid_builder::centroid_vector(&texts)
        .ok_or_else(|| ConstellationError::Store("no vectors produced".into()))?;
    let sample_ids: Vec<String> = samples
        .iter()
        .map(|sample| sample.sample_id.clone())
        .collect();
    let version = store::next_version(db, target)?;
    let centroid_id = stable_id(
        "constellation",
        &[target, &version.to_string(), &sample_ids.join("|")],
    );
    let centroid = ConstellationCentroid {
        centroid_id: centroid_id.clone(),
        target: target.to_string(),
        version,
        vector,
        sample_ids,
        sample_count: samples.len(),
        source_relation: "meaning_samples".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    store::insert_centroid(db, &centroid)?;
    store::insert_vector(db, &centroid.centroid_id, &centroid.vector)?;
    Ok(BuildReport {
        target: target.to_string(),
        centroid_id: Some(centroid_id),
        version: Some(version),
        samples_seen,
        sample_count: samples.len(),
        vector_rows: store::count_vectors(db)?,
    })
}

pub fn target_positive_samples(db: &DbInstance, target: &str) -> Result<Vec<MeaningSample>> {
    let target = validate_target(target)?;
    let samples = archon_meaning::list_samples(db)?;
    Ok(samples
        .into_iter()
        .filter(|sample| sample.label == MeaningLabel::Positive)
        .filter(|sample| target_matches(sample, target))
        .collect())
}

pub fn validate_target(target: &str) -> Result<&str> {
    if is_known_target(target) {
        Ok(target)
    } else {
        Err(ConstellationError::InvalidTarget(target.to_string()))
    }
}

pub(crate) fn is_known_target(target: &str) -> bool {
    matches!(
        target,
        "project" | "research-domain" | "strategic-workflow" | "memory" | "docs" | "session"
    )
}

pub fn stable_id(prefix: &str, parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prefix.as_bytes());
    for part in parts {
        hasher.update([0]);
        hasher.update(part.as_bytes());
    }
    format!("{prefix}-{}", hex::encode(&hasher.finalize()[..12]))
}

fn target_matches(sample: &MeaningSample, target: &str) -> bool {
    match target {
        "strategic-workflow" => {
            sample.workspace_id == "gametheory-runs"
                || sample.event_type.contains("GameTheory")
                || sample
                    .metadata_json
                    .get("source")
                    .and_then(serde_json::Value::as_str)
                    == Some("gt_final_reports")
        }
        "research-domain" => {
            sample.event_type.contains("Claim")
                || sample.event_type.contains("Citation")
                || sample
                    .metadata_json
                    .get("domain")
                    .and_then(serde_json::Value::as_str)
                    == Some("research")
        }
        "project" => sample.workspace_id != "gametheory-runs",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_ids_are_deterministic() {
        assert_eq!(
            stable_id("constellation", &["project", "1"]),
            stable_id("constellation", &["project", "1"])
        );
    }

    #[test]
    fn strategic_target_matches_gametheory_samples() {
        let sample = MeaningSample {
            sample_id: "s1".into(),
            workspace_id: "gametheory-runs".into(),
            artifact_id: "a1".into(),
            label: MeaningLabel::Positive,
            source_event_id: "e1".into(),
            event_type: "GameTheoryFinalReport".into(),
            text: "mechanism recommendation".into(),
            metadata_json: serde_json::json!({"source": "gt_final_reports"}),
            created_at: "now".into(),
        };
        assert!(target_matches(&sample, "strategic-workflow"));
        assert!(!target_matches(&sample, "project"));
    }

    #[test]
    fn invalid_targets_are_rejected() {
        assert!(matches!(
            validate_target("projct"),
            Err(ConstellationError::InvalidTarget(_))
        ));
    }
}

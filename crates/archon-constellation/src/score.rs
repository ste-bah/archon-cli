use serde::{Deserialize, Serialize};

use crate::centroid_builder::{cosine_similarity, text_vector};
use crate::errors::{ConstellationError, Result};
use crate::store;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreResult {
    pub target: String,
    pub centroid_id: String,
    pub version: u32,
    pub similarity: f64,
    pub distance: f64,
    pub sample_count: usize,
    pub feature_space: String,
    pub scoring_source: String,
}

pub fn score_text(db: &cozo::DbInstance, target: &str, text: &str) -> Result<ScoreResult> {
    let target = crate::validate_target(target)?;
    if text.trim().is_empty() {
        return Err(ConstellationError::EmptyInput);
    }
    let centroid = store::latest_centroid(db, target)?
        .ok_or_else(|| ConstellationError::MissingCentroid(target.to_string()))?;
    let query = text_vector(text);
    let (similarity, distance, scoring_source) = match store::query_vectors(db, &query, 16) {
        Ok(rows) => rows
            .into_iter()
            .find(|(centroid_id, _)| centroid_id == &centroid.centroid_id)
            .map(|(_, distance)| {
                (
                    (1.0 - distance).clamp(-1.0, 1.0),
                    distance,
                    "vec_constellations:constellation_embedding_idx".to_string(),
                )
            })
            .unwrap_or_else(|| {
                let similarity = cosine_similarity(&centroid.vector, &query);
                (
                    similarity,
                    1.0 - similarity,
                    "constellation_centroids.vector_json".to_string(),
                )
            }),
        Err(error) => {
            tracing::warn!(error = %error, "constellation vector index unavailable; falling back to stored centroid JSON");
            let similarity = cosine_similarity(&centroid.vector, &query);
            (
                similarity,
                1.0 - similarity,
                "constellation_centroids.vector_json".to_string(),
            )
        }
    };
    Ok(ScoreResult {
        target: centroid.target,
        centroid_id: centroid.centroid_id,
        version: centroid.version,
        similarity,
        distance,
        sample_count: centroid.sample_count,
        feature_space: store::LEXICAL_CENTROID_FEATURE_SPACE.to_string(),
        scoring_source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema;

    #[test]
    fn empty_input_is_rejected_before_store_lookup() {
        let db = cozo::DbInstance::new("mem", "", Default::default()).unwrap();
        schema::ensure_schema(&db).unwrap();
        assert!(matches!(
            score_text(&db, "project", " "),
            Err(ConstellationError::EmptyInput)
        ));
    }
}

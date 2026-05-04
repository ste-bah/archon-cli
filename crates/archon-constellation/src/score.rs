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
}

pub fn score_text(db: &cozo::DbInstance, target: &str, text: &str) -> Result<ScoreResult> {
    let target = crate::validate_target(target)?;
    if text.trim().is_empty() {
        return Err(ConstellationError::EmptyInput);
    }
    let centroid = store::latest_centroid(db, target)?
        .ok_or_else(|| ConstellationError::MissingCentroid(target.to_string()))?;
    let query = text_vector(text);
    let similarity = cosine_similarity(&centroid.vector, &query);
    Ok(ScoreResult {
        target: centroid.target,
        centroid_id: centroid.centroid_id,
        version: centroid.version,
        similarity,
        distance: 1.0 - similarity,
        sample_count: centroid.sample_count,
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

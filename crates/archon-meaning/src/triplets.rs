use serde::{Deserialize, Serialize};

use crate::contrastive::ContrastivePair;
use crate::errors::{MeaningError, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TripletRecord {
    pub triplet_id: String,
    pub workspace_id: String,
    pub anchor_artifact_id: String,
    pub positive_sample_id: String,
    pub negative_sample_id: String,
    pub created_at: String,
}

impl TripletRecord {
    pub fn validate(&self) -> Result<()> {
        if self.positive_sample_id == self.negative_sample_id {
            return Err(MeaningError::InvalidTriplet(
                "positive and negative samples must differ".into(),
            ));
        }
        if self.anchor_artifact_id.is_empty() {
            return Err(MeaningError::InvalidTriplet(
                "anchor artifact must be present".into(),
            ));
        }
        Ok(())
    }
}

pub fn build_triplets(pairs: &[ContrastivePair], now: &str) -> Vec<TripletRecord> {
    pairs
        .iter()
        .map(|pair| TripletRecord {
            triplet_id: crate::stable_id(
                "triplet",
                &[
                    &pair.anchor_artifact_id,
                    &pair.positive_sample_id,
                    &pair.negative_sample_id,
                ],
            ),
            workspace_id: pair.workspace_id.clone(),
            anchor_artifact_id: pair.anchor_artifact_id.clone(),
            positive_sample_id: pair.positive_sample_id.clone(),
            negative_sample_id: pair.negative_sample_id.clone(),
            created_at: now.to_string(),
        })
        .filter(|triplet| triplet.validate().is_ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_triplet_with_same_positive_and_negative() {
        let triplet = TripletRecord {
            triplet_id: "t".into(),
            workspace_id: "ws".into(),
            anchor_artifact_id: "artifact".into(),
            positive_sample_id: "same".into(),
            negative_sample_id: "same".into(),
            created_at: "now".into(),
        };
        assert!(triplet.validate().is_err());
    }
}

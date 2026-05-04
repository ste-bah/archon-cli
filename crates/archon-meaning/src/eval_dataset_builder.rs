use serde::{Deserialize, Serialize};

use crate::samples::MeaningSample;
use crate::triplets::TripletRecord;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalDataset {
    pub dataset_id: String,
    pub sample_count: usize,
    pub triplet_count: usize,
    pub created_at: String,
}

pub fn build_dataset(
    samples: &[MeaningSample],
    triplets: &[TripletRecord],
    now: &str,
) -> EvalDataset {
    EvalDataset {
        dataset_id: crate::stable_id(
            "eval",
            &[&samples.len().to_string(), &triplets.len().to_string(), now],
        ),
        sample_count: samples.len(),
        triplet_count: triplets.len(),
        created_at: now.to_string(),
    }
}

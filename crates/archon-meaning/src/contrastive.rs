use serde::{Deserialize, Serialize};

use crate::samples::{MeaningLabel, MeaningSample};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContrastivePair {
    pub pair_id: String,
    pub workspace_id: String,
    pub positive_sample_id: String,
    pub negative_sample_id: String,
    pub anchor_artifact_id: String,
    pub created_at: String,
}

pub fn build_pairs(samples: &[MeaningSample], now: &str) -> Vec<ContrastivePair> {
    let positives = samples
        .iter()
        .filter(|sample| sample.label == MeaningLabel::Positive);
    let negatives: Vec<_> = samples
        .iter()
        .filter(|sample| sample.label == MeaningLabel::Negative)
        .collect();
    positives
        .flat_map(|positive| {
            negatives
                .iter()
                .filter(move |negative| negative.workspace_id == positive.workspace_id)
                .map(move |negative| ContrastivePair {
                    pair_id: crate::stable_id("pair", &[&positive.sample_id, &negative.sample_id]),
                    workspace_id: positive.workspace_id.clone(),
                    positive_sample_id: positive.sample_id.clone(),
                    negative_sample_id: negative.sample_id.clone(),
                    anchor_artifact_id: positive.artifact_id.clone(),
                    created_at: now.to_string(),
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str, workspace: &str, label: MeaningLabel) -> MeaningSample {
        MeaningSample {
            sample_id: id.into(),
            workspace_id: workspace.into(),
            artifact_id: format!("artifact-{id}"),
            label,
            source_event_id: format!("event-{id}"),
            event_type: "UserAccepted".into(),
            text: id.into(),
            metadata_json: serde_json::json!({}),
            created_at: "now".into(),
        }
    }

    #[test]
    fn pairs_positive_and_negative_in_same_workspace() {
        let pairs = build_pairs(
            &[
                sample("p", "ws", MeaningLabel::Positive),
                sample("n", "ws", MeaningLabel::Negative),
                sample("other", "else", MeaningLabel::Negative),
            ],
            "now",
        );
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].positive_sample_id, "p");
        assert_eq!(pairs[0].negative_sample_id, "n");
    }
}

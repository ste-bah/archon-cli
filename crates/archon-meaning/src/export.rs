use serde::Serialize;

use crate::samples::MeaningSample;
use crate::triplets::TripletRecord;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExportRow<'a> {
    pub row_type: &'a str,
    pub id: &'a str,
    pub workspace_id: &'a str,
    pub text: Option<&'a str>,
    pub positive_sample_id: Option<&'a str>,
    pub negative_sample_id: Option<&'a str>,
}

pub fn samples_jsonl(samples: &[MeaningSample]) -> crate::Result<String> {
    let mut lines = Vec::new();
    for sample in samples {
        lines.push(serde_json::to_string(&ExportRow {
            row_type: "sample",
            id: &sample.sample_id,
            workspace_id: &sample.workspace_id,
            text: Some(&sample.text),
            positive_sample_id: None,
            negative_sample_id: None,
        })?);
    }
    Ok(lines.join("\n"))
}

pub fn triplets_jsonl(triplets: &[TripletRecord]) -> crate::Result<String> {
    let mut lines = Vec::new();
    for triplet in triplets {
        lines.push(serde_json::to_string(&ExportRow {
            row_type: "triplet",
            id: &triplet.triplet_id,
            workspace_id: &triplet.workspace_id,
            text: Some(&triplet.anchor_artifact_id),
            positive_sample_id: Some(&triplet.positive_sample_id),
            negative_sample_id: Some(&triplet.negative_sample_id),
        })?);
    }
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::samples::MeaningLabel;

    #[test]
    fn sample_export_has_jsonl_shape() {
        let sample = MeaningSample {
            sample_id: "s".into(),
            workspace_id: "ws".into(),
            artifact_id: "a".into(),
            label: MeaningLabel::Positive,
            source_event_id: "e".into(),
            event_type: "UserAccepted".into(),
            text: "accepted output".into(),
            metadata_json: serde_json::json!({}),
            created_at: "now".into(),
        };
        let jsonl = samples_jsonl(&[sample]).unwrap();
        assert!(jsonl.contains("\"row_type\":\"sample\""));
        assert!(jsonl.contains("accepted output"));
    }
}

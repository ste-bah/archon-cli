use std::fmt::Display;

use archon_workflow::{
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2ResidualGap, WorkflowV2Result,
    WorkflowV2Status,
};

pub(super) fn failed_v2_result(call_id: &str, err: impl Display) -> WorkflowV2Result {
    let error = err.to_string();
    WorkflowV2Result {
        status: WorkflowV2Status::Failed,
        summary: format!("workflow v2 call '{call_id}' failed: {error}"),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Blocker,
            error.clone(),
        )],
        residual_gaps: vec![WorkflowV2ResidualGap {
            id: format!("v2_call_failed_{}", sanitize_v2_gap_id(call_id)),
            description: error.clone(),
            severity: Some("blocking".to_string()),
        }],
        data: serde_json::json!({ "error": error }),
        ..WorkflowV2Result::default()
    }
}

fn sanitize_v2_gap_id(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

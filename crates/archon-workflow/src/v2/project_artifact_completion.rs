//! Declared artifact contract enforcement.
//!
//! One contract source: the paths validated here are exactly the paths handed
//! to the agent — the same extraction that builds the prompt's resolved
//! artifact section (`project_artifact_prompt::declared_project_artifact_entries`)
//! drives validation, so an artifact can only be demanded if the agent was
//! instructed to produce it. No inference, no heuristic recovery of intent.
//!
//! A missing declared artifact produces a failed result VALUE for the call —
//! never a run-level block.

use std::path::Path;

use super::host_api::WorkflowV2ArtifactRequirement;
use super::project_artifact_prompt::declared_project_artifact_entries;
use super::{
    WorkflowV2Artifact, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2ProjectArtifactContext, WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2Status,
};

pub(super) fn enforce_declared_artifact_requirements(
    item_id: &str,
    input: &serde_json::Value,
    required_artifacts: &[WorkflowV2ArtifactRequirement],
    result: &mut WorkflowV2Result,
    context: &WorkflowV2ProjectArtifactContext,
) {
    if context.is_empty() {
        return;
    }
    let declared = declared_project_artifact_entries(input, required_artifacts, context);
    if declared.is_empty() {
        return;
    }
    let mut missing = Vec::new();
    for (raw, absolute) in &declared {
        if Path::new(absolute).exists() {
            record_declared_artifact(result, raw);
        } else {
            missing.push(raw.clone());
        }
    }
    if missing.is_empty() {
        return;
    }
    result.status = WorkflowV2Status::Failed;
    result.summary = format!(
        "declared project artifacts missing for '{item_id}': {}",
        missing.join(", ")
    );
    if let serde_json::Value::Object(data) = &mut result.data {
        data.insert(
            "missing_required_artifacts".to_string(),
            serde_json::json!(missing),
        );
    } else if result.data.is_null() {
        result.data = serde_json::json!({ "missing_required_artifacts": missing });
    }
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!("missing_declared_artifacts_{}", sanitize_gap_id(item_id)),
        description: format!(
            "declared artifact contract not satisfied; missing: {}",
            missing.join(", ")
        ),
        severity: Some("failed".to_string()),
    });
}

fn record_declared_artifact(result: &mut WorkflowV2Result, path: &str) {
    if !result
        .artifacts
        .iter()
        .any(|existing| existing.path == path)
    {
        result.artifacts.push(WorkflowV2Artifact {
            id: artifact_id(path),
            path: path.to_string(),
            description: Some("declared project artifact".to_string()),
        });
    }
    result.evidence.push(WorkflowV2Evidence {
        kind: WorkflowV2EvidenceKind::Artifact,
        summary: format!("existing required project artifact: {path}"),
        source: Some(path.to_string()),
    });
}

fn artifact_id(path: &str) -> String {
    let id = path
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    id.trim_matches('-').to_string()
}

fn sanitize_gap_id(raw: &str) -> String {
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

use super::*;

pub(crate) fn execute_local_host_call(
    execution: &WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
    task_universe: Option<&WorkflowV2TaskUniverse>,
) -> archon_workflow::WorkflowResult<Option<WorkflowV2Result>> {
    let result = match execution.call.method {
        WorkflowV2HostMethod::Checkpoint => checkpoint_result(execution),
        WorkflowV2HostMethod::SaveArtifact => save_artifact_result(execution, v2_store)?,
        WorkflowV2HostMethod::RequireArtifact => require_artifact_result(execution, v2_store)?,
        WorkflowV2HostMethod::FinalReport => {
            final_report_result(execution, v2_store, task_universe)?
        }
        WorkflowV2HostMethod::QualityGate => {
            quality_gate_result(execution, v2_store, task_universe)?
        }
        WorkflowV2HostMethod::HumanGate => human_gate_result(execution),
        _ => return Ok(None),
    };
    Ok(Some(result))
}

fn checkpoint_result(execution: &WorkflowV2CallExecution) -> WorkflowV2Result {
    let mut result = WorkflowV2Result::accepted(format!(
        "checkpoint '{}' recorded typed workflow state",
        execution.call.id
    ));
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        "checkpoint host call persisted by the workflow V2 result store",
    ));
    result.data = execution.input.clone();
    result
}

fn save_artifact_result(
    execution: &WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let artifact_path = artifact_path(v2_store.root(), &execution.call.id);
    let payload = execution
        .input
        .get("source_data")
        .unwrap_or(&execution.input);
    write_json(&artifact_path, payload)?;
    let mut result = WorkflowV2Result::accepted(format!(
        "artifact '{}' saved by workflow host API",
        execution.call.id
    ));
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Artifact,
        format!("saved typed artifact to {}", artifact_path.display()),
    ));
    result.artifacts.push(WorkflowV2Artifact {
        id: execution.call.id.clone(),
        path: artifact_path.display().to_string(),
        description: Some("workflow V2 saved artifact".to_string()),
    });
    result.data = serde_json::json!({
        "artifact_id": execution.call.id,
        "path": artifact_path,
    });
    Ok(result)
}

fn require_artifact_result(
    execution: &WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    // Declared artifact paths are project-artifact-root relative; resolve them
    // against the project root that owns this run's `.archon` directory, never
    // against the process working directory.
    let project_root = v2_store
        .root()
        .ancestors()
        .find(|ancestor| ancestor.file_name().is_some_and(|name| name == ".archon"))
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf);
    let paths: Vec<std::path::PathBuf> = artifact_paths_from_input(&execution.input)
        .into_iter()
        .map(|path| match (&project_root, path.is_relative()) {
            (Some(root), true) => root.join(path),
            _ => path,
        })
        .collect();
    if paths.is_empty() {
        let mut result = WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: format!(
                "required artifact '{}' did not include concrete artifact path(s); remediation or investigation is required",
                execution.call.id
            ),
            ..WorkflowV2Result::default()
        };
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Artifact,
            "required artifact host check found no explicit artifact path in source data",
        ));
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id: format!("required_artifact_paths_missing_{}", execution.call.id),
            severity: Some("remediation".to_string()),
            description:
                "requireArtifact needs concrete artifact paths from source data before acceptance"
                    .to_string(),
        });
        result.data = serde_json::json!({
            "paths": [],
            "choices": required_artifact_review_choices(&execution.call.id),
        });
        return Ok(result);
    }
    let missing = paths
        .iter()
        .filter(|path| !path.exists())
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        let mut result =
            WorkflowV2Result::accepted(format!("required artifact '{}' exists", execution.call.id));
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Artifact,
            "required artifact host check found concrete artifact path(s)",
        ));
        result
            .artifacts
            .extend(paths.iter().map(|path| WorkflowV2Artifact {
                id: execution.call.id.clone(),
                path: path.display().to_string(),
                description: Some("required workflow V2 artifact".to_string()),
            }));
        result.data = serde_json::json!({ "paths": paths });
        return Ok(result);
    }

    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: format!(
            "required artifact '{}' is missing {} path(s); remediation or user choice is required",
            execution.call.id,
            missing.len()
        ),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "required artifact host check produced typed missing-artifact remediation input",
    ));
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!("missing_artifact_{}", sanitize_id(&execution.call.id)),
        description: format!(
            "missing required artifact path(s): {}",
            missing
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        severity: Some("remediation".to_string()),
    });
    result.data = serde_json::json!({
        "missing_paths": missing,
        "choices": [
            {
                "id": "generate_artifact",
                "label": "Generate missing artifact",
                "action": "continue_with_remediation"
            },
            {
                "id": "retry_after_fix",
                "label": "Retry after dependency fix",
                "action": "restart_call",
                "call_id": execution.call.id
            },
            {
                "id": "record_residual_gap",
                "label": "Record as residual gap",
                "action": "continue_with_gap"
            }
        ]
    });
    Ok(result)
}

fn required_artifact_review_choices(call_id: &str) -> serde_json::Value {
    serde_json::json!([
        {
            "id": "discover_artifact_paths",
            "label": "Discover artifact paths",
            "action": "continue_with_investigation"
        },
        {
            "id": "retry_after_fix",
            "label": "Retry after dependency fix",
            "action": "restart_call",
            "call_id": call_id
        },
        {
            "id": "record_residual_gap",
            "label": "Record as residual gap",
            "action": "continue_with_gap"
        }
    ])
}

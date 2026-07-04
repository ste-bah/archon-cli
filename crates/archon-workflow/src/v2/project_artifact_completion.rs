use super::{
    WorkflowV2Artifact, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2ProjectArtifactContext, WorkflowV2Result, WorkflowV2Status, WorkflowV2TaskCoverage,
    WorkflowV2TaskCoverageStatus, WorkflowV2WriteSafetyError, has_project_artifact_evidence,
    normalize_project_artifact_files,
};

pub(super) fn complete_project_artifact_requirements(
    item_id: &str,
    input: &serde_json::Value,
    result: &mut WorkflowV2Result,
    context: &WorkflowV2ProjectArtifactContext,
) -> Result<(), WorkflowV2WriteSafetyError> {
    if context.is_empty()
        || !matches!(
            result.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop
        )
    {
        return Ok(());
    }
    let mut completed = Vec::new();
    for path in artifact_requirement_paths(input) {
        collect_existing_requirement(item_id, &path, result, context, &mut completed)?;
    }
    if completed.is_empty() {
        return Ok(());
    }
    add_artifact_evidence(result, &completed);
    add_missing_task_coverage(result, &canonical_task_ids(input), &completed);
    Ok(())
}

fn collect_existing_requirement(
    item_id: &str,
    path: &str,
    result: &mut WorkflowV2Result,
    context: &WorkflowV2ProjectArtifactContext,
    completed: &mut Vec<String>,
) -> Result<(), WorkflowV2WriteSafetyError> {
    let mut probe = WorkflowV2Result::accepted("project artifact requirement probe");
    probe.artifacts.push(WorkflowV2Artifact {
        id: artifact_id(path),
        path: path.to_string(),
        description: Some("required project artifact".to_string()),
    });
    normalize_project_artifact_files(item_id, &mut probe, context)?;
    if has_project_artifact_evidence(&probe, context) {
        for artifact in probe.artifacts {
            push_artifact(result, artifact, completed);
        }
    } else if !probe.residual_gaps.is_empty() {
        result.residual_gaps.extend(probe.residual_gaps);
        result.status = WorkflowV2Status::NeedsReview;
    }
    Ok(())
}

fn add_artifact_evidence(result: &mut WorkflowV2Result, paths: &[String]) {
    for path in paths {
        result.evidence.push(WorkflowV2Evidence {
            kind: WorkflowV2EvidenceKind::Artifact,
            summary: format!("existing required project artifact: {path}"),
            source: Some(path.clone()),
        });
    }
}

fn add_missing_task_coverage(result: &mut WorkflowV2Result, task_ids: &[String], paths: &[String]) {
    for task_id in task_ids {
        if result
            .task_coverage
            .iter()
            .any(|coverage| coverage.task_id == *task_id)
        {
            continue;
        }
        result.task_coverage.push(WorkflowV2TaskCoverage {
            task_id: task_id.clone(),
            status: coverage_status(result.status),
            summary: "required project artifact evidence exists".to_string(),
            evidence: paths
                .iter()
                .map(|path| WorkflowV2Evidence {
                    kind: WorkflowV2EvidenceKind::Artifact,
                    summary: format!("project artifact evidence: {path}"),
                    source: Some(path.clone()),
                })
                .collect(),
        });
    }
}

fn coverage_status(status: WorkflowV2Status) -> WorkflowV2TaskCoverageStatus {
    match status {
        WorkflowV2Status::Noop => WorkflowV2TaskCoverageStatus::Noop,
        _ => WorkflowV2TaskCoverageStatus::Accepted,
    }
}

fn push_artifact(
    result: &mut WorkflowV2Result,
    artifact: WorkflowV2Artifact,
    completed: &mut Vec<String>,
) {
    if !result
        .artifacts
        .iter()
        .any(|existing| existing.path == artifact.path)
    {
        result.artifacts.push(artifact.clone());
    }
    if !completed.iter().any(|path| path == &artifact.path) {
        completed.push(artifact.path);
    }
}

fn artifact_requirement_paths(value: &serde_json::Value) -> Vec<String> {
    let mut paths = Vec::new();
    collect_paths(value, &mut paths);
    paths
}

fn collect_paths(value: &serde_json::Value, paths: &mut Vec<String>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_paths(item, paths);
            }
        }
        serde_json::Value::Object(object) => collect_object_paths(object, paths),
        _ => {}
    }
}

fn collect_object_paths(
    object: &serde_json::Map<String, serde_json::Value>,
    paths: &mut Vec<String>,
) {
    for key in [
        "artifact_requirements",
        "project_artifact_requirements",
        "required_artifacts",
    ] {
        if let Some(value) = object.get(key) {
            collect_value_paths(value, paths);
        }
    }
    for value in object.values() {
        collect_paths(value, paths);
    }
}

fn collect_value_paths(value: &serde_json::Value, paths: &mut Vec<String>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_value_paths(item, paths);
            }
        }
        serde_json::Value::Object(object) => {
            if let Some(path) = object.get("path").and_then(serde_json::Value::as_str) {
                paths.push(path.to_string());
            }
        }
        serde_json::Value::String(path) => paths.push(path.to_string()),
        _ => {}
    }
}

fn canonical_task_ids(value: &serde_json::Value) -> Vec<String> {
    let mut task_ids = Vec::new();
    collect_task_ids(value, &mut task_ids);
    task_ids.sort();
    task_ids.dedup();
    task_ids
}

fn collect_task_ids(value: &serde_json::Value, task_ids: &mut Vec<String>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_task_ids(item, task_ids);
            }
        }
        serde_json::Value::Object(object) => collect_object_task_ids(object, task_ids),
        _ => {}
    }
}

fn collect_object_task_ids(
    object: &serde_json::Map<String, serde_json::Value>,
    task_ids: &mut Vec<String>,
) {
    for key in ["canonical_task_ids", "task_ids"] {
        if let Some(value) = object.get(key) {
            collect_value_task_ids(value, task_ids);
        }
    }
    for key in ["canonical_task_id", "task_id"] {
        if let Some(task_id) = object.get(key).and_then(serde_json::Value::as_str) {
            task_ids.push(task_id.to_string());
        }
    }
    for value in object.values() {
        collect_task_ids(value, task_ids);
    }
}

fn collect_value_task_ids(value: &serde_json::Value, task_ids: &mut Vec<String>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_value_task_ids(item, task_ids);
            }
        }
        serde_json::Value::String(task_id) => task_ids.push(task_id.to_string()),
        _ => {}
    }
}

fn artifact_id(path: &str) -> String {
    let id = path
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    id.trim_matches('-').to_string()
}

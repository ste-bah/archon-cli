use std::fs;
use std::path::{Path, PathBuf};

use archon_workflow::{
    WorkflowError, WorkflowV2Artifact, WorkflowV2CallExecution, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2FinalReportBuilder, WorkflowV2HostMethod,
    WorkflowV2ReportPaths, WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2ResultStore,
    WorkflowV2Status,
};

pub(super) fn execute_local_host_call(
    execution: &WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<Option<WorkflowV2Result>> {
    let result = match execution.call.method {
        WorkflowV2HostMethod::Checkpoint => checkpoint_result(execution),
        WorkflowV2HostMethod::SaveArtifact => save_artifact_result(execution, v2_store)?,
        WorkflowV2HostMethod::RequireArtifact => require_artifact_result(execution, v2_store)?,
        WorkflowV2HostMethod::FinalReport => final_report_result(execution, v2_store)?,
        WorkflowV2HostMethod::QualityGate => quality_gate_result(execution, v2_store)?,
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
    write_json(&artifact_path, &execution.input)?;
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
    let paths = artifact_paths_from_input(&execution.input);
    let default_path = artifact_path(v2_store.root(), &execution.call.id);
    let paths = if paths.is_empty() {
        vec![default_path]
    } else {
        paths
    };
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

fn final_report_result(
    execution: &WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let source_results = source_results(execution, v2_store)?;
    let required_task_ids = required_task_ids_from_results(&source_results);
    let paths = report_paths(v2_store.root());
    let report = WorkflowV2FinalReportBuilder::new()
        .build(paths, &required_task_ids, &source_results)
        .map_err(|err| WorkflowError::SpecInvalid(err.to_string()))?;
    let report_path = artifact_path(
        v2_store.root(),
        &format!("{}-final-report", execution.call.id),
    );
    write_json(&report_path, &report)?;

    let mut result = WorkflowV2Result {
        status: report.status,
        summary: format!(
            "final report '{}' produced status {:?}",
            execution.call.id, report.status
        ),
        artifacts: vec![WorkflowV2Artifact {
            id: execution.call.id.clone(),
            path: report_path.display().to_string(),
            description: Some("workflow V2 final acceptance report".to_string()),
        }],
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "final report was derived from typed prior host-call results",
    ));
    if report.status != WorkflowV2Status::Accepted {
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Review,
            "final report contains failed, review-needed, missing, residual, or unverified work",
        ));
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id: format!(
                "final_report_not_accepted_{}",
                sanitize_id(&execution.call.id)
            ),
            description: format!(
                "failed={:?}; blocked={:?}; missing={:?}; residual_gaps={}",
                report.failed_tasks,
                report.blocked_tasks,
                report.missing_tasks,
                report.residual_gaps.len()
            ),
            severity: Some("review".to_string()),
        });
    }
    result.data = serde_json::to_value(report)?;
    Ok(result)
}

fn quality_gate_result(
    execution: &WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<WorkflowV2Result> {
    let source_results = source_results(execution, v2_store)?;
    let failed = source_results
        .iter()
        .filter(|result| {
            matches!(
                result.status,
                WorkflowV2Status::Failed
                    | WorkflowV2Status::Blocked
                    | WorkflowV2Status::NeedsReview
                    | WorkflowV2Status::Cancelled
            )
        })
        .count();
    if failed == 0 && !source_results.is_empty() {
        let mut result = WorkflowV2Result::accepted(format!(
            "quality gate '{}' accepted {} typed result(s)",
            execution.call.id,
            source_results.len()
        ));
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Review,
            "quality gate checked typed input statuses",
        ));
        result.data = serde_json::json!({ "checked": source_results.len() });
        return Ok(result);
    }
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: format!(
            "quality gate '{}' needs review with {} non-accepted input(s)",
            execution.call.id, failed
        ),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "quality gate produced typed remediation or user-choice input for non-accepted results",
    ));
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!("quality_gate_{}", sanitize_id(&execution.call.id)),
        description: "quality gate input set is empty or contains non-accepted results".to_string(),
        severity: Some("review".to_string()),
    });
    result.data = serde_json::json!({
        "checked": source_results.len(),
        "failed": failed,
        "choices": [
            {
                "id": "run_remediation",
                "label": "Run remediation",
                "action": "continue_with_remediation"
            },
            {
                "id": "restart_inputs",
                "label": "Restart upstream inputs",
                "action": "restart_sources",
                "sources": execution.call.options.source
            },
            {
                "id": "accept_residual_gaps",
                "label": "Accept residual gaps",
                "action": "continue_with_gap"
            }
        ]
    });
    Ok(result)
}

fn human_gate_result(execution: &WorkflowV2CallExecution) -> WorkflowV2Result {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: format!("human gate '{}' requires a user choice", execution.call.id),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "humanGate produced structured choices instead of a generic blocked result",
    ));
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!("human_gate_{}", sanitize_id(&execution.call.id)),
        description: "human choice is required before this workflow can be accepted".to_string(),
        severity: Some("human_decision".to_string()),
    });
    result.data = serde_json::json!({
        "choices": [
            {
                "id": "approve_continue",
                "label": "Approve and continue",
                "action": "approve"
            },
            {
                "id": "request_remediation",
                "label": "Request remediation",
                "action": "continue_with_remediation"
            },
            {
                "id": "cancel_workflow",
                "label": "Cancel workflow",
                "action": "cancel"
            }
        ]
    });
    result
}

fn source_results(
    execution: &WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<Vec<WorkflowV2Result>> {
    if let Some(source_data) = execution.input.get("source_data") {
        let mut results = Vec::new();
        collect_source_results(source_data, &mut results)?;
        return Ok(results);
    }
    if let Some(inputs) = execution.input.get("inputs") {
        let mut results = Vec::new();
        collect_source_results(inputs, &mut results)?;
        return Ok(results);
    }
    let Some(source) = execution.call.options.source.as_deref() else {
        return Ok(Vec::new());
    };
    let mut results = Vec::new();
    for call_id in source_call_ids(source) {
        if let Some(record) = v2_store.load_call_record(&call_id)? {
            results.push(record.result);
        }
    }
    Ok(results)
}

fn collect_source_results(
    value: &serde_json::Value,
    results: &mut Vec<WorkflowV2Result>,
) -> archon_workflow::WorkflowResult<()> {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_source_results(item, results)?;
            }
        }
        serde_json::Value::Object(object) => {
            if let Some(result) = object.get("result") {
                results.push(serde_json::from_value(result.clone())?);
            } else if object.contains_key("status") && object.contains_key("summary") {
                results.push(serde_json::from_value(value.clone())?);
            } else if let Some(inputs) = object.get("inputs") {
                collect_source_results(inputs, results)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn source_call_ids(source: &str) -> Vec<String> {
    let trimmed = source.trim();
    let body = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(trimmed);
    body.split(',')
        .map(|part| {
            part.trim()
                .split_once('.')
                .map(|(head, _)| head)
                .unwrap_or_else(|| part.trim())
                .trim_matches(|ch| ch == '"' || ch == '\'')
                .to_string()
        })
        .filter(|part| !part.is_empty())
        .collect()
}

fn required_task_ids_from_results(results: &[WorkflowV2Result]) -> Vec<String> {
    let mut ids = results
        .iter()
        .flat_map(|result| result.task_coverage.iter())
        .map(|coverage| coverage.task_id.clone())
        .filter(|task_id| !task_id.trim().is_empty())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    ids
}

fn report_paths(v2_root: &Path) -> WorkflowV2ReportPaths {
    let run_root = v2_root.parent().unwrap_or(v2_root);
    WorkflowV2ReportPaths {
        harness_path: run_root.join("workflow.js").display().to_string(),
        run_state_path: run_root.join("state.json").display().to_string(),
        event_log_path: run_root.join("events.jsonl").display().to_string(),
    }
}

fn artifact_paths_from_input(input: &serde_json::Value) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    collect_artifact_paths(input, &mut paths);
    paths
}

fn collect_artifact_paths(value: &serde_json::Value, paths: &mut Vec<PathBuf>) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_artifact_paths(item, paths);
            }
        }
        serde_json::Value::Object(object) => {
            if let Some(path) = object.get("path").and_then(serde_json::Value::as_str) {
                paths.push(PathBuf::from(path));
            }
            if let Some(items) = object.get("artifacts") {
                collect_artifact_paths(items, paths);
            }
            if let Some(source_data) = object.get("source_data") {
                collect_artifact_paths(source_data, paths);
            }
        }
        serde_json::Value::String(path) => paths.push(PathBuf::from(path)),
        _ => {}
    }
}

fn artifact_path(v2_root: &Path, id: &str) -> PathBuf {
    v2_root
        .join("artifacts")
        .join(format!("{}.json", sanitize_id(id)))
}

fn sanitize_id(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> archon_workflow::WorkflowResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| WorkflowError::Io {
            path: parent.to_path_buf(),
            source: err,
        })?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(value)?).map_err(|err| WorkflowError::Io {
        path: tmp.clone(),
        source: err,
    })?;
    fs::rename(&tmp, path).map_err(|err| WorkflowError::Io {
        path: path.to_path_buf(),
        source: err,
    })
}

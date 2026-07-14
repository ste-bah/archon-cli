fn quality_gate_result(
    execution: &WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
    task_universe: Option<&WorkflowV2TaskUniverse>,
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
    if execution.call.id == "final-acceptance-gate" {
        return final_acceptance_gate_result(
            execution,
            v2_store,
            task_universe,
            failed,
            source_results.len(),
        );
    }
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
            if let Some(result) = object.get("result").filter(|value| is_workflow_result_value(value)) {
                push_source_result(result, results)?;
            } else if is_workflow_result_value(value) {
                push_source_result(value, results)?;
            } else {
                for child in object.values() {
                    collect_source_results(child, results)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn push_source_result(
    value: &serde_json::Value,
    results: &mut Vec<WorkflowV2Result>,
) -> archon_workflow::WorkflowResult<()> {
    let result: WorkflowV2Result = serde_json::from_value(value.clone())?;
    results.push(result);
    Ok(())
}

fn source_result_has_concrete_evidence(result: &WorkflowV2Result) -> bool {
    result
        .evidence
        .iter()
        .any(|evidence| !evidence.summary.trim().is_empty())
        || result.commands_run.iter().any(|command| !command.command.trim().is_empty())
        || result.task_coverage.iter().any(|coverage| {
            coverage
                .evidence
                .iter()
                .any(|evidence| !evidence.summary.trim().is_empty())
        })
}

fn is_workflow_result_value(value: &serde_json::Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.contains_key("status")
        && (object.contains_key("summary")
            || object.contains_key("residual_gaps")
            || object.contains_key("task_coverage")
            || object.contains_key("commands_run")
            || object.contains_key("evidence"))
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

fn authoritative_task_ids(task_universe: Option<&WorkflowV2TaskUniverse>) -> Option<Vec<String>> {
    let mut ids = task_universe?
        .tasks
        .iter()
        .map(|task| task.canonical_task_id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    (!ids.is_empty()).then_some(ids)
}

fn completion_ledger_state(
    v2_store: &WorkflowV2ResultStore,
    required_task_ids: BTreeSet<String>,
) -> archon_workflow::WorkflowResult<(BTreeSet<String>, BTreeSet<String>, Vec<String>)> {
    let (credit, mut artifact_gaps) = validated_completion_credit(v2_store)?;
    let completed = credit
        .completed_ids()
        .intersection(&required_task_ids)
        .cloned()
        .collect::<BTreeSet<_>>();
    let missing = required_task_ids
        .difference(&completed)
        .cloned()
        .collect::<BTreeSet<_>>();
    artifact_gaps.sort();
    artifact_gaps.dedup();
    Ok((completed, missing, artifact_gaps))
}

fn validated_completion_credit(
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<(CompletionCredit, Vec<String>)> {
    let mut credit = CompletionCredit::default();
    let mut gaps = Vec::new();
    for record in v2_store.load_call_records()? {
        collect_valid_credit(v2_store, &record.completion_evidence, &mut credit, &mut gaps);
    }
    for outcome in v2_store.load_branch_outcomes()? {
        collect_valid_credit(v2_store, &outcome.completion_evidence, &mut credit, &mut gaps);
    }
    Ok((credit, gaps))
}

fn collect_valid_credit(
    store: &WorkflowV2ResultStore,
    evidence: &[archon_workflow::WorkflowV2TaskCompletionEvidence],
    credit: &mut CompletionCredit,
    gaps: &mut Vec<String>,
) {
    for item in evidence {
        if artifact_paths_exist(store.root(), &item.artifact_paths) {
            credit.record(item);
        } else {
            gaps.push(format!("{}:missing artifact evidence", item.task_id));
        }
    }
}

fn artifact_paths_exist(v2_root: &Path, paths: &[String]) -> bool {
    let concrete_paths = paths
        .iter()
        .map(|path| path.trim())
        .filter(|path| !path.is_empty())
        .filter(|path| !artifact_path_is_placeholder(path))
        .collect::<Vec<_>>();
    if concrete_paths.is_empty() {
        return paths.is_empty();
    }
    concrete_paths
        .iter()
        .all(|path| artifact_path_exists(v2_root, path))
}

pub(super) fn artifact_path_exists(v2_root: &Path, path: &str) -> bool {
    if super::workflow_live_artifact_refs::is_nonfilesystem_artifact_ref(path) {
        return true;
    }
    let path = Path::new(path);
    if path.exists() {
        return true;
    }
    if path.is_absolute() {
        return false;
    }
    if v2_root
        .parent()
        .map(|run_root| run_root.join(path).exists())
        .unwrap_or(false)
    {
        return true;
    }
    project_root_for_v2(v2_root)
        .map(|project_root| project_root.join(path).exists())
        .unwrap_or(false)
        || repository_root_for_v2(v2_root)
            .map(|repo_root| repo_root.join(path).exists())
            .unwrap_or(false)
}

fn project_root_for_v2(v2_root: &Path) -> Option<&Path> {
    let mut current = Some(v2_root);
    while let Some(path) = current {
        if path.file_name().and_then(|name| name.to_str()) == Some(".archon") {
            return path.parent();
        }
        current = path.parent();
    }
    None
}

fn repository_root_for_v2(v2_root: &Path) -> Option<PathBuf> {
    let run_root = v2_root.parent()?;
    let state_path = run_root.join("state.json");
    let raw = fs::read_to_string(state_path).ok()?;
    let state: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let root = state
        .get("spec")
        .and_then(|spec| spec.get("target_repository_root"))
        .or_else(|| state.get("target_repository_root"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(PathBuf::from(root))
}

fn artifact_path_is_placeholder(path: &str) -> bool {
    path.contains('<') || path.contains('>') || path.contains('*')
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
            for key in ["path", "artifact_path", "artifactPath"] {
                if let Some(path) = object.get(key).and_then(serde_json::Value::as_str) {
                    paths.push(PathBuf::from(path));
                }
            }
            for key in [
                "artifacts",
                "artifact_paths",
                "artifactPaths",
                "required_artifacts",
                "requiredArtifacts",
                "source_data",
            ] {
                if let Some(items) = object.get(key) {
                    collect_artifact_paths(items, paths);
                }
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

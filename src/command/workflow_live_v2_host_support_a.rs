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
            if let Some(result) = object
                .get("result")
                .filter(|value| is_workflow_result_value(value))
            {
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
        || result
            .commands_run
            .iter()
            .any(|command| !command.command.trim().is_empty())
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
    task_universe: Option<&WorkflowV2TaskUniverse>,
) -> archon_workflow::WorkflowResult<(BTreeSet<String>, BTreeSet<String>, Vec<String>)> {
    let (credit, mut artifact_gaps) = validated_completion_credit(v2_store, task_universe)?;
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

pub(super) fn validated_completion_credit(
    v2_store: &WorkflowV2ResultStore,
    task_universe: Option<&WorkflowV2TaskUniverse>,
) -> archon_workflow::WorkflowResult<(CompletionCredit, Vec<String>)> {
    let mut credit = CompletionCredit::default();
    let mut gaps = Vec::new();
    for record in v2_store.load_call_records()? {
        collect_valid_credit(
            v2_store,
            &record.completion_evidence,
            Some(&record.result),
            task_universe,
            &mut credit,
            &mut gaps,
        );
    }
    for outcome in v2_store.load_branch_outcomes()? {
        collect_valid_credit(
            v2_store,
            &outcome.completion_evidence,
            outcome.result.as_ref(),
            task_universe,
            &mut credit,
            &mut gaps,
        );
    }
    Ok((credit, gaps))
}

fn collect_valid_credit(
    store: &WorkflowV2ResultStore,
    evidence: &[archon_workflow::WorkflowV2TaskCompletionEvidence],
    result: Option<&WorkflowV2Result>,
    task_universe: Option<&WorkflowV2TaskUniverse>,
    credit: &mut CompletionCredit,
    gaps: &mut Vec<String>,
) {
    for item in evidence {
        let is_noop_credit = matches!(
            item.evidence_kind,
            archon_workflow::WorkflowV2TaskCompletionEvidenceKind::VerifiedNoop
        ) || (item.evidence_kind
            == archon_workflow::WorkflowV2TaskCompletionEvidenceKind::ImplementationCandidate
            && item.status == WorkflowV2Status::Noop);
        let noop_criteria_valid = !is_noop_credit
            || noop_acceptance_criteria_satisfied(&item.task_id, result, task_universe);
        let contradicted_claims = contradicted_artifact_existence_claims(
            store.root(),
            &item.artifact_paths,
        );
        if artifact_paths_exist(store.root(), &item.artifact_paths)
            && noop_criteria_valid
            && contradicted_claims.is_empty()
        {
            credit.record(item);
        } else {
            if !contradicted_claims.is_empty() {
                for contradiction in contradicted_claims {
                    gaps.push(format!("{}:{}", item.task_id, contradiction));
                }
            } else {
                gaps.push(format!(
                    "{}:{}",
                    item.task_id,
                    if noop_criteria_valid {
                        "missing artifact evidence"
                    } else {
                        "noop acceptance criteria were not explicitly satisfied"
                    }
                ));
            }
        }
    }
}

fn contradicted_artifact_existence_claims(v2_root: &Path, paths: &[String]) -> Vec<String> {
    let mut contradictions = Vec::new();
    for path in paths {
        let Some(resolved) = resolve_artifact_path(v2_root, path) else {
            continue;
        };
        if !matches!(
            resolved.extension().and_then(|ext| ext.to_str()),
            Some("json" | "jsonl" | "md" | "txt")
        ) {
            continue;
        }
        let Ok(metadata) = fs::metadata(&resolved) else {
            continue;
        };
        if metadata.len() > 1_048_576 {
            continue;
        }
        let Ok(text) = fs::read_to_string(&resolved) else {
            continue;
        };
        contradictions.extend(contradicted_existence_claims(v2_root, &text));
    }
    contradictions.sort();
    contradictions.dedup();
    contradictions
}

fn contradicted_existence_claims(v2_root: &Path, text: &str) -> Vec<String> {
    let mut contradictions = Vec::new();
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        let expected = if [" missing", " absent", "does not exist", "not found"]
            .iter()
            .any(|term| lower.contains(term))
        {
            Some(false)
        } else if [" exists", " present", "found at"]
            .iter()
            .any(|term| lower.contains(term))
        {
            Some(true)
        } else {
            None
        };
        let Some(expected) = expected else {
            continue;
        };
        for path in filesystem_paths_in_text(line) {
            let actual = artifact_path_exists(v2_root, &path);
            if actual != expected {
                contradictions.push(format!(
                    "artifact existence claim contradicted by disk: path={path}; claimed={}; actual={}",
                    if expected { "exists" } else { "missing" },
                    if actual { "exists" } else { "missing" },
                ));
            }
        }
    }
    contradictions
}

fn filesystem_paths_in_text(text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for raw in text.split_whitespace() {
        let mut token = raw.trim_matches(|ch: char| {
            matches!(ch, '`' | '\'' | '"' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';')
        });
        if let Some((_, value)) = token.split_once('=') {
            token = value;
        }
        let token = token.trim_end_matches(['.', ':']);
        let looks_like_path = token.starts_with('/')
            || token.starts_with(".archon/")
            || token.starts_with("./")
            || (token.contains('/')
                && [".json", ".jsonl", ".md", ".txt", ".csv", ".zip"]
                    .iter()
                    .any(|extension| token.ends_with(extension)));
        if looks_like_path && !artifact_path_is_placeholder(token) {
            paths.push(token.to_string());
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn resolve_artifact_path(v2_root: &Path, raw: &str) -> Option<PathBuf> {
    if super::workflow_live_artifact_refs::is_nonfilesystem_artifact_ref(raw) {
        return None;
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return path.exists().then(|| path.to_path_buf());
    }
    let candidates = [
        v2_root.parent().map(|root| root.join(path)),
        project_root_for_v2(v2_root).map(|root| root.join(path)),
        repository_root_for_v2(v2_root).map(|root| root.join(path)),
    ];
    candidates.into_iter().flatten().find(|path| path.exists())
}


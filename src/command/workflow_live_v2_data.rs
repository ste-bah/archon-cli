use archon_workflow::{
    WorkflowError, WorkflowSpec, WorkflowV2AgentRequest, WorkflowV2BranchOutcome,
    WorkflowV2CallExecution, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FanoutItem,
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2ResidualGap, WorkflowV2Result,
    WorkflowV2ResultStore, WorkflowV2Status,
};

use super::workflow_live_v2_aggregate::attach_branch_evidence;

pub(super) fn execution_with_resolved_source(
    execution: &WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<WorkflowV2CallExecution> {
    if execution.input.get("source_data").is_some() {
        return Ok(execution.clone());
    }
    let Some(source) = execution.call.options.source.as_deref() else {
        return Ok(execution.clone());
    };
    let source_data = resolve_source_value(source, v2_store)?;
    let mut enriched = execution.clone();
    if let Some(object) = enriched.input.as_object_mut() {
        object.insert(
            "source".to_string(),
            serde_json::Value::String(source.to_string()),
        );
        object.insert("source_data".to_string(), source_data);
    } else {
        enriched.input = serde_json::json!({
            "input": enriched.input,
            "source": source,
            "source_data": source_data,
        });
    }
    Ok(enriched)
}

pub(super) fn fanout_items_for_call(
    execution: &WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<Vec<WorkflowV2FanoutItem>> {
    let (source, values) = fanout_source_values(execution, v2_store)?;
    let role = execution
        .call
        .options
        .role
        .clone()
        .unwrap_or_else(|| role_for_v2_call(execution.call.method).to_string());
    Ok(values
        .into_iter()
        .enumerate()
        .map(|(idx, value)| {
            let item_id = fanout_item_id(&value, idx);
            let mut branch_call = execution.call.clone();
            branch_call.id = format!("{}-{item_id}", execution.call.id);
            branch_call.options.source = None;
            if branch_call.options.target_files_from_item {
                let item_targets = target_files_from_value(&value);
                if !item_targets.is_empty() {
                    branch_call.options.target_files = item_targets;
                }
            }
            branch_call.method = if branch_call.write_mode.is_some() {
                WorkflowV2HostMethod::Implementation
            } else {
                WorkflowV2HostMethod::Agent
            };
            let input = serde_json::json!({
                "fanout_call_id": execution.call.id,
                "fanout_item_id": item_id,
                "source": source,
                "item": value,
            });
            WorkflowV2FanoutItem::read_only(
                branch_call.id.clone(),
                role.clone(),
                branch_call,
                input,
            )
        })
        .collect())
}

fn fanout_source_values(
    execution: &WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<(String, Vec<serde_json::Value>)> {
    if let Some(source_data) = execution.input.get("source_data") {
        return Ok((
            execution
                .call
                .options
                .source
                .clone()
                .unwrap_or_else(|| "workflow.js source argument".to_string()),
            array_from_source_data(source_data)?,
        ));
    }
    let source = execution.call.options.source.as_deref().ok_or_else(|| {
        WorkflowError::SpecInvalid(format!(
            "w.{}('{}') requires a typed source expression or runtime source argument",
            execution.call.method.as_str(),
            execution.call.id
        ))
    })?;
    Ok((source.to_string(), resolve_fanout_source(source, v2_store)?))
}

fn array_from_source_data(
    source_data: &serde_json::Value,
) -> archon_workflow::WorkflowResult<Vec<serde_json::Value>> {
    if let Some(values) = source_data.as_array() {
        return Ok(values.clone());
    }
    if let Some(values) = source_data
        .get("items")
        .and_then(serde_json::Value::as_array)
    {
        return Ok(values.clone());
    }
    Err(WorkflowError::SpecInvalid(
        "fanout runtime source argument resolved to non-array typed data".to_string(),
    ))
}

fn resolve_fanout_source(
    source: &str,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<Vec<serde_json::Value>> {
    let cursor = resolve_source_value(source, v2_store)?;
    cursor.as_array().cloned().ok_or_else(|| {
        WorkflowError::SpecInvalid(format!(
            "fanout source '{source}' resolved to non-array typed data"
        ))
    })
}

pub(super) fn resolve_source_value(
    source: &str,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<serde_json::Value> {
    let trimmed = source.trim();
    if let Some(list) = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
    {
        let mut values = Vec::new();
        for part in list.split(',') {
            let source = part.trim();
            if !source.is_empty() {
                values.push(resolve_single_source_value(source, v2_store)?);
            }
        }
        return Ok(serde_json::Value::Array(values));
    }
    resolve_single_source_value(trimmed, v2_store)
}

fn resolve_single_source_value(
    source: &str,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<serde_json::Value> {
    if let Some((call_id, path)) = source.split_once('.') {
        return source_value_from_call_path(call_id, Some(path), source, v2_store);
    }
    if source
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return source_value_from_call_path(source, None, source, v2_store);
    }
    Err(WorkflowError::SpecInvalid(format!(
        "source '{source}' must reference a prior call or field, for example inventory.items"
    )))
}

fn source_value_from_call_path(
    call_id: &str,
    path: Option<&str>,
    source: &str,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<serde_json::Value> {
    let record = v2_store.load_call_record(call_id)?.ok_or_else(|| {
        WorkflowError::SpecInvalid(format!(
            "source '{source}' references missing prior call '{call_id}'"
        ))
    })?;
    let mut cursor = if record.result.data.is_null() {
        serde_json::to_value(&record.result)?
    } else {
        record.result.data.clone()
    };
    if let Some(path) = path {
        for segment in path.split('.') {
            cursor = cursor.get(segment).cloned().ok_or_else(|| {
                WorkflowError::SpecInvalid(format!(
                    "source '{source}' field '{segment}' is absent from prior result data"
                ))
            })?;
        }
    }
    Ok(cursor)
}

fn fanout_item_id(value: &serde_json::Value, idx: usize) -> String {
    value
        .get("id")
        .or_else(|| value.get("task_id"))
        .or_else(|| value.get("work_unit_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(sanitize_v2_id)
        .unwrap_or_else(|| idx.to_string())
}

fn target_files_from_value(value: &serde_json::Value) -> Vec<String> {
    value
        .get("target_files")
        .or_else(|| value.get("expected_target_files"))
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn sanitize_v2_id(raw: &str) -> String {
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

pub(super) fn result_from_fanout_report(
    call: &WorkflowV2HostCall,
    report: archon_workflow::WorkflowV2FanoutReport,
) -> WorkflowV2Result {
    let peak_parallelism = report.peak_parallelism;
    let max_parallelism = report.max_parallelism;
    let outcomes = normalize_fanout_outcomes(report.outcomes);
    let typed_results = typed_results_from_outcomes(&outcomes);
    if outcomes.is_empty() {
        let mut result = WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: format!(
                "fanout '{}' resolved zero items without typed no-op proof",
                call.id
            ),
            ..WorkflowV2Result::default()
        };
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Blocker,
            "fanout source resolved to zero items; upstream output must provide typed no-op proof or remediation inventory before work can be skipped",
        ));
        result.residual_gaps.push(WorkflowV2ResidualGap {
            id: format!("empty_fanout_{}", sanitize_v2_id(&call.id)),
            description: "fanout resolved zero items without typed no-op proof".to_string(),
            severity: Some("review".to_string()),
        });
        result.data = serde_json::json!({
            "items": typed_results,
            "outcomes": outcomes,
            "peak_parallelism": peak_parallelism,
            "max_parallelism": max_parallelism,
        });
        return result;
    }
    let cancelled_count = count_outcomes_with_status(&outcomes, WorkflowV2Status::Cancelled);
    let blocked_count = count_outcomes_with_status(&outcomes, WorkflowV2Status::Blocked);
    let failed_count = count_outcomes_with_status(&outcomes, WorkflowV2Status::Failed);
    let review_count = count_outcomes_with_status(&outcomes, WorkflowV2Status::NeedsReview);
    let mut result = if cancelled_count > 0 {
        WorkflowV2Result {
            status: WorkflowV2Status::Cancelled,
            summary: format!(
                "fanout '{}' cancelled with {} cancelled branch(es)",
                call.id, cancelled_count
            ),
            ..WorkflowV2Result::default()
        }
    } else if failed_count > 0 || blocked_count > 0 || review_count > 0 {
        WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: format!(
                "fanout '{}' completed with {} branch finding(s) for workflow.js to reduce or remediate",
                call.id,
                failed_count + blocked_count + review_count
            ),
            ..WorkflowV2Result::default()
        }
    } else {
        WorkflowV2Result::accepted(format!(
            "fanout '{}' completed {} branch(es)",
            call.id,
            outcomes.len()
        ))
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Inspection,
        format!(
            "fanout scheduler ran with peak parallelism {} and max parallelism {}",
            peak_parallelism, max_parallelism
        ),
    ));
    if matches!(result.status, WorkflowV2Status::NeedsReview) {
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Review,
            "fanout returned all branch outcomes as typed data; workflow.js must decide whether to reduce, remediate, retry, ask the user, or accept residual gaps",
        ));
    }
    attach_branch_evidence(&mut result, &typed_results);
    result.data = serde_json::json!({
        "items": typed_results,
        "outcomes": outcomes,
        "peak_parallelism": peak_parallelism,
        "max_parallelism": max_parallelism,
    });
    result
}

fn normalize_fanout_outcomes(
    outcomes: Vec<WorkflowV2BranchOutcome>,
) -> Vec<WorkflowV2BranchOutcome> {
    outcomes
        .into_iter()
        .map(|mut outcome| {
            match outcome.status {
                WorkflowV2Status::Blocked if outcome.result.is_none() => {
                    outcome.result = Some(blocked_branch_result(&outcome));
                }
                WorkflowV2Status::Failed if outcome.result.is_none() => {
                    let error = outcome.error.clone().unwrap_or_default();
                    outcome.result = Some(failed_branch_error_result(&outcome, &error));
                }
                _ => {}
            }
            outcome
        })
        .collect()
}

fn blocked_branch_result(outcome: &WorkflowV2BranchOutcome) -> WorkflowV2Result {
    let mut result = outcome.result.clone().unwrap_or_else(|| WorkflowV2Result {
        status: WorkflowV2Status::Blocked,
        summary: format!(
            "fanout branch '{}' reported a blocker for workflow.js to handle",
            outcome.item_id
        ),
        ..WorkflowV2Result::default()
    });
    result.status = WorkflowV2Status::Blocked;
    if !result
        .evidence
        .iter()
        .any(|evidence| evidence.kind == WorkflowV2EvidenceKind::Blocker)
    {
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Blocker,
            "blocked fanout branch was retained as typed remediation or user-choice input",
        ));
    }
    for gap in &mut result.residual_gaps {
        gap.severity = Some("blocking".to_string());
    }
    result
}

fn failed_branch_error_result(outcome: &WorkflowV2BranchOutcome, error: &str) -> WorkflowV2Result {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::Failed,
        summary: format!(
            "fanout branch '{}' produced invalid structured output after repair",
            outcome.item_id
        ),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Blocker,
        "branch output was invalid or asked for confirmation; the branch outcome was retained as typed data for workflow.js",
    ));
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!("invalid_branch_output_{}", sanitize_v2_id(&outcome.item_id)),
        description: truncate_for_result(error, 500),
        severity: Some("blocking".to_string()),
    });
    result.data = serde_json::json!({
        "branch_id": outcome.item_id,
        "role": outcome.role,
        "branch_error_from_runtime": true,
        "error": truncate_for_result(error, 2_000),
    });
    result
}

fn typed_results_from_outcomes(outcomes: &[WorkflowV2BranchOutcome]) -> Vec<WorkflowV2Result> {
    outcomes
        .iter()
        .filter_map(|outcome| outcome.result.clone())
        .collect()
}

fn count_outcomes_with_status(
    outcomes: &[WorkflowV2BranchOutcome],
    status: WorkflowV2Status,
) -> usize {
    outcomes
        .iter()
        .filter(|outcome| outcome.status == status)
        .count()
}

fn truncate_for_result(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for ch in value.chars().take(max_chars) {
        output.push(ch);
    }
    if value.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

pub(super) fn v2_agent_request(
    task: &str,
    spec: &WorkflowSpec,
    execution: &WorkflowV2CallExecution,
) -> WorkflowV2AgentRequest {
    let stage = spec
        .stages
        .iter()
        .find(|stage| stage.id == execution.call.id);
    let mut constraints = vec![
        "Return exactly one typed WorkflowV2Result JSON object.".to_string(),
        "Do not return markdown, prose-only summaries, or plan-only implementation text."
            .to_string(),
    ];
    if stage_declares_items_output(stage) {
        constraints.push(
            "This call feeds downstream fanout: put work items in data.items as a flat JSON array of item objects. Do not nest items under dependency_phases, groups, phases, or any other wrapper.".to_string(),
        );
    }
    WorkflowV2AgentRequest {
        call: execution.call.clone(),
        role: execution
            .call
            .options
            .role
            .clone()
            .unwrap_or_else(|| role_for_v2_call(execution.call.method).to_string()),
        task: execution
            .call
            .options
            .task
            .clone()
            .or_else(|| stage.and_then(|stage| stage.task.clone()))
            .unwrap_or_else(|| {
                format!(
                    "Execute workflow V2 host call '{}' for objective: {}",
                    execution.call.id, task
                )
            }),
        constraints,
        input: execution.input.clone(),
        repository_root: spec.target_repository_root.clone(),
        target_files: if execution.call.options.target_files.is_empty() {
            stage
                .map(|stage| stage.expected_target_files.clone())
                .unwrap_or_default()
        } else {
            execution.call.options.target_files.clone()
        },
    }
}

fn stage_declares_items_output(stage: Option<&archon_workflow::StageSpec>) -> bool {
    stage
        .and_then(|stage| stage.extra.get("outputs"))
        .is_some_and(|outputs| match outputs {
            serde_json::Value::Array(values) => values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(|value| value.eq_ignore_ascii_case("items")),
            serde_json::Value::String(value) => value.eq_ignore_ascii_case("items"),
            _ => false,
        })
}

pub(super) fn role_for_v2_call(method: WorkflowV2HostMethod) -> &'static str {
    match method {
        WorkflowV2HostMethod::Implementation => "coder",
        WorkflowV2HostMethod::Fanout | WorkflowV2HostMethod::Parallel => "coder",
        WorkflowV2HostMethod::Reduce | WorkflowV2HostMethod::FinalReport => "reducer",
        WorkflowV2HostMethod::QualityGate | WorkflowV2HostMethod::HumanGate => "critic",
        WorkflowV2HostMethod::Tool
        | WorkflowV2HostMethod::SaveArtifact
        | WorkflowV2HostMethod::RequireArtifact
        | WorkflowV2HostMethod::Checkpoint => "tool",
        WorkflowV2HostMethod::Agent => "researcher",
    }
}

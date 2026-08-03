use super::*;

pub(super) fn canonical_task_ids_from_result(result: &WorkflowV2Result) -> Vec<String> {
    let mut ids = canonical_task_ids_from_generated_value(&result.data, None);
    ids.extend(
        string_array(result.data.get("canonical_task_ids"))
            .into_iter()
            .chain(string_array(result.data.get("canonicalTaskIds")))
            .chain(string_array(result.data.get("canonical_task_id")))
            .chain(string_array(result.data.get("canonicalTaskId")))
            .chain(string_array(result.data.get("task_ids")))
            .chain(string_array(result.data.get("taskIds")))
            .chain(string_array(result.data.get("task_id")))
            .collect::<Vec<_>>(),
    );
    ids.extend(result.task_coverage.iter().filter_map(|coverage| {
        matches!(
            coverage.status,
            WorkflowV2TaskCoverageStatus::Accepted | WorkflowV2TaskCoverageStatus::Noop
        )
        .then(|| coverage.task_id.trim().to_string())
        .filter(|task_id| !task_id.is_empty())
    }));
    sorted_unique(ids)
}

pub(super) fn evidence_summaries_from_result(result: &WorkflowV2Result) -> Vec<String> {
    let accepted_or_noop = matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    );
    let mut evidence = result
        .evidence
        .iter()
        .filter(|item| {
            !accepted_or_noop
                || matches!(
                    item.kind,
                    WorkflowV2EvidenceKind::Implementation | WorkflowV2EvidenceKind::Test
                )
        })
        .map(|item| item.summary.trim().to_string())
        .filter(|summary| !summary.is_empty())
        .collect::<Vec<_>>();
    for coverage in &result.task_coverage {
        if accepted_or_noop
            && !matches!(
                coverage.status,
                WorkflowV2TaskCoverageStatus::Accepted | WorkflowV2TaskCoverageStatus::Noop
            )
        {
            continue;
        }
        evidence.extend(
            coverage
                .evidence
                .iter()
                .map(|item| item.summary.trim().to_string())
                .filter(|summary| !summary.is_empty()),
        );
    }
    evidence.extend(
        result
            .commands_run
            .iter()
            .filter(|command| {
                !accepted_or_noop || command.status == WorkflowV2CommandStatus::Succeeded
            })
            .map(|command| command.command.trim().to_string())
            .filter(|command| !command.is_empty()),
    );
    evidence.extend(
        result
            .files_changed
            .iter()
            .map(|file| file.path.trim().to_string())
            .filter(|path| !path.is_empty()),
    );
    evidence.extend(
        result
            .artifacts
            .iter()
            .map(|artifact| artifact.path.trim().to_string())
            .filter(|path| !path.is_empty()),
    );
    evidence.extend(evidence_refs_from_generated_value(&result.data));
    if !accepted_or_noop {
        evidence.extend(
            result
                .residual_gaps
                .iter()
                .map(|gap| gap.description.trim().to_string())
                .filter(|description| !description.is_empty()),
        );
        evidence.extend(string_array(result.data.get("evidence")));
    }
    sorted_unique(evidence)
}

pub(super) fn string_array(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        Some(serde_json::Value::String(value)) => value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

pub(super) fn string_value(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn sorted_unique(values: Vec<String>) -> Vec<String> {
    use std::collections::BTreeSet;

    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn blocked_branch_result(outcome: &WorkflowV2BranchOutcome) -> WorkflowV2Result {
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

pub(super) fn failed_branch_error_result(
    outcome: &WorkflowV2BranchOutcome,
    error: &str,
) -> WorkflowV2Result {
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

pub(super) fn typed_results_from_outcomes(
    outcomes: &[WorkflowV2BranchOutcome],
) -> Vec<WorkflowV2Result> {
    outcomes
        .iter()
        .filter_map(|outcome| outcome.result.clone())
        .collect()
}

pub(super) fn count_outcomes_with_status(
    outcomes: &[WorkflowV2BranchOutcome],
    status: WorkflowV2Status,
) -> usize {
    outcomes
        .iter()
        .filter(|outcome| outcome.status == status)
        .count()
}

pub(super) fn count_outcomes_with_failure_kind(
    outcomes: &[WorkflowV2BranchOutcome],
    kinds: &[BranchFailureKind],
) -> usize {
    outcomes
        .iter()
        .filter(|outcome| {
            outcome
                .failure_kind
                .as_ref()
                .is_some_and(|kind| kinds.iter().any(|candidate| candidate == kind))
        })
        .count()
}

pub(super) fn truncate_for_result(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for ch in value.chars().take(max_chars) {
        output.push(ch);
    }
    if value.chars().count() > max_chars {
        output.push_str("...");
    }
    output
}

use super::*;

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

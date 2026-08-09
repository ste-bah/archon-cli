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

/// Whether a branch error is about the shape of the agent's output.
///
/// Deliberately narrow: anything not recognisably a schema complaint is
/// described by its own text rather than mislabelled. Over-matching here
/// reinstates the bug — a transport error dressed up as a schema failure.
fn error_looks_like_schema_failure(error: &str) -> bool {
    let lowered = error.to_ascii_lowercase();
    [
        "schema repair",
        "invalid structured output",
        "unknown variant",
        "missing field",
        "expected one of",
        "must be one json",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
}

pub(super) fn failed_branch_error_result(
    outcome: &WorkflowV2BranchOutcome,
    error: &str,
) -> WorkflowV2Result {
    // The summary used to assert "produced invalid structured output after
    // repair" for EVERY failure reaching this function, including transport and
    // notification-delivery errors that say nothing about output shape. A live
    // branch killed by a dropped activity channel was labelled a schema failure,
    // and the label was believed over the error sitting beside it. Name the
    // schema case only when the error actually looks like one.
    let schema_failure = error_looks_like_schema_failure(error);
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::Failed,
        summary: if schema_failure {
            format!(
                "fanout branch '{}' produced invalid structured output after repair",
                outcome.item_id
            )
        } else {
            format!(
                "fanout branch '{}' failed before a usable result was produced: {}",
                outcome.item_id,
                truncate_for_result(error, 200)
            )
        },
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Blocker,
        if schema_failure {
            "branch output was invalid or asked for confirmation; the branch outcome was retained as typed data for workflow.js"
        } else {
            "branch failed before producing output; the runtime error is recorded verbatim in the residual gap"
        },
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

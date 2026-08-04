use super::*;

#[derive(Debug)]
pub struct WorkflowV2NormalizedFanout {
    pub result: WorkflowV2Result,
    pub outcomes: Vec<WorkflowV2BranchOutcome>,
}

impl std::ops::Deref for WorkflowV2NormalizedFanout {
    type Target = WorkflowV2Result;

    fn deref(&self) -> &Self::Target {
        &self.result
    }
}

pub fn result_from_fanout_report(
    call: &WorkflowV2HostCall,
    report: crate::WorkflowV2FanoutReport,
) -> WorkflowV2NormalizedFanout {
    let peak_parallelism = report.peak_parallelism;
    let max_parallelism = report.max_parallelism;
    let implementation_write_fanout = implementation_write_fanout(call);
    let outcomes = normalize_fanout_outcomes(call, report.outcomes);
    let typed_results = typed_results_from_outcomes(&outcomes);
    let outcome_views = fanout_outcome_views(&outcomes);
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
            "outcomes": outcome_views,
            "peak_parallelism": peak_parallelism,
            "max_parallelism": max_parallelism,
        });
        return WorkflowV2NormalizedFanout { result, outcomes };
    }
    let cancelled_count = count_outcomes_with_status(&outcomes, WorkflowV2Status::Cancelled);
    let blocked_count = count_outcomes_with_status(&outcomes, WorkflowV2Status::Blocked);
    let review_count = count_outcomes_with_status(&outcomes, WorkflowV2Status::NeedsReview);
    let terminal_failure_count = count_outcomes_with_failure_kind(
        &outcomes,
        &[BranchFailureKind::Safety, BranchFailureKind::Execution],
    );
    let semantic_or_contract_count = count_outcomes_with_failure_kind(
        &outcomes,
        &[BranchFailureKind::Semantic, BranchFailureKind::Contract],
    );
    let usable_structured_count = outcomes
        .iter()
        .filter(|outcome| {
            outcome
                .result
                .as_ref()
                .is_some_and(|result| result.validate().is_ok())
        })
        .count();
    let mut result = if cancelled_count > 0 {
        WorkflowV2Result {
            status: WorkflowV2Status::Cancelled,
            summary: format!(
                "fanout '{}' cancelled with {} cancelled branch(es)",
                call.id, cancelled_count
            ),
            ..WorkflowV2Result::default()
        }
    } else if implementation_write_fanout && terminal_failure_count > 0 {
        WorkflowV2Result {
            status: WorkflowV2Status::Failed,
            summary: format!(
                "write-capable fanout '{}' failed with {} safety/execution failure branch(es)",
                call.id, terminal_failure_count
            ),
            ..WorkflowV2Result::default()
        }
    } else if !implementation_write_fanout
        && terminal_failure_count == outcomes.len()
        && usable_structured_count == 0
    {
        WorkflowV2Result {
            status: WorkflowV2Status::Failed,
            summary: format!(
                "fanout '{}' failed because every branch failed structurally or at runtime",
                call.id
            ),
            ..WorkflowV2Result::default()
        }
    } else if terminal_failure_count > 0
        || semantic_or_contract_count > 0
        || blocked_count > 0
        || review_count > 0
    {
        WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: format!(
                "fanout '{}' completed with {} branch finding(s) for workflow.js to reduce or remediate",
                call.id,
                terminal_failure_count + semantic_or_contract_count + blocked_count + review_count
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
        "outcomes": outcome_views,
        "peak_parallelism": peak_parallelism,
        "max_parallelism": max_parallelism,
    });
    WorkflowV2NormalizedFanout { result, outcomes }
}

pub(super) fn normalize_fanout_outcomes(
    call: &WorkflowV2HostCall,
    outcomes: Vec<WorkflowV2BranchOutcome>,
) -> Vec<WorkflowV2BranchOutcome> {
    let validate_implementation_contract = implementation_write_fanout(call);
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
            if outcome.failure_kind.is_none() {
                outcome.failure_kind = failure_kind_for_outcome(&outcome);
            }
            if validate_implementation_contract {
                normalize_implementation_branch_contract(&mut outcome);
            }
            normalize_focused_verification_outcome(&call.id, &mut outcome);
            attach_completion_evidence_for_call(call, &mut outcome);
            outcome
        })
        .collect()
}

pub(super) fn implementation_write_fanout(call: &WorkflowV2HostCall) -> bool {
    call.method == WorkflowV2HostMethod::Fanout
        && matches!(
            call.write_mode,
            Some(WorkflowV2WriteMode::Coordinated | WorkflowV2WriteMode::Worktree)
        )
        && call
            .options
            .item_kind
            .as_deref()
            .is_some_and(|kind| kind.eq_ignore_ascii_case("implementation"))
}

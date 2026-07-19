#[derive(Debug)]
pub(super) struct WorkflowV2NormalizedFanout {
    pub(super) result: WorkflowV2Result,
    pub(super) outcomes: Vec<WorkflowV2BranchOutcome>,
}

impl std::ops::Deref for WorkflowV2NormalizedFanout {
    type Target = WorkflowV2Result;

    fn deref(&self) -> &Self::Target {
        &self.result
    }
}

pub(super) fn result_from_fanout_report(
    call: &WorkflowV2HostCall,
    report: archon_workflow::WorkflowV2FanoutReport,
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

fn normalize_fanout_outcomes(
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

pub(super) fn attach_completion_evidence_for_call(
    call: &WorkflowV2HostCall,
    outcome: &mut WorkflowV2BranchOutcome,
) {
    let Some(kind) = task_completion_evidence_kind(&call.id) else {
        return;
    };
    if !matches!(
        outcome.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) {
        return;
    }
    let Some(result) = outcome.result.as_ref() else {
        return;
    };
    let task_ids = canonical_task_ids_from_result(result);
    let evidence_refs = evidence_summaries_from_result(result);
    if task_ids.is_empty() || evidence_refs.is_empty() {
        return;
    }
    let artifact_paths = result
        .artifacts
        .iter()
        .map(|artifact| artifact.path.trim().to_string())
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    let command_refs = result
        .commands_run
        .iter()
        .filter(|command| command.status == WorkflowV2CommandStatus::Succeeded)
        .map(|command| command.command.trim().to_string())
        .filter(|command| !command.is_empty())
        .collect::<Vec<_>>();
    let source_call_id = string_value(result.data.get("source_call_id"))
        .or_else(|| string_value(result.data.get("sourceCallId")));
    let source_item_id = string_value(result.data.get("source_item_id"))
        .or_else(|| string_value(result.data.get("sourceItemId")));
    let mut evidence = Vec::new();
    for task_id in task_ids {
        let mut item = WorkflowV2TaskCompletionEvidence::new(
            task_id,
            kind.clone(),
            call.id.clone(),
            outcome.item_id.clone(),
            outcome.status,
        );
        item.source_call_id = source_call_id.clone();
        item.source_item_id = source_item_id.clone();
        if matches!(
            kind,
            WorkflowV2TaskCompletionEvidenceKind::FocusedVerification
        ) {
            item.source_fingerprint =
                Some(FOCUSED_VERIFICATION_EVIDENCE_CONTRACT_VERSION.to_string());
        }
        item.evidence_refs = evidence_refs.clone();
        item.artifact_paths = artifact_paths.clone();
        item.command_refs = command_refs.clone();
        item.item_input_hash = outcome.item_input_hash.clone();
        evidence.push(item);
    }
    outcome.completion_evidence = evidence;
}

fn task_completion_evidence_kind(call_id: &str) -> Option<WorkflowV2TaskCompletionEvidenceKind> {
    if call_id.starts_with("noop-proof-verification-")
        || call_id.starts_with("noop-proof-reverification-")
    {
        return Some(WorkflowV2TaskCompletionEvidenceKind::VerifiedNoop);
    }
    if call_id.starts_with("verification-wave-") || call_id.starts_with("review-verification-wave-")
    {
        return Some(WorkflowV2TaskCompletionEvidenceKind::FocusedVerification);
    }
    if call_id.starts_with("implementation-wave-")
        || call_id.starts_with("remediation-wave-")
        || call_id.starts_with("review-remediation-wave-")
    {
        return Some(WorkflowV2TaskCompletionEvidenceKind::ImplementationCandidate);
    }
    None
}

fn implementation_write_fanout(call: &WorkflowV2HostCall) -> bool {
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

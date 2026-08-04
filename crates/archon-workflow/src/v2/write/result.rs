use super::*;

pub(super) fn result_from_write_fanout(
    call: &WorkflowV2HostCall,
    mut branch_results: Vec<WorkflowV2Result>,
    plan: &WorkflowV2WritePlan,
    peak_parallelism: usize,
    fallback_reason: Option<String>,
) -> WorkflowV2Result {
    annotate_write_ownership_expansions(&mut branch_results, plan);
    let outcome_views = sanitized_write_fanout_outcomes(call, &branch_results);
    let invalid_outcome_count = outcome_views
        .iter()
        .filter(|outcome| {
            outcome
                .get("contract_valid")
                .and_then(serde_json::Value::as_bool)
                == Some(false)
        })
        .count();
    let cancelled = count_results_with_status(&branch_results, WorkflowV2Status::Cancelled);
    let blocked = count_results_with_status(&branch_results, WorkflowV2Status::Blocked);
    let needs_review = count_results_with_status(&branch_results, WorkflowV2Status::NeedsReview);
    let terminal_failure = branch_results
        .iter()
        .filter(|result| {
            matches!(
                failure_kind_from_write_result(result),
                Some(BranchFailureKind::Safety | BranchFailureKind::Execution)
            )
        })
        .count();
    let semantic_or_contract_findings = branch_results
        .iter()
        .filter(|result| {
            matches!(
                failure_kind_from_write_result(result),
                Some(BranchFailureKind::Semantic | BranchFailureKind::Contract)
            )
        })
        .count();
    let mut result = if cancelled > 0 {
        WorkflowV2Result {
            status: WorkflowV2Status::Cancelled,
            summary: format!(
                "write-capable fanout '{}' cancelled with {} cancelled branch(es)",
                call.id, cancelled
            ),
            ..WorkflowV2Result::default()
        }
    } else if terminal_failure > 0 {
        WorkflowV2Result {
            status: WorkflowV2Status::Failed,
            summary: format!(
                "write-capable fanout '{}' failed with {} safety/execution failure branch(es)",
                call.id, terminal_failure
            ),
            residual_gaps: vec![WorkflowV2ResidualGap {
                id: format!("write_fanout_failed_{}", sanitize_v2_path_segment(&call.id)),
                description: format!(
                    "write-capable fanout '{}' hit terminal branch safety/execution failure; restart or fix the failed branch before downstream acceptance",
                    call.id
                ),
                severity: Some("blocking".to_string()),
            }],
            ..WorkflowV2Result::default()
        }
    } else if blocked > 0
        || needs_review > 0
        || invalid_outcome_count > 0
        || semantic_or_contract_findings > 0
    {
        WorkflowV2Result {
            status: WorkflowV2Status::NeedsReview,
            summary: format!(
                "write-capable fanout '{}' completed with branch findings for workflow.js remediation: blocked {}, review {}, malformed {}, semantic/contract {}",
                call.id,
                blocked,
                needs_review,
                invalid_outcome_count,
                semantic_or_contract_findings
            ),
            residual_gaps: vec![WorkflowV2ResidualGap {
                id: format!("write_fanout_review_{}", sanitize_v2_path_segment(&call.id)),
                description: format!(
                    "write-capable fanout '{}' returned branch findings; inspect data.items and run remediation or review before final acceptance",
                    call.id
                ),
                severity: Some("review".to_string()),
            }],
            ..WorkflowV2Result::default()
        }
    } else {
        WorkflowV2Result::accepted(format!(
            "write-capable fanout '{}' completed {} branch(es)",
            call.id,
            branch_results.len()
        ))
    };
    add_write_fanout_evidence(&mut result, plan, fallback_reason.clone());
    attach_branch_evidence(&mut result, &branch_results);
    result.data = serde_json::json!({
        "items": branch_results,
        "outcomes": outcome_views,
        "write_mode": plan.mode,
        "waves": plan.waves.iter().map(|wave| {
            serde_json::json!({
                "assignments": wave.assignments.iter().map(|assignment| {
                    serde_json::json!({
                        "item_id": assignment.item_id,
                        "owned_targets": assignment.owned_targets,
                        "worktree_path": assignment.worktree_path,
                    })
                }).collect::<Vec<_>>()
            })
        }).collect::<Vec<_>>(),
        "conflicts": plan.conflicts.iter().map(|conflict| {
            serde_json::json!({
                "left_item": conflict.left_item,
                "right_item": conflict.right_item,
                "target": conflict.target,
                "isolated_by_worktree": conflict.isolated_by_worktree,
            })
        }).collect::<Vec<_>>(),
        "peak_parallelism": peak_parallelism,
        "serial_fallback_reason": fallback_reason,
    });
    result
}

pub(super) fn sanitized_write_fanout_outcomes(
    call: &WorkflowV2HostCall,
    branch_results: &[WorkflowV2Result],
) -> Vec<serde_json::Value> {
    branch_results
        .iter()
        .map(|result| sanitized_write_fanout_outcome(call, result))
        .collect()
}

pub(super) fn sanitized_write_fanout_outcome(
    call: &WorkflowV2HostCall,
    result: &WorkflowV2Result,
) -> serde_json::Value {
    let item_id = item_id_from_branch_result(result);
    let canonical_task_ids = canonical_task_ids_from_branch_result(result);
    let evidence = concrete_evidence_from_branch_result(result);
    let mut failure_kind = failure_kind_from_write_result(result);
    let mut status = result.status;
    let mut contract_errors = Vec::new();
    if item_id.is_none() {
        contract_errors.push("missing item_id/id".to_string());
    }
    if canonical_task_ids.is_empty() {
        contract_errors.push("missing canonical_task_ids".to_string());
    }
    if evidence.is_empty() {
        contract_errors.push("missing concrete evidence".to_string());
    }
    if !contract_errors.is_empty()
        && !matches!(
            result.status,
            WorkflowV2Status::Failed | WorkflowV2Status::Cancelled
        )
    {
        status = WorkflowV2Status::NeedsReview;
        failure_kind = Some(BranchFailureKind::Contract);
    }
    // Durable completion evidence must survive into the persisted outcome
    // view; the completion ledger and result reuse read it from here.
    let mut ledger_outcome = WorkflowV2BranchOutcome {
        item_id: item_id.clone().unwrap_or_default(),
        role: String::new(),
        status,
        result: Some(result.clone()),
        error: None,
        failure_kind: failure_kind.clone(),
        item_input_hash: None,
        completion_evidence: Vec::new(),
    };
    attach_completion_evidence_for_call(call, &mut ledger_outcome);
    serde_json::json!({
        "item_id": item_id.clone(),
        "id": item_id,
        "canonical_task_ids": canonical_task_ids,
        "status": status,
        "failure_kind": failure_kind,
        "evidence": evidence,
        "contract_valid": contract_errors.is_empty(),
        "contract_errors": contract_errors,
        "summary": result.summary,
        "completion_evidence": ledger_outcome.completion_evidence,
    })
}

pub(super) fn item_id_from_branch_result(result: &WorkflowV2Result) -> Option<String> {
    result
        .data
        .get("item_id")
        .or_else(|| result.data.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

pub(super) fn canonical_task_ids_from_branch_result(result: &WorkflowV2Result) -> Vec<String> {
    let mut ids = BTreeSet::new();
    for id in canonical_task_ids_from_generated_value(&result.data, None)
        .into_iter()
        .chain(string_array_from_data(
            result
                .data
                .get("canonical_task_ids")
                .or_else(|| result.data.get("canonicalTaskIds"))
                .or_else(|| result.data.get("canonical_task_id"))
                .or_else(|| result.data.get("canonicalTaskId"))
                .or_else(|| result.data.get("task_ids"))
                .or_else(|| result.data.get("taskIds"))
                .or_else(|| result.data.get("task_id")),
        ))
    {
        ids.insert(id);
    }
    for coverage in &result.task_coverage {
        if matches!(
            coverage.status,
            WorkflowV2TaskCoverageStatus::Accepted
                | WorkflowV2TaskCoverageStatus::Noop
                | WorkflowV2TaskCoverageStatus::Partial
                | WorkflowV2TaskCoverageStatus::Blocked
        ) && !coverage.task_id.trim().is_empty()
        {
            ids.insert(coverage.task_id.trim().to_string());
        }
    }
    ids.into_iter().collect()
}

pub(super) fn concrete_evidence_from_branch_result(
    result: &WorkflowV2Result,
) -> Vec<serde_json::Value> {
    let accepted_or_noop = matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    );
    let mut evidence = Vec::new();
    evidence.extend(branch_evidence_records(result, accepted_or_noop));
    evidence.extend(branch_command_records(result, accepted_or_noop));
    evidence.extend(branch_file_records(result));
    evidence.extend(branch_artifact_records(result));
    evidence.extend(branch_data_evidence_refs(result));
    evidence.extend(branch_task_coverage_records(result, accepted_or_noop));
    evidence.extend(branch_residual_gap_records(result, accepted_or_noop));
    evidence
}

pub(super) fn branch_evidence_records(
    result: &WorkflowV2Result,
    accepted_or_noop: bool,
) -> Vec<serde_json::Value> {
    result
        .evidence
        .iter()
        .filter(|item| {
            !accepted_or_noop
                || matches!(
                    item.kind,
                    WorkflowV2EvidenceKind::Implementation | WorkflowV2EvidenceKind::Test
                )
        })
        .map(|item| {
            serde_json::json!({
                "kind": item.kind,
                "summary": item.summary.clone(),
                "source": item.source.clone(),
            })
        })
        .collect()
}

pub(super) fn branch_command_records(
    result: &WorkflowV2Result,
    accepted_or_noop: bool,
) -> Vec<serde_json::Value> {
    result
        .commands_run
        .iter()
        .filter(|command| !accepted_or_noop || command.status == WorkflowV2CommandStatus::Succeeded)
        .map(|command| {
            serde_json::json!({
                "kind": "command",
                "command": command.command.clone(),
                "status": command.status,
                "exit_code": command.exit_code,
                "output_summary": command.output_summary.clone(),
            })
        })
        .collect()
}

pub(super) fn branch_file_records(result: &WorkflowV2Result) -> Vec<serde_json::Value> {
    result
        .files_changed
        .iter()
        .map(|file| {
            serde_json::json!({
                "kind": "file_changed",
                "path": file.path.clone(),
                "purpose": file.purpose.clone(),
            })
        })
        .collect()
}

pub(super) fn branch_artifact_records(result: &WorkflowV2Result) -> Vec<serde_json::Value> {
    result
        .artifacts
        .iter()
        .map(|artifact| {
            serde_json::json!({
                "kind": "artifact",
                "id": artifact.id.clone(),
                "path": artifact.path.clone(),
                "description": artifact.description.clone(),
            })
        })
        .collect()
}

pub(super) fn branch_data_evidence_refs(result: &WorkflowV2Result) -> Vec<serde_json::Value> {
    evidence_refs_from_generated_value(&result.data)
        .into_iter()
        .map(|reference| {
            serde_json::json!({
                "kind": "evidence_ref",
                "summary": reference,
            })
        })
        .collect()
}

pub(super) fn branch_task_coverage_records(
    result: &WorkflowV2Result,
    accepted_or_noop: bool,
) -> Vec<serde_json::Value> {
    result
        .task_coverage
        .iter()
        .filter(|coverage| task_coverage_counts_as_evidence(coverage, accepted_or_noop))
        .flat_map(|coverage| {
            coverage.evidence.iter().map(|item| {
                serde_json::json!({
                    "kind": item.kind,
                    "summary": item.summary.clone(),
                    "source": item.source.clone(),
                    "task_id": coverage.task_id.clone(),
                })
            })
        })
        .collect()
}

pub(super) fn task_coverage_counts_as_evidence(
    coverage: &WorkflowV2TaskCoverage,
    accepted_or_noop: bool,
) -> bool {
    !accepted_or_noop
        || matches!(
            coverage.status,
            WorkflowV2TaskCoverageStatus::Accepted | WorkflowV2TaskCoverageStatus::Noop
        )
}

pub(super) fn branch_residual_gap_records(
    result: &WorkflowV2Result,
    accepted_or_noop: bool,
) -> Vec<serde_json::Value> {
    if accepted_or_noop {
        return Vec::new();
    }
    result
        .residual_gaps
        .iter()
        .map(|gap| {
            serde_json::json!({
                "kind": "review",
                "summary": format!("residual gap {}: {}", gap.id, gap.description),
                "id": gap.id.clone(),
                "severity": gap.severity.clone(),
            })
        })
        .collect()
}

pub(super) fn string_array_from_data(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Array(values)) => values
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

pub(super) fn add_write_fanout_evidence(
    result: &mut WorkflowV2Result,
    plan: &WorkflowV2WritePlan,
    fallback_reason: Option<String>,
) {
    let status_evidence = match result.status {
        WorkflowV2Status::NeedsReview => Some((
            crate::WorkflowV2EvidenceKind::Review,
            "write fanout branch findings were retained as typed review/remediation data for workflow.js",
        )),
        _ => None,
    };
    if let Some((kind, summary)) = status_evidence {
        result
            .evidence
            .push(crate::WorkflowV2Evidence::new(kind, summary));
    }
    let detail = fallback_reason.unwrap_or_else(|| {
        format!(
            "write-capable fanout used {:?} planning across {} wave(s)",
            plan.mode,
            plan.waves.len()
        )
    });
    result.evidence.push(crate::WorkflowV2Evidence::new(
        crate::WorkflowV2EvidenceKind::Implementation,
        detail,
    ));
}

pub(super) fn count_results_with_status(
    results: &[WorkflowV2Result],
    status: WorkflowV2Status,
) -> usize {
    results
        .iter()
        .filter(|result| result.status == status)
        .count()
}

pub(super) fn record_write_peak(peak: &AtomicUsize, observed: usize) {
    let mut current = peak.load(Ordering::SeqCst);
    while observed > current {
        match peak.compare_exchange(current, observed, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return,
            Err(next) => current = next,
        }
    }
}

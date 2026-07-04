fn normalize_implementation_branch_contract(outcome: &mut WorkflowV2BranchOutcome) {
    if is_actual_execution_failure(outcome) || matches!(outcome.status, WorkflowV2Status::Cancelled)
    {
        return;
    }
    let missing_id = outcome.item_id.trim().is_empty();
    let canonical_task_ids = outcome
        .result
        .as_ref()
        .map(canonical_task_ids_from_result)
        .unwrap_or_default();
    let evidence = outcome
        .result
        .as_ref()
        .map(evidence_summaries_from_result)
        .unwrap_or_default();
    if !missing_id && !canonical_task_ids.is_empty() && !evidence.is_empty() {
        return;
    }
    let mut result = outcome.result.take().unwrap_or_else(|| WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: format!(
            "implementation branch '{}' returned malformed outcome evidence",
            outcome.item_id
        ),
        ..WorkflowV2Result::default()
    });
    result.status = WorkflowV2Status::NeedsReview;
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "implementation branch outcome must include item_id/id, canonical_task_ids, status, and concrete evidence before it can unblock dependent work",
    ));
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "invalid_implementation_branch_contract_{}",
            sanitize_v2_id(&outcome.item_id)
        ),
        description: format!(
            "missing branch outcome contract fields: {}",
            missing_contract_fields(missing_id, &canonical_task_ids, &evidence).join(", ")
        ),
        severity: Some("review".to_string()),
    });
    result.data = merge_branch_contract_data(result.data, &canonical_task_ids, &evidence);
    outcome.status = WorkflowV2Status::NeedsReview;
    outcome.failure_kind = Some(BranchFailureKind::Contract);
    outcome.result = Some(result);
}

fn is_actual_execution_failure(outcome: &WorkflowV2BranchOutcome) -> bool {
    if matches!(
        outcome.failure_kind,
        Some(BranchFailureKind::Execution | BranchFailureKind::Safety)
    ) {
        return true;
    }
    if outcome.status != WorkflowV2Status::Failed {
        return false;
    }
    let error = outcome
        .error
        .as_deref()
        .or_else(|| {
            outcome
                .result
                .as_ref()
                .and_then(|result| result.data.get("error").and_then(serde_json::Value::as_str))
        })
        .unwrap_or_default()
        .to_ascii_lowercase();
    error.contains("agent transport failed")
        || error.contains("tool execution failed")
        || error.contains("process failed")
        || error.contains("timed out")
        || error.contains("rate limit")
}

fn failure_kind_for_outcome(outcome: &WorkflowV2BranchOutcome) -> Option<BranchFailureKind> {
    match outcome.status {
        WorkflowV2Status::Failed => {
            let error = outcome
                .error
                .as_deref()
                .or_else(|| {
                    outcome.result.as_ref().and_then(|result| {
                        result.data.get("error").and_then(serde_json::Value::as_str)
                    })
                })
                .unwrap_or_default()
                .to_ascii_lowercase();
            if error.contains("changed files outside")
                || error.contains("outside declared ownership")
                || error.contains("read-only")
                || error.contains("patch apply")
                || error.contains("ownership")
            {
                Some(BranchFailureKind::Safety)
            } else if error.contains("agent transport failed")
                || error.contains("tool execution failed")
                || error.contains("process failed")
                || error.contains("timed out")
                || error.contains("rate limit")
                || error.contains("cancelled")
            {
                Some(BranchFailureKind::Execution)
            } else if outcome.result.is_some() {
                Some(BranchFailureKind::Semantic)
            } else {
                Some(BranchFailureKind::Contract)
            }
        }
        WorkflowV2Status::Blocked | WorkflowV2Status::NeedsReview => {
            Some(BranchFailureKind::Semantic)
        }
        WorkflowV2Status::Cancelled => Some(BranchFailureKind::Execution),
        _ => None,
    }
}

fn missing_contract_fields(
    missing_id: bool,
    canonical_task_ids: &[String],
    evidence: &[String],
) -> Vec<&'static str> {
    let mut fields = Vec::new();
    if missing_id {
        fields.push("item_id");
    }
    if canonical_task_ids.is_empty() {
        fields.push("canonical_task_ids");
    }
    if evidence.is_empty() {
        fields.push("evidence");
    }
    fields
}

fn merge_branch_contract_data(
    data: serde_json::Value,
    canonical_task_ids: &[String],
    evidence: &[String],
) -> serde_json::Value {
    let mut object = data.as_object().cloned().unwrap_or_default();
    object.insert(
        "canonical_task_ids".to_string(),
        serde_json::json!(canonical_task_ids),
    );
    object.insert("evidence".to_string(), serde_json::json!(evidence));
    serde_json::Value::Object(object)
}

fn fanout_outcome_views(outcomes: &[WorkflowV2BranchOutcome]) -> Vec<serde_json::Value> {
    outcomes
        .iter()
        .map(|outcome| {
            let result = outcome.result.clone().unwrap_or_default();
            let canonical_task_ids = canonical_task_ids_from_result(&result);
            let evidence = evidence_summaries_from_result(&result);
            serde_json::json!({
                "item_id": outcome.item_id,
                "id": outcome.item_id,
                "role": outcome.role,
                "status": outcome.status,
                "failure_kind": outcome.failure_kind,
                "canonical_task_ids": canonical_task_ids,
                "evidence": evidence,
                "result": outcome.result,
                "error": outcome.error,
                "completion_evidence": outcome.completion_evidence,
            })
        })
        .collect()
}

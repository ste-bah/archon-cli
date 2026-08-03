use super::*;

pub(super) fn downgrade_read_only_accepted_task_coverage(
    call: &WorkflowV2HostCall,
    result: &mut WorkflowV2Result,
) {
    if call.write_mode.is_some() || call.method == WorkflowV2HostMethod::Implementation {
        return;
    }
    let has_implementation_evidence = !result.files_changed.is_empty()
        || result
            .evidence
            .iter()
            .any(|evidence| matches!(evidence.kind, WorkflowV2EvidenceKind::Implementation));
    let verified_task_ids = focused_verification_accepted_task_ids(call, result);
    let mut downgraded = Vec::new();
    for coverage in &mut result.task_coverage {
        if coverage.status != WorkflowV2TaskCoverageStatus::Accepted {
            continue;
        }
        if verified_task_ids.contains(&coverage.task_id) {
            continue;
        }
        let coverage_has_implementation_evidence = coverage.evidence.iter().any(|evidence| {
            matches!(
                evidence.kind,
                WorkflowV2EvidenceKind::Implementation | WorkflowV2EvidenceKind::Test
            )
        });
        if !has_implementation_evidence && !coverage_has_implementation_evidence {
            coverage.status = WorkflowV2TaskCoverageStatus::Unknown;
            coverage.evidence.push(WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Review,
                "read-only workflow calls cannot accept implementation task coverage without concrete implementation or test evidence",
            ));
            downgraded.push(coverage.task_id.clone());
        }
    }
    if downgraded.is_empty() {
        return;
    }
    if matches!(result.status, WorkflowV2Status::Accepted) {
        result.status = WorkflowV2Status::NeedsReview;
    }
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        format!(
            "downgraded read-only accepted task coverage to unknown for: {}",
            downgraded.join(", ")
        ),
    ));
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!("read_only_task_acceptance_{}", sanitize_v2_gap_id(&call.id)),
        description: "read-only call claimed implementation acceptance without concrete implementation/test evidence".to_string(),
        severity: Some("review".to_string()),
    });
}

pub(super) fn guard_empty_items_output(execution: &WorkflowV2CallExecution, result: &mut WorkflowV2Result) {
    if !call_declares_items_output(execution) || !items_output_is_empty(result) {
        return;
    }
    if result
        .task_coverage
        .iter()
        .any(|coverage| coverage.status == WorkflowV2TaskCoverageStatus::Noop)
    {
        return;
    }
    if matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) {
        result.status = WorkflowV2Status::NeedsReview;
    }
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        "items-producing call returned an empty data.items array without typed no-op proof",
    ));
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "empty_items_output_{}",
            sanitize_v2_gap_id(&execution.call.id)
        ),
        description:
            "implementation inventory cannot be empty unless every required task has concrete no-op proof"
                .to_string(),
        severity: Some("review".to_string()),
    });
}

pub(super) fn call_declares_items_output(execution: &WorkflowV2CallExecution) -> bool {
    execution
        .call
        .options
        .extra
        .get("outputs")
        .is_some_and(outputs_value_declares_items)
}

pub(super) fn outputs_value_declares_items(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Array(values) => values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .any(|value| value.eq_ignore_ascii_case("items")),
        serde_json::Value::String(value) => value.eq_ignore_ascii_case("items"),
        _ => false,
    }
}

pub(super) fn items_output_is_empty(result: &WorkflowV2Result) -> bool {
    result
        .data
        .get("items")
        .and_then(serde_json::Value::as_array)
        .is_some_and(Vec::is_empty)
}

pub(crate) fn sanitize_v2_gap_id(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

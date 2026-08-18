use super::*;

pub(super) fn downgrade_read_only_accepted_task_coverage(
    call: &WorkflowV2HostCall,
    result: &mut WorkflowV2Result,
) {
    if call.write_mode.is_some() || call.method == WorkflowV2HostMethod::Implementation {
        return;
    }
    // This guard exists for exactly one thing: a read-only FOCUSED-VERIFICATION
    // call (verification-wave / review-verification-wave) that accepts a task as
    // implemented without concrete evidence. That is the only read-only call
    // whose accepted task coverage is an implementation-acceptance claim.
    //
    // Every other read-only call carries accepted coverage as structural
    // bookkeeping, not an implementation claim: a no-op proof credits on
    // acceptance-criteria inspection (its substantive check is
    // `noop_acceptance_criteria_satisfied` in completion_credit.rs); inventory,
    // shape/dependency repair, reconcile, planning and discovery build or mend
    // the plan. Downgrading their coverage to needs_review sends the caller back
    // into a repair loop that can never converge, because a read-only call can
    // never mint the implementation/test evidence this guard then demands.
    // Observed live twice from the same root cause: noop-proof looping
    // repair-2-1..2-3 (an earlier, too-narrow fix exempted only that), then
    // inventory-shape-repair <-> dependency-graph-repair cycling on this exact
    // gap. Fire only for the calls that mint FocusedVerification evidence; the
    // write/implementation calls are already returned above.
    if crate::v2::completion_evidence::task_completion_evidence_kind(&call.id)
        != Some(crate::WorkflowV2TaskCompletionEvidenceKind::FocusedVerification)
    {
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

pub(super) fn guard_empty_items_output(
    execution: &WorkflowV2CallExecution,
    result: &mut WorkflowV2Result,
) {
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

pub fn sanitize_v2_gap_id(raw: &str) -> String {
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

/// The typed result a host call fails with when the host itself could not
/// produce one — a transport error, an unparseable payload, a rejected script
/// request. It sits beside [`sanitize_v2_gap_id`] because it is the only
/// caller that has to agree with the gap ids `helpers_a` already mints for the
/// same call; the binary carried a byte-identical private copy of the
/// sanitizer until this moved.
pub fn failed_v2_result(call_id: &str, err: impl std::fmt::Display) -> WorkflowV2Result {
    let error = err.to_string();
    WorkflowV2Result {
        status: WorkflowV2Status::Failed,
        summary: format!("workflow v2 call '{call_id}' failed: {error}"),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Blocker,
            error.clone(),
        )],
        residual_gaps: vec![WorkflowV2ResidualGap {
            id: format!("v2_call_failed_{}", sanitize_v2_gap_id(call_id)),
            description: error.clone(),
            severity: Some("blocking".to_string()),
        }],
        data: serde_json::json!({ "error": error }),
        ..WorkflowV2Result::default()
    }
}

#[cfg(test)]
mod downgrade_tests {
    use super::*;
    use crate::v2::result::WorkflowV2TaskCoverage;

    fn read_only_call(id: &str) -> WorkflowV2HostCall {
        WorkflowV2HostCall {
            id: id.to_string(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        }
    }

    fn accepted_inspection_coverage() -> WorkflowV2Result {
        let mut result = WorkflowV2Result::accepted("verified against acceptance criteria");
        result.task_coverage.push(WorkflowV2TaskCoverage {
            task_id: "TASK-TDL-010".to_string(),
            status: WorkflowV2TaskCoverageStatus::Accepted,
            summary: "registry schema already satisfies every criterion".to_string(),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Inspection,
                "registry_schema_v1 has 4 #[test] fns; structs defined",
            )],
        });
        result
    }

    /// The live loop: a read-only no-op proof accepts a task on inspection
    /// evidence and was downgraded to needs_review for lacking impl/test
    /// evidence, which a read-only call can never mint — spinning the repair
    /// cycle to the cap. A no-op proof call must keep its accepted coverage.
    #[test]
    fn a_noop_proof_keeps_its_accepted_coverage() {
        let mut result = accepted_inspection_coverage();
        downgrade_read_only_accepted_task_coverage(
            &read_only_call("noop-proof-verification-2"),
            &mut result,
        );
        assert_eq!(result.status, WorkflowV2Status::Accepted);
        assert_eq!(
            result.task_coverage[0].status,
            WorkflowV2TaskCoverageStatus::Accepted
        );
        assert!(
            result.residual_gaps.is_empty(),
            "no read-only-acceptance gap for a no-op proof: {:?}",
            result.residual_gaps
        );
    }

    #[test]
    fn a_noop_proof_reverification_keeps_its_accepted_coverage() {
        let mut result = accepted_inspection_coverage();
        downgrade_read_only_accepted_task_coverage(
            &read_only_call("noop-proof-reverification-2-3"),
            &mut result,
        );
        assert_eq!(result.status, WorkflowV2Status::Accepted);
        assert_eq!(
            result.task_coverage[0].status,
            WorkflowV2TaskCoverageStatus::Accepted
        );
    }

    /// The guard still bites the calls it is for: both focused-verification
    /// call types are read-only and must not accept a task as implemented on
    /// inspection alone.
    #[test]
    fn focused_verification_calls_are_still_downgraded() {
        for id in ["verification-wave-1", "review-verification-wave-1"] {
            let mut result = accepted_inspection_coverage();
            downgrade_read_only_accepted_task_coverage(&read_only_call(id), &mut result);
            assert_eq!(
                result.status,
                WorkflowV2Status::NeedsReview,
                "{id} must be downgraded"
            );
            assert_eq!(
                result.task_coverage[0].status,
                WorkflowV2TaskCoverageStatus::Unknown,
                "{id}"
            );
            assert!(!result.residual_gaps.is_empty(), "{id}");
        }
    }

    /// The root fix: EVERY read-only structural call — inventory, shape and
    /// dependency repair, reconcile, planning, remediation-inventory, evidence
    /// repair, discovery, and no-op proofs — carries accepted coverage as
    /// bookkeeping, not an implementation claim, and must never be downgraded.
    /// Downgrading any of them loops the caller into non-converging repair.
    /// This is the exhaustive check the two earlier too-narrow fixes lacked.
    #[test]
    fn no_structural_read_only_call_is_ever_downgraded() {
        for id in [
            "canonical-implementation-inventory",
            "inventory-shape-repair-1",
            "inventory-shape-repair-2",
            "dependency-graph-repair-1",
            "dependency-graph-repair-deadlock-1",
            "task-universe-reconcile-1",
            "evidence-repair-1",
            "verification-plan-1",
            "verification-plan-repair-1-1",
            "remediation-inventory-1",
            "initial-readonly-discovery",
            "noop-proof-verification-2",
            "noop-proof-reverification-2-3",
        ] {
            let mut result = accepted_inspection_coverage();
            downgrade_read_only_accepted_task_coverage(&read_only_call(id), &mut result);
            assert_eq!(
                result.status,
                WorkflowV2Status::Accepted,
                "structural call {id} must not be downgraded"
            );
            assert_eq!(
                result.task_coverage[0].status,
                WorkflowV2TaskCoverageStatus::Accepted,
                "structural call {id} coverage must stay accepted"
            );
            assert!(
                result.residual_gaps.is_empty(),
                "structural call {id} must not accrue a read-only-acceptance gap"
            );
        }
    }
}

//! Generated-PRD contract reducers: which agent errors a reducer may be
//! re-asked about at the host level, and the review result that stands in
//! for a reducer whose output stayed malformed. Split from the dispatcher so
//! both files stay readable.
use super::*;

pub(super) fn generated_prd_contract_repairable_reduce(
    call: &WorkflowV2HostCall,
    error: &WorkflowV2AgentError,
) -> bool {
    call.method == WorkflowV2HostMethod::Reduce
        && generated_prd_contract_reduce_id(&call.id)
        && repairable_agent_contract_error(error)
}

fn generated_prd_contract_reduce_id(call_id: &str) -> bool {
    call_id == "canonical-implementation-inventory"
        || call_id.starts_with("inventory-shape-repair-")
        || call_id.starts_with("task-universe-reconcile-")
        || call_id.starts_with("dependency-graph-repair-")
        || call_id.starts_with("target-file-discovery-")
        || call_id.starts_with("verification-requirements-discovery-")
        || call_id.starts_with("artifact-requirements-discovery-")
        || call_id.starts_with("provider-environment-discovery-")
        || call_id.starts_with("evidence-repair-")
}

fn repairable_agent_contract_error(error: &WorkflowV2AgentError) -> bool {
    match error {
        WorkflowV2AgentError::MalformedOutput(_)
        | WorkflowV2AgentError::InvalidResult(_)
        | WorkflowV2AgentError::RestoredContextSummary
        | WorkflowV2AgentError::ConfirmationQuestion
        | WorkflowV2AgentError::ReadOnlyChangedFiles
        // An empty reply is the provider's, and a reducer that was never
        // answered is exactly what the review stand-in exists for.
        | WorkflowV2AgentError::EmptyReply => true,
        WorkflowV2AgentError::RepairExhausted {
            first_error,
            repair_error,
        } => {
            repairable_agent_contract_error(first_error)
                && repairable_agent_contract_error(repair_error)
        }
        // NOT repairable here, deliberately. This error has already spent its
        // bounded re-ask inside the repair loop with the violation quoted back;
        // admitting it to this set would hand it a third attempt through a
        // different mechanism and convert a terminal failure into a
        // NeedsReview result carrying `"items": []` — precisely the empty
        // declared output the enforcement exists to stop, re-introduced one
        // layer up.
        WorkflowV2AgentError::DeclaredOutputUnsatisfied { .. }
        | WorkflowV2AgentError::Transport(_)
        | WorkflowV2AgentError::NotificationDelivery(_)
        | WorkflowV2AgentError::PlanOnlyImplementation
        | WorkflowV2AgentError::ImplementationAcceptedWithoutChanges
        | WorkflowV2AgentError::ImplementationNoopWithoutTaskCoverage
        | WorkflowV2AgentError::ImplementationNoopWithDeclaredRequiredTools
        | WorkflowV2AgentError::ImplementationAcceptedWithRequiredToolUnexercised(_)
        | WorkflowV2AgentError::ImplementationNoopMissingProjectArtifactEvidence
        | WorkflowV2AgentError::ImplementationChangedFilesOutsideOwnership(_)
        | WorkflowV2AgentError::DeclaredArtifactAbsent(_) => false,
    }
}

pub(super) fn repairable_generated_reduce_result(
    call_id: &str,
    error: &WorkflowV2AgentError,
) -> WorkflowV2Result {
    let message = format!(
        "generated PRD reducer '{}' returned repairable malformed contract output: {}",
        call_id, error
    );
    WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: message.clone(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Review,
            message.clone(),
        )],
        residual_gaps: vec![WorkflowV2ResidualGap {
            id: format!(
                "repairable_generated_reduce_contract_{}",
                sanitize_generated_contract_gap_id(call_id)
            ),
            description: message.clone(),
            severity: Some("review".to_string()),
        }],
        data: serde_json::json!({
            "items": [],
            "unresolved_issues": [{
                "kind": "inventory_shape_repair",
                "field": "result_envelope",
                "message": message,
                "item_id": null,
                "canonical_task_ids": []
            }],
            "repairable_schema_failure": true,
            "failed_call_id": call_id,
        }),
        ..WorkflowV2Result::default()
    }
}

fn sanitize_generated_contract_gap_id(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

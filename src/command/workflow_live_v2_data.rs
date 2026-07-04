use archon_workflow::{
    BranchFailureKind, WorkflowError, WorkflowV2AgentRequest, WorkflowV2BranchOutcome,
    WorkflowV2CallExecution, WorkflowV2CommandStatus, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2FanoutItem, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2ResidualGap,
    WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Status, WorkflowV2TaskCompletionEvidence,
    WorkflowV2TaskCompletionEvidenceKind, WorkflowV2TaskCoverageStatus, WorkflowV2WriteMode,
};

use super::super::workflow_live_generated_contract::{
    canonical_task_ids_from_generated_value, evidence_refs_from_generated_value,
};
use super::workflow_live_v2_aggregate::attach_branch_evidence;
use super::workflow_live_v2_verification::{
    FOCUSED_VERIFICATION_EVIDENCE_CONTRACT_VERSION, normalize_focused_verification_outcome,
    stamp_focused_verification_input,
};

pub(super) fn execution_with_resolved_source(
    execution: &WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<WorkflowV2CallExecution> {
    if execution.input.get("source_data").is_some() {
        let mut enriched = execution.clone();
        if execution.call.method == WorkflowV2HostMethod::Reduce {
            if let Some(object) = enriched.input.as_object_mut() {
                if let Some(source_data) = object.get("source_data").cloned() {
                    object.insert("source_data".to_string(), source_pack_value(&source_data));
                }
            }
        }
        return Ok(enriched);
    }
    let Some(source) = execution.call.options.source.as_deref() else {
        return Ok(execution.clone());
    };
    let mut source_data = resolve_source_value(source, v2_store)?;
    if execution.call.method == WorkflowV2HostMethod::Reduce {
        source_data = source_pack_value(&source_data);
    }
    let mut enriched = execution.clone();
    if let Some(object) = enriched.input.as_object_mut() {
        object.insert(
            "source".to_string(),
            serde_json::Value::String(source.to_string()),
        );
        object.insert("source_data".to_string(), source_data);
    } else {
        enriched.input = serde_json::json!({
            "input": enriched.input,
            "source": source,
            "source_data": source_data,
        });
    }
    Ok(enriched)
}

include!("workflow_live_v2_data_source_pack.rs");

include!("workflow_live_v2_data_source.rs");

include!("workflow_live_v2_data_fanout_result.rs");

include!("workflow_live_v2_data_branch_contract.rs");

include!("workflow_live_v2_data_evidence.rs");

include!("workflow_live_v2_data_agent_request.rs");

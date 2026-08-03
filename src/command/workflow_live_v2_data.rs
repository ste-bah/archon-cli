use archon_workflow::{
    BranchFailureKind, WorkflowError, WorkflowV2AgentRequest, WorkflowV2BranchOutcome,
    WorkflowV2CallExecution, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FanoutItem,
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2ResidualGap, WorkflowV2Result,
    WorkflowV2ResultStore, WorkflowV2Status, WorkflowV2WriteMode,
};

use super::workflow_live_v2_verification::{
    normalize_focused_verification_outcome, stamp_focused_verification_input,
};
use archon_workflow::v2::branch_evidence::attach_branch_evidence;
use archon_workflow::v2::completion_evidence::{
    attach_completion_evidence_for_call, canonical_task_ids_from_result,
    evidence_summaries_from_result,
};

pub(super) fn execution_with_resolved_source(
    execution: &WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
) -> archon_workflow::WorkflowResult<WorkflowV2CallExecution> {
    if execution.input.get("source_data").is_some() {
        let mut enriched = execution.clone();
        if execution.call.method == WorkflowV2HostMethod::Reduce
            && let Some(object) = enriched.input.as_object_mut()
            && let Some(source_data) = object.get("source_data").cloned()
        {
            object.insert("source_data".to_string(), source_pack_value(&source_data));
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

#[path = "workflow_live_v2_data_source_pack.rs"]
mod workflow_live_v2_data_source_pack;
pub(super) use workflow_live_v2_data_source_pack::source_pack_value;

#[path = "workflow_live_v2_data_source.rs"]
mod workflow_live_v2_data_source;
pub(super) use workflow_live_v2_data_source::fanout_items_for_call;
use workflow_live_v2_data_source::*;

#[path = "workflow_live_v2_data_fanout_result.rs"]
mod workflow_live_v2_data_fanout_result;
pub(super) use workflow_live_v2_data_fanout_result::result_from_fanout_report;

#[path = "workflow_live_v2_data_branch_contract.rs"]
mod workflow_live_v2_data_branch_contract;
use workflow_live_v2_data_branch_contract::*;

#[path = "workflow_live_v2_data_evidence.rs"]
mod workflow_live_v2_data_evidence;
use workflow_live_v2_data_evidence::*;

#[path = "workflow_live_v2_data_agent_request.rs"]
mod workflow_live_v2_data_agent_request;
pub(super) use workflow_live_v2_data_agent_request::v2_agent_request;
use workflow_live_v2_data_agent_request::*;

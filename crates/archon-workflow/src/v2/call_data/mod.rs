//! Typed shaping of what goes into and comes out of a V2 host call.
//!
//! Three jobs, all pure over this crate's own types:
//!
//! * resolve a call's declared `source` expression against the result store and
//!   fold the resolved data into the call input;
//! * expand a fanout call into its per-item branch calls, stripping any
//!   agent-authored tool declaration at the single builder both write and
//!   read-only branches share;
//! * normalize what comes back — branch outcomes get a failure classification,
//!   an implementation branch gets its outcome contract enforced, and the
//!   fanout report is reduced to one typed result.
//!
//! It sits in this crate rather than the binary because every input and output
//! is a type this crate owns. What stayed behind is the dispatch that decides
//! *when* to call these, not what they compute.

use crate::v2::branch_evidence::attach_branch_evidence;
use crate::v2::completion_evidence::{
    attach_completion_evidence_for_call, canonical_task_ids_from_result,
    evidence_summaries_from_result,
};
use crate::v2::verification::{
    normalize_focused_verification_outcome, stamp_focused_verification_input,
};
use crate::{
    BranchFailureKind, WorkflowError, WorkflowV2AgentRequest, WorkflowV2BranchOutcome,
    WorkflowV2CallExecution, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FanoutItem,
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2ResidualGap, WorkflowV2Result,
    WorkflowV2ResultStore, WorkflowV2Status, WorkflowV2WriteMode,
};

pub fn execution_with_resolved_source(
    execution: &WorkflowV2CallExecution,
    v2_store: &WorkflowV2ResultStore,
) -> crate::WorkflowResult<WorkflowV2CallExecution> {
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

pub use crate::v2::source_pack::source_pack_value;

mod source;
pub use source::fanout_items_for_call;
use source::*;

mod fanout_result;
pub use fanout_result::{WorkflowV2NormalizedFanout, result_from_fanout_report};

mod branch_contract;
use branch_contract::*;

mod evidence;
use evidence::*;

mod agent_request;
pub use agent_request::v2_agent_request;
use agent_request::*;

// `call_data_tests*`, not `tests*`: the runtime-genericity gate identifies test
// sources by a `_tests` infix and would otherwise scan these as runtime code —
// they carry fixture-domain vocabulary by design.
#[cfg(test)]
#[path = "call_data_tests.rs"]
mod tests;

//! Generation-owned persistence for fixed-decomposition call evidence.

use archon_workflow::{
    WorkflowError, WorkflowResult, WorkflowStore, WorkflowUiEvent, WorkflowV2CallRecord,
    WorkflowV2Checkpoint, WorkflowV2ResultStore,
};

use crate::command::workflow_decompose_state::{FixedCallProjectionKind, project_fixed_call};

pub(super) fn persist_generation_owned_call(
    workflow_store: &WorkflowStore,
    run_id: &str,
    v2_store: &WorkflowV2ResultStore,
    record: &WorkflowV2CallRecord,
    kind: FixedCallProjectionKind,
    expected_generation: Option<u64>,
) -> WorkflowResult<Option<WorkflowUiEvent>> {
    let operation = |locked: &WorkflowStore| {
        if let Some(expected) = expected_generation {
            let current = locked.load_state(run_id)?;
            if current.generation != expected {
                return Err(WorkflowError::ControlCancelled(format!(
                    "fixed executor generation {expected} cannot persist call {} for run {run_id}; current generation is {}",
                    record.call.id, current.generation
                )));
            }
        }
        if kind != FixedCallProjectionKind::Reused {
            v2_store.save_call_record(record)?;
        }
        let event = project_fixed_call(locked, run_id, record, kind)?;
        if kind != FixedCallProjectionKind::Started {
            let mut checkpoint = v2_store
                .load_checkpoint()?
                .unwrap_or_else(WorkflowV2Checkpoint::default);
            if archon_workflow::v2::script::is_reusable_status(record.status) {
                checkpoint.mark_completed(&record.call.id);
            } else {
                checkpoint.remove_completed_call(&record.call.id);
            }
            v2_store.save_checkpoint(&checkpoint)?;
        }
        Ok(event)
    };
    if expected_generation.is_some() {
        workflow_store.with_run_lock(run_id, operation)
    } else {
        operation(workflow_store)
    }
}

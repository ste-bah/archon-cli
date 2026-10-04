//! Call publication and generation fencing share the lifecycle run lock.
use crate::command::workflow_decompose_state::{FixedCallProjectionKind, project_fixed_call};
use archon_workflow::{
    WorkflowError, WorkflowResult, WorkflowStore, WorkflowUiEvent, WorkflowV2CallRecord,
    WorkflowV2Checkpoint, WorkflowV2ResultStore,
};

pub(super) enum CallPublication {
    Published(Option<WorkflowUiEvent>),
    Superseded,
}

pub(super) fn persist_generation_owned_call(
    store: &WorkflowStore,
    run_id: &str,
    v2: &WorkflowV2ResultStore,
    record: &WorkflowV2CallRecord,
    kind: FixedCallProjectionKind,
    generation: Option<u64>,
) -> WorkflowResult<Option<WorkflowUiEvent>> {
    match publish(store, run_id, v2, record, kind, generation, false)? {
        CallPublication::Published(event) => Ok(event),
        CallPublication::Superseded => Err(WorkflowError::ControlCancelled(
            "call publication superseded".into(),
        )),
    }
}

pub(super) fn persist_dispatched_call(
    store: &WorkflowStore,
    run_id: &str,
    v2: &WorkflowV2ResultStore,
    record: &WorkflowV2CallRecord,
    generation: Option<u64>,
) -> WorkflowResult<CallPublication> {
    publish(
        store,
        run_id,
        v2,
        record,
        FixedCallProjectionKind::Executed,
        generation,
        true,
    )
}

fn publish(
    store: &WorkflowStore,
    run_id: &str,
    v2: &WorkflowV2ResultStore,
    record: &WorkflowV2CallRecord,
    kind: FixedCallProjectionKind,
    generation: Option<u64>,
    redispatch: bool,
) -> WorkflowResult<CallPublication> {
    #[cfg(test)]
    if kind == FixedCallProjectionKind::Executed {
        publication_hook::run(store.run_dir(run_id));
    }
    store.with_run_lock(run_id, |locked| {
        let current = locked.load_state(run_id)?;
        if let Some(expected) = generation
            && current.generation != expected
        {
            if redispatch && current.status == archon_workflow::RunStatus::Running
                && current.execution_owned_at(expected)
            {
                return Ok(CallPublication::Superseded);
            }
            archon_workflow::poll_v2_run_control(locked, run_id, &record.call.id)?;
            return Err(WorkflowError::ControlCancelled(format!(
                "executor generation {expected} cannot persist call {} for run {run_id}; current generation is {}",
                record.call.id, current.generation
            )));
        }
        archon_workflow::poll_v2_run_control(locked, run_id, &record.call.id)?;
        if kind != FixedCallProjectionKind::Reused {
            v2.save_call_record(record)?;
        }
        let event = project_fixed_call(locked, run_id, record, kind)?;
        if kind != FixedCallProjectionKind::Started {
            let mut checkpoint = v2.load_checkpoint()?.unwrap_or_else(WorkflowV2Checkpoint::default);
            if archon_workflow::v2::script::is_reusable_status(record.status) {
                checkpoint.mark_completed(&record.call.id);
            } else {
                checkpoint.remove_completed_call(&record.call.id);
            }
            v2.save_checkpoint(&checkpoint)?;
        }
        Ok(CallPublication::Published(event))
    })
}

#[cfg(test)]
pub(super) mod publication_hook {
    use std::{collections::BTreeMap, path::PathBuf, sync::Mutex};
    type Hook = Box<dyn FnOnce() + Send>;
    static HOOKS: Mutex<BTreeMap<PathBuf, Hook>> = Mutex::new(BTreeMap::new());

    pub(crate) fn install(path: PathBuf, hook: Hook) {
        HOOKS.lock().expect("hooks").insert(path, hook);
    }

    pub(super) fn run(path: PathBuf) {
        let hook = HOOKS.lock().expect("hooks").remove(&path);
        if let Some(hook) = hook {
            hook();
        }
    }
}

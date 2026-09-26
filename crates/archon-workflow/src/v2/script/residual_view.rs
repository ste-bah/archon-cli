//! Issue-117: the residual plan on the pre-acceptance checkpoint's view.

use std::path::Path;

use serde_json::{Value, json};

use super::super::{
    WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2Result,
    WorkflowV2ResultStore,
};
use super::{
    DESCRIPTION_CHARS, PlannedRound, RESIDUAL_GAPS_KEY, RESIDUAL_GAPS_MARKER, clip, plan_from,
};
use crate::task_universe::WorkflowV2TaskUniverse;

/// The checkpoint the prelude records once a round's remediation returned.
pub fn done_checkpoint_id(key: &str) -> String {
    format!("{key}-done")
}

/// This session's records, as the plan at the slot reads them.
pub fn session_records(store: &WorkflowV2ResultStore) -> Vec<WorkflowV2CallRecord> {
    store
        .load_call_records()
        .unwrap_or_default()
        .into_iter()
        .filter(|record| store.in_session(&record.call.id))
        .collect()
}

/// The plan as the pre-acceptance checkpoint's view carries it.
pub fn residual_plan_view(
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    root: Option<&Path>,
) -> Vec<Value> {
    let records = session_records(store);
    let refs: Vec<&WorkflowV2CallRecord> = records.iter().collect();
    plan_from(&refs, universe, root)
        .rounds
        .iter()
        .map(|round| round_view(round, store))
        .collect()
}

pub fn round_view(round: &PlannedRound, store: &WorkflowV2ResultStore) -> Value {
    let attempted = store
        .load_call_record(&done_checkpoint_id(&round.key))
        .ok()
        .flatten()
        .is_some();
    json!({
        "source": "host",
        "key": round.key,
        "kind": round.kind.as_str(),
        "task_ids": round.tasks,
        "expansion_files": round.files,
        "severity": round.severity().as_str(),
        "findings": round.residuals.iter().map(|residual| json!({
            "id": residual.id,
            "severity": residual.severity.as_str(),
            "description": clip(&residual.description, DESCRIPTION_CHARS),
            "recorded_by": residual.recorded_by,
            "recorded_summary": clip(&residual.recorded_summary, DESCRIPTION_CHARS),
            "paths": residual.files,
        })).collect::<Vec<_>>(),
        "unit_key": round.unit_key,
        "refusal": round.refusal,
        "attempted": attempted,
    })
}

fn asks_for_plan(record: &WorkflowV2CallRecord) -> bool {
    record.call.method == WorkflowV2HostMethod::Checkpoint
        && record.call.options.extra.get(RESIDUAL_GAPS_MARKER) == Some(&Value::Bool(true))
}

/// Whether `call` is the pre-acceptance checkpoint asking for the plan.
pub fn is_residual_slot(call: &WorkflowV2HostCall) -> bool {
    call.method == WorkflowV2HostMethod::Checkpoint
        && call.options.extra.get(RESIDUAL_GAPS_MARKER) == Some(&Value::Bool(true))
}

/// `result` with the host's residual plan, for the view of the checkpoint
/// that asked for it; `None` for every other record. The key is the host's
/// alone: one already in the data is dropped.
pub fn with_residual_plan(
    record: &WorkflowV2CallRecord,
    result: &WorkflowV2Result,
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    root: Option<&Path>,
) -> Option<WorkflowV2Result> {
    let carried = result.data.get(RESIDUAL_GAPS_KEY).is_some();
    if !asks_for_plan(record) && !carried {
        return None;
    }
    let mut viewed = result.clone();
    if !viewed.data.is_object() {
        viewed.data = json!({});
    }
    if let Some(data) = viewed.data.as_object_mut() {
        data.remove(RESIDUAL_GAPS_KEY);
    }
    if asks_for_plan(record) {
        viewed.data[RESIDUAL_GAPS_KEY] = Value::Array(residual_plan_view(store, universe, root));
    }
    Some(viewed)
}

//! Batch O2: the residual passes after the third at the final gate -- each
//! pass N's plan over the executed calls before its slot, exactly what its
//! slot's view planned (`residual_later_pass`).

use std::path::Path;

use super::super::super::{WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2ResultStore};
use super::super::{ResidualPlan, later_pass_plan, slot_pass};
use crate::task_universe::WorkflowV2TaskUniverse;

/// The plans of every pass from the fourth that the script reached, in
/// pass order. A call id seen twice keeps its LAST position, as the gate
/// reads the store.
pub(super) fn later_plans(
    calls: &[WorkflowV2HostCall],
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    repository_root: Option<&Path>,
) -> Vec<ResidualPlan> {
    let mut last: Vec<(usize, &WorkflowV2HostCall)> = Vec::new();
    for (at, call) in calls.iter().enumerate() {
        last.retain(|(_, seen)| seen.id != call.id);
        last.push((at, call));
    }
    let mut slots: Vec<(u64, usize)> = last
        .iter()
        .filter_map(|(at, call)| slot_pass(call).filter(|pass| *pass >= 4).map(|p| (p, *at)))
        .collect();
    slots.sort();
    slots.dedup_by_key(|(pass, _)| *pass);
    slots
        .into_iter()
        .map(|(pass, slot)| {
            let before: Vec<WorkflowV2CallRecord> = last
                .iter()
                .filter(|(at, _)| *at < slot)
                .filter_map(|(_, call)| store.load_call_record(&call.id).ok().flatten())
                .filter(|record| record.invalidated_by.is_none())
                .collect();
            let refs: Vec<&WorkflowV2CallRecord> = before.iter().collect();
            later_pass_plan(pass, &refs, store, universe, repository_root)
        })
        .collect()
}

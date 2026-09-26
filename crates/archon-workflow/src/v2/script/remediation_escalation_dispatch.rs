//! The escalated round at dispatch (Issue-107): it stands only on the
//! host's own plan, and the script's view of every answer is built here, by
//! the one function the live host and its tests both call.
//!
//! The executed-plan validator can only see that a call claims to be the
//! escalated round; it cannot see the verdict that bought it. So before an
//! escalated call is answered at all -- run or replayed -- the host finds the
//! refused verdict of the same unit this session answered last, rebuilds the
//! plan from it, and refuses the call unless its owners, blocker files, task
//! ids and targets are exactly what that plan allows. A refused call
//! dispatches nothing and is not recorded: the fix reads as a round that
//! landed nothing, and the unit ends on the refusal it was bought with.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Value, json};

use super::super::resume_drift::remediation_unit;
use super::super::{
    ScriptEnvelopeShape, WorkflowResult, WorkflowV2CallExecution, WorkflowV2CallRecord,
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2Result, WorkflowV2ResultStore,
    WorkflowV2Status, is_reusable_status, remediation_contract, remediation_contract_string,
    result_view_json_shaped,
};
use super::{
    ESCALATION_CONTRACT_KEY, declaring_tasks, escalation_plan, is_escalated_remediation,
    unit_task_ids, with_escalation_plan,
};
use crate::task_universe::WorkflowV2TaskUniverse;

/// The script's view of `record`: the envelope in `shape`, with the host's
/// cross-owner plan when the record is a refused remediation verdict.
pub fn script_view(
    record: &WorkflowV2CallRecord,
    universe: Option<&WorkflowV2TaskUniverse>,
    repository_root: Option<&Path>,
    shape: ScriptEnvelopeShape,
) -> WorkflowResult<String> {
    let planned = with_escalation_plan(&record.call, &record.result, universe, repository_root);
    result_view_json_shaped(planned.as_ref().unwrap_or(&record.result), shape)
}

/// [`script_view`], with the host's re-verification plan (Issue-111) when
/// `record` is a fix that landed nothing on a tree the run moved since the
/// refusal before its round. The live host's `result_view` renders this.
pub fn script_view_in(
    record: &WorkflowV2CallRecord,
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    repository_root: Option<&Path>,
    shape: ScriptEnvelopeShape,
) -> WorkflowResult<String> {
    let planned = with_escalation_plan(&record.call, &record.result, universe, repository_root);
    let base = planned.as_ref().unwrap_or(&record.result);
    let viewed = super::with_reverify_plan(record, base, store, universe, repository_root);
    result_view_json_shaped(viewed.as_ref().unwrap_or(base), shape)
}

/// What the script is handed for a refused escalated call: nothing landed.
pub fn refused_escalation_result(reason: &str) -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Failed,
        summary: reason.to_string(),
        data: json!({ "patch_landed": false, "escalation_refused": reason }),
        ..WorkflowV2Result::default()
    }
}

/// Why the escalated remediation call `execution` may not be answered, or
/// `None` when it is no such call or is exactly what the host's plan allows.
pub fn escalation_refusal(
    execution: &WorkflowV2CallExecution,
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    repository_root: Option<&Path>,
) -> Option<String> {
    let call = &execution.call;
    let contract = remediation_contract(call)?;
    let escalation = contract.get(ESCALATION_CONTRACT_KEY)?;
    refusal(
        execution,
        store,
        universe,
        repository_root,
        contract,
        escalation,
    )
    .map(|why| format!("escalated remediation `{}` refused: {why}", call.id))
}

fn refusal(
    execution: &WorkflowV2CallExecution,
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
    root: Option<&Path>,
    contract: &Value,
    escalation: &Value,
) -> Option<String> {
    let Some(universe) = universe else {
        return Some("there is no task universe to check it against".into());
    };
    let Some((unit, round)) = remediation_unit(&execution.call) else {
        return Some("its contract names no unit and round".into());
    };
    let records = match store.load_call_records() {
        Ok(records) => records,
        Err(error) => return Some(format!("the call records are unreadable: {error}")),
    };
    let Some(refused) = last_refusal(&records, store, &unit, round) else {
        return Some("no refused verdict of its unit was answered in this session".into());
    };
    let Some(plan) = escalation_plan(&refused.call, &refused.result, Some(universe), root) else {
        return Some(format!(
            "the refusal `{}` carries no host plan",
            refused.call.id
        ));
    };
    let owners = set(&plan["owner_task_ids"]);
    let files = set(&plan["target_files"]);
    let universe_ids: BTreeSet<String> = universe
        .tasks
        .iter()
        .map(|task| task.canonical_task_id.clone())
        .collect();
    if !owners.is_subset(&universe_ids) {
        return Some("its owners are not all universe tasks".into());
    }
    if set(&escalation["ownerTaskIds"]) != owners || set(&escalation["blockerPaths"]) != files {
        return Some(format!(
            "its contract does not match the plan of `{}` (owners {owners:?}, files {files:?})",
            refused.call.id
        ));
    }
    // Issue-111: the no-patch checkpoint of an escalated round dispatches
    // nothing and carries no item; its contract matching the plan is all
    // there is to check. Refused, it went unrecorded, so the escalated fix
    // stood in the executed plan with no verify stage after it.
    if execution.call.method == WorkflowV2HostMethod::Checkpoint {
        return None;
    }
    let unit_tasks = unit_task_ids(contract);
    let item = &execution.input["source_data"][0];
    let expected: BTreeSet<String> = unit_tasks.union(&owners).cloned().collect();
    if set(&item["canonical_task_ids"]) != expected {
        return Some(format!("its tasks are not exactly {expected:?}"));
    }
    if execution.call.write_mode.is_none() {
        return None;
    }
    if set(&item["escalation_owner_task_ids"]) != owners
        || set(&item["escalation_blocker_paths"]) != files
    {
        return Some("its item does not name the plan's owners and blocker files".into());
    }
    let targets = set(&item["target_files"]);
    if !files.is_subset(&targets) {
        return Some("its targets omit a blocker file".into());
    }
    targets.difference(&files).find_map(|target| {
        let owners = declaring_tasks(universe, target, root);
        (!owners.is_empty() && owners.is_disjoint(&unit_tasks))
            .then(|| format!("its target {target} belongs only to {owners:?}, outside the plan"))
    })
}

/// The refused verdict of `unit` before `round` this session answered last:
/// the one the script's escalation was decided on.
pub(super) fn last_refusal<'a>(
    records: &'a [WorkflowV2CallRecord],
    store: &WorkflowV2ResultStore,
    unit: &str,
    round: u64,
) -> Option<&'a WorkflowV2CallRecord> {
    records
        .iter()
        .filter(|record| {
            is_verdict(&record.call)
                && !is_reusable_status(record.status)
                && store.in_session(&record.call.id)
        })
        .filter_map(|record| {
            let (key, at) = remediation_unit(&record.call)?;
            (key == unit && at < round).then_some((at, finished(record), record))
        })
        .max_by(|left, right| (left.0, left.1).cmp(&(right.0, right.1)))
        .map(|(_, _, record)| record)
}

fn is_verdict(call: &WorkflowV2HostCall) -> bool {
    remediation_contract_string(call, "stage") == Some("verify")
        && call.method != WorkflowV2HostMethod::Checkpoint
        && !is_escalated_remediation(call)
}

pub(super) fn finished(record: &WorkflowV2CallRecord) -> i64 {
    chrono::DateTime::parse_from_rfc3339(&record.finished_at)
        .map(|at| at.timestamp_nanos_opt().unwrap_or(i64::MIN))
        .unwrap_or(i64::MIN)
}

fn set(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

//! REM-14 test support: the task stage a remediation-only test does not run,
//! RECORDED.
//!
//! The terminal rule proves every universe task accepted from the host's
//! records. A test that drives review or acceptance remediation alone starts
//! after the task stage, so its host holds no write or verify for any task;
//! judged as it is, every universe task would hold the run for having never
//! been implemented.
//!
//! Batch O2 (m6): the stage is no longer a list of invented facts handed to
//! the rule. It is recorded in the run's store as the live host records it
//! -- one accepted write fanout and one later accepted verification wave,
//! each dispatching every task -- before every record the test's own run
//! made, and listed first; the rule's facts are then built from those
//! records by the production `authored_call_facts`, exactly as the live
//! host builds them.
#![allow(dead_code)]
use std::collections::BTreeSet;

use archon_workflow::v2::WorkflowV2DispatchedItem;
use archon_workflow::*;
use serde_json::json;

const WRITE: &str = "task-stage-write";
const VERIFY: &str = "verification-wave-task-stage";

/// One accepted stage call over `tasks`, recorded at `at` unless the store
/// already holds it.
fn record(
    store: &WorkflowV2ResultStore,
    id: &str,
    write: bool,
    tasks: &BTreeSet<String>,
    at: &str,
) -> WorkflowV2HostCall {
    let call = WorkflowV2HostCall {
        id: id.into(),
        method: if write {
            WorkflowV2HostMethod::Fanout
        } else {
            WorkflowV2HostMethod::Parallel
        },
        write_mode: write.then_some(WorkflowV2WriteMode::Worktree),
        options: Default::default(),
    };
    if store.load_call_record(id).unwrap().is_some() {
        return call;
    }
    let items: Vec<WorkflowV2DispatchedItem> = tasks
        .iter()
        .map(|task| WorkflowV2DispatchedItem {
            item_id: format!("{id}-{task}"),
            canonical_task_ids: vec![task.clone()],
        })
        .collect();
    let mut result = WorkflowV2Result::accepted("task stage");
    result.data = json!({"outcomes": items.iter().map(|item| json!({
        "item_id": item.item_id, "status": "accepted",
        "canonical_task_ids": item.canonical_task_ids,
        "result": {"status": "accepted"}})).collect::<Vec<_>>()});
    let mut record = WorkflowV2CallRecord::new("run", call.clone(), 1, "h".into(), result, vec![])
        .with_dispatched_items(items);
    record.started_at = at.to_string();
    record.finished_at = at.to_string();
    store.save_call_record(&record).unwrap();
    call
}

/// `calls`, after the task stage recorded in `store`: a write, then a
/// verify, of every task in `tasks`, both before any record the run made.
pub fn with_task_stage(
    store: &WorkflowV2ResultStore,
    tasks: &BTreeSet<String>,
    calls: &[WorkflowV2HostCall],
) -> Vec<WorkflowV2HostCall> {
    // Before every record of the run: one second, then two, before its
    // earliest start.
    let earliest = store
        .load_call_records()
        .unwrap()
        .iter()
        .filter(|record| record.call.id != WRITE && record.call.id != VERIFY)
        .filter_map(|record| chrono::DateTime::parse_from_rfc3339(&record.started_at).ok())
        .min()
        .unwrap_or_else(|| chrono::Utc::now().fixed_offset());
    let at = |seconds: i64| (earliest - chrono::Duration::seconds(seconds)).to_rfc3339();
    let mut staged = vec![
        record(store, WRITE, true, tasks, &at(2)),
        record(store, VERIFY, false, tasks, &at(1)),
    ];
    staged.extend(calls.iter().cloned());
    staged
}

/// `accounting` (JSON text) naming every task in `tasks` the stage accepted
/// -- every one it does not already report blocked -- as accepted.
pub fn named(tasks: &BTreeSet<String>, accounting: &str) -> String {
    let mut value: serde_json::Value = serde_json::from_str(accounting).unwrap();
    let blocked: BTreeSet<String> = value["blocked"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry["taskId"].as_str().map(str::to_string))
        .collect();
    let mut accepted: Vec<serde_json::Value> =
        value["accepted"].as_array().cloned().unwrap_or_default();
    for task in tasks.difference(&blocked) {
        if !accepted.iter().any(|named| named == task) {
            accepted.push(serde_json::Value::from(task.clone()));
        }
    }
    value["accepted"] = serde_json::Value::Array(accepted);
    value.to_string()
}

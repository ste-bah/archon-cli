//! REM-14: the task set completed by the host, not by the script's memory.
//!
//! An authored script decides which universe tasks it dispatches. One it
//! skips -- never written, so neither accepted nor blocked -- used to be
//! caught only as an accounting divergence that failed the run, and nothing
//! implemented it. Now, before the first review, the prelude asks the host
//! through a checkpoint carrying [`TASK_COMPLETION_MARKER`] which universe
//! tasks no write of THIS session named. The host answers on that
//! checkpoint's view, under [`TASK_COMPLETION_KEY`], computed from its own
//! call records at the moment of asking (never persisted): one entry per such
//! task, in dependency-wave order (`author_wave_groups`, so a skipped
//! task's skipped dependency is completed first), with the unit id the prelude files its calls
//! under and what the task declares (its task file, the files it may write,
//! its focused tests, its deliverable artifacts). The prelude then runs each
//! as a task: an implementation write, a verification, retries on the
//! progress-following budget, and the review of every task it completes.
//!
//! The completion units' own calls never count as "a write named it", so a
//! resumed session asks the same question, gets the same plan and replays
//! the same calls. Nothing here decides a task: the terminal rule reads the
//! units' write and verify records like any task's.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::{
    AuthoredCallRole, WorkflowV2CallRecord, WorkflowV2HostMethod, WorkflowV2Result,
    WorkflowV2ResultStore, authored_call_role, declared_path, task_outcomes,
};
use crate::task_universe::WorkflowV2TaskUniverse;

/// The checkpoint option that asks for the completion plan.
pub const TASK_COMPLETION_MARKER: &str = "taskCompletion";
/// Key of the plan in that checkpoint's view.
pub const TASK_COMPLETION_KEY: &str = "task_completion";
/// Prefix of every completion unit id (its write is `<unit>-impl-<n>`, its
/// verifier `verification-wave-<unit>-verify-<n>`). Reserved: an authored
/// `agent()`/`agents()` id is a slug, which never begins with `_`, so no
/// call the script makes can pass for a host completion unit's.
pub const COMPLETION_UNIT_PREFIX: &str = "__host-complete-";
/// A unit that writes: the task declares files it may change.
pub const COMPLETION_MODE_WRITE: &str = "write";
/// A unit for a task that declares no file it may write: one read-only
/// verification, which the host accepts only as a recorded no-op.
pub const COMPLETION_MODE_NOOP_VERIFY: &str = "noop_verify";

fn slug(text: &str) -> String {
    let lowered: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let parts: Vec<&str> = lowered.split('-').filter(|part| !part.is_empty()).collect();
    parts.join("-").chars().take(40).collect()
}

fn fnv(text: &str) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in text.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:08x}")
}

/// The unit id the host plans for completing `task`: readable, and unique
/// per task id whatever its slug shares with another's.
pub fn completion_unit(task: &str) -> String {
    format!("{COMPLETION_UNIT_PREFIX}{}-{}", slug(task), fnv(task))
}

/// Whether `call_id` is one of a completion unit's own calls.
pub fn is_completion_call(call_id: &str) -> bool {
    let id = call_id
        .strip_prefix("verification-wave-")
        .unwrap_or(call_id);
    id.starts_with(COMPLETION_UNIT_PREFIX)
}

/// The universe tasks (canonical spelling) some write this session recorded
/// or replayed names, completion units excluded.
fn written_in_session(
    records: &[WorkflowV2CallRecord],
    store: &WorkflowV2ResultStore,
    universe: &WorkflowV2TaskUniverse,
) -> BTreeSet<String> {
    let mut written = BTreeSet::new();
    for record in records {
        if record.invalidated_by.is_some()
            || is_completion_call(&record.call.id)
            || !store.in_session(&record.call.id)
            || authored_call_role(&record.call) != AuthoredCallRole::Write
        {
            continue;
        }
        for named in task_outcomes(record, false).0.keys() {
            if let Some(task) = universe
                .tasks
                .iter()
                .find(|task| task.canonical_task_id.eq_ignore_ascii_case(named.trim()))
            {
                written.insert(task.canonical_task_id.clone());
            }
        }
    }
    written
}

/// One entry per universe task no write of this session named.
pub fn completion_plan(
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
) -> Vec<Value> {
    let Some(universe) = universe else {
        return Vec::new();
    };
    let records = match store.load_call_records() {
        Ok(records) => records,
        // An unreadable store plans nothing; the terminal rule still names
        // every task without an accepted outcome.
        Err(_) => return Vec::new(),
    };
    let written = written_in_session(&records, store, universe);
    let paths = |entries: &[String]| -> Vec<String> {
        entries.iter().filter_map(|e| declared_path(e)).collect()
    };
    // Dependency waves first, so a unit finds what it builds on done.
    let order: Vec<String> = super::author_wave_groups(universe)
        .into_iter()
        .flat_map(|group| group.task_ids)
        .collect();
    let mut tasks: Vec<_> = universe.tasks.iter().collect();
    tasks.sort_by_key(|task| {
        order
            .iter()
            .position(|id| *id == task.canonical_task_id)
            .unwrap_or(usize::MAX)
    });
    tasks
        .into_iter()
        .filter(|task| !written.contains(&task.canonical_task_id))
        .map(|task| {
            let mut targets = paths(&task.files_expected_to_change);
            for path in paths(&task.shared_append_target_files) {
                if !targets.contains(&path) {
                    targets.push(path);
                }
            }
            let artifacts: Vec<&str> = task
                .deliverable_contracts
                .iter()
                .map(|contract| contract.artifact_path.as_str())
                .filter(|path| !path.trim().is_empty())
                .collect();
            json!({
                "source": "host",
                "task_id": task.canonical_task_id,
                "unit": completion_unit(&task.canonical_task_id),
                "task_file": task.source_path,
                "target_files": targets,
                "focused_tests": task.focused_tests,
                "artifacts": artifacts,
                "mode": if targets.is_empty() {
                    COMPLETION_MODE_NOOP_VERIFY
                } else {
                    COMPLETION_MODE_WRITE
                },
            })
        })
        .collect()
}

fn asks_for_plan(record: &WorkflowV2CallRecord) -> bool {
    record.call.method == WorkflowV2HostMethod::Checkpoint
        && record.call.options.extra.get(TASK_COMPLETION_MARKER) == Some(&Value::Bool(true))
}

/// `result` with the host's completion plan, for the view of the checkpoint
/// that asked; `None` for every other record. The key is the host's alone.
pub fn with_task_completion(
    record: &WorkflowV2CallRecord,
    result: &WorkflowV2Result,
    store: &WorkflowV2ResultStore,
    universe: Option<&WorkflowV2TaskUniverse>,
) -> Option<WorkflowV2Result> {
    let asks = asks_for_plan(record);
    if !asks && result.data.get(TASK_COMPLETION_KEY).is_none() {
        return None;
    }
    let mut viewed = result.clone();
    if !viewed.data.is_object() {
        viewed.data = json!({});
    }
    if let Some(data) = viewed.data.as_object_mut() {
        data.remove(TASK_COMPLETION_KEY);
    }
    if asks {
        viewed.data[TASK_COMPLETION_KEY] = Value::Array(completion_plan(store, universe));
    }
    Some(viewed)
}

#[cfg(test)]
#[path = "v3_run_facts_completion_tests.rs"]
mod tests;

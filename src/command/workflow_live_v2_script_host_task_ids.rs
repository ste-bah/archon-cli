//! Task ids a stored call record speaks for, split out of
//! `workflow_live_v2_script_host_exec.rs` to hold the 500-line ceiling.

use super::*;

/// Canonical task ids a stored record speaks for.
///
/// Three sources, unioned, because no single one is populated for every call
/// kind: wave records carry `completed_ids`/`completion_evidence`, while a v3
/// `implement-task-*`/`remediate-task-*` record carries no task-id evidence at
/// all and names its task only in the call id.

pub(super) fn record_task_ids(
    record: &WorkflowV2CallRecord,
    universe: Option<&WorkflowV2TaskUniverse>,
) -> std::collections::BTreeSet<String> {
    let mut tasks = record
        .completed_ids
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    for evidence in &record.completion_evidence {
        let task_id = evidence.task_id.trim();
        if !task_id.is_empty() {
            tasks.insert(task_id.to_string());
        }
    }
    if let Some(universe) = universe {
        let call_id = record.call.id.to_ascii_lowercase();
        for task in &universe.tasks {
            if call_id_names_task(&call_id, &task.canonical_task_id.to_ascii_lowercase()) {
                tasks.insert(task.canonical_task_id.clone());
            }
        }
    }
    tasks
}

/// Whether a lowercased call id embeds a lowercased canonical task id as a
/// whole token. A bare `contains` would let a shorter id (`TASK-01`) match the
/// call id of a longer one (`TASK-010`) and taint an unrelated task, so the
/// match must not be followed by another alphanumeric character.
pub(super) fn call_id_names_task(call_id_lower: &str, task_id_lower: &str) -> bool {
    if task_id_lower.is_empty() {
        return false;
    }
    call_id_lower
        .match_indices(task_id_lower)
        .any(|(start, _)| {
            call_id_lower[start + task_id_lower.len()..]
                .chars()
                .next()
                .is_none_or(|next| !next.is_ascii_alphanumeric())
        })
}

impl WorkflowScriptHost {
    /// Record that a call just RE-EXECUTED, so every task it speaks for — and
    /// everything downstream of those tasks in the authoritative task universe —
    /// can no longer be served from cache by a reuse path that cannot key on the
    /// input hash. Nothing else fires an invalidation mid-run: the store's
    /// `invalidate_*` routines are reachable only from `workflow restart`.
    pub(super) fn mark_tasks_reexecuted(&self, record: &WorkflowV2CallRecord) {
        let Some(universe) = self.runner.task_universe.as_ref() else {
            // No task universe means no dependency graph — and also no
            // `resume_completed_ids`, so the hash-free reuse paths are inert.
            return;
        };
        let touched = record_task_ids(record, Some(universe));
        if touched.is_empty() {
            return;
        }
        let closure = touched
            .iter()
            .flat_map(|task_id| universe.downstream_task_closure(task_id))
            .collect::<Vec<_>>();
        if let Ok(mut dirty) = self.runner.reexecuted_task_closure.lock() {
            dirty.extend(closure);
        }
    }

    /// Whether reusing `record` WITHOUT an input-hash match would replay a
    /// result whose inputs have already moved under it in this run.
    pub(super) fn hash_free_reuse_stale(&self, record: &WorkflowV2CallRecord) -> bool {
        let Ok(dirty) = self.runner.reexecuted_task_closure.lock() else {
            // A poisoned lock means we cannot prove freshness; fail closed onto
            // the content-keyed paths rather than replay blind.
            return true;
        };
        if dirty.is_empty() {
            return false;
        }
        record_task_ids(record, self.runner.task_universe.as_ref())
            .iter()
            .any(|task_id| dirty.contains(task_id))
    }
}

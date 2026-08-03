//! The task files' declared `status:`, as the scheduler sees it.
//!
//! [`super::LifecycleContract`] is the one place that already holds both the
//! task universe and the run's completed set, so it is where "is this task
//! done?" gets answered. The classification itself lives in
//! [`crate::command::workflow_live::workflow_live_task_universe::task_status`],
//! which carries the full table of what each value causes.

use std::collections::BTreeSet;

use serde_json::Value;

use super::LifecycleContract;
use crate::command::workflow_live::workflow_live_task_universe::WorkflowV2TaskUniverseTask;

impl LifecycleContract<'_> {
    /// Whether the task universe declares this canonical task already finished.
    ///
    /// `done` means done on a resume as well as on a fresh run; nothing else in
    /// the table implies completion, `in_review` least of all.
    pub(in super::super) fn declared_complete(&self, canonical_task_id: &str) -> bool {
        self.task_universe.tasks.iter().any(|task| {
            task.canonical_task_id == canonical_task_id && task.declared_status_is_complete()
        })
    }

    /// A task is complete when this run completed it, or its file declares it
    /// was already complete before the run started.
    pub(in super::super) fn task_is_complete(
        &self,
        canonical_task_id: &str,
        completed: &BTreeSet<String>,
    ) -> bool {
        completed.contains(canonical_task_id) || self.declared_complete(canonical_task_id)
    }

    /// What the task files claimed about their own status, for the run record.
    ///
    /// A `done` declaration removes work from the schedule on the strength of a
    /// line in a markdown file, and a `blocked` one is the author's account of
    /// why the task is not eligible yet. Neither is applied silently: both are
    /// written into the run's evidence with the file that made the claim, so a
    /// reader can go and check it. `None` when no task declared either.
    pub(in super::super) fn declared_status_notice(&self) -> Option<Value> {
        let complete =
            self.declared_entries(WorkflowV2TaskUniverseTask::declared_status_is_complete);
        let blocked = self.declared_entries(WorkflowV2TaskUniverseTask::declared_status_is_blocked);
        if complete.is_empty() && blocked.is_empty() {
            return None;
        }
        Some(serde_json::json!({
            "kind": "declared-task-status",
            "declared_complete_not_scheduled": complete,
            "declared_blocked_behind_dependencies": blocked,
        }))
    }

    fn declared_entries(&self, matches: fn(&WorkflowV2TaskUniverseTask) -> bool) -> Vec<Value> {
        self.task_universe
            .tasks
            .iter()
            .filter(|task| matches(task))
            .map(|task| {
                serde_json::json!({
                    "canonical_task_id": task.canonical_task_id,
                    "source_path": task.source_path,
                })
            })
            .collect()
    }
}

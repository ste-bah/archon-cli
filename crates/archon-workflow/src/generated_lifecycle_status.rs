//! The task files' declared `status:`, as the scheduler sees it.
//!
//! [`super::LifecycleContract`] is the one place that already holds both the
//! task universe and the run's completed set, so it is where "is this task
//! done?" gets answered. The classification itself lives in
//! [`crate::task_universe::task_status`],
//! which carries the full table of what each value causes.

use std::collections::BTreeSet;

use serde_json::Value;

use super::LifecycleContract;
use crate::task_universe::WorkflowV2TaskUniverseTask;

impl LifecycleContract<'_> {
    /// Whether the task universe declares this canonical task already finished.
    ///
    /// Completion-group declarations protect tasks on a resume and a fresh run.
    pub(super) fn declared_complete(&self, canonical_task_id: &str) -> bool {
        self.task_universe.tasks.iter().any(|task| {
            task.canonical_task_id == canonical_task_id && task.declared_status_is_complete()
        })
    }

    /// A task is complete when this run completed it, or its file declares it
    /// was already complete before the run started.
    pub(super) fn task_is_complete(
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
    pub fn declared_status_notice(&self) -> Option<Value> {
        let complete =
            self.declared_entries(WorkflowV2TaskUniverseTask::declared_status_is_complete);
        let blocked = self.declared_entries(WorkflowV2TaskUniverseTask::declared_status_is_blocked);
        let states = frozen_states(self.task_universe);
        if complete.is_empty() && blocked.is_empty() && states.is_none() {
            return None;
        }
        let mut notice = serde_json::json!({
            "kind": "declared-task-status",
            "declared_complete_not_scheduled": complete,
            "declared_blocked_behind_dependencies": blocked,
        });
        if let Some(states) = states {
            notice["frozen_requirement_states"] = states;
        }
        Some(notice)
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

fn frozen_states(universe: &crate::task_universe::WorkflowV2TaskUniverse) -> Option<Value> {
    let mut done = Vec::new();
    let mut undecided = Vec::new();
    let mut inspection = Vec::new();
    let mut kept = Vec::new();
    let mut present = false;
    let mut skeletons = std::collections::BTreeMap::new();
    for task in &universe.tasks {
        if task.status.as_deref() == Some("kept") {
            kept.push(task.canonical_task_id.clone());
        }
        let root = std::path::Path::new(&task.source_path).parent();
        let skeleton = skeletons.entry(root).or_insert_with(|| {
            root.and_then(|p| {
                std::fs::read(p.join(crate::task_set_contract::TASK_SKELETON_FILE)).ok()
            })
            .and_then(|b| serde_json::from_slice::<crate::task_skeleton::TaskSkeleton>(&b).ok())
        });
        for (id, state) in &task.requirement_states {
            present = true;
            let reason = skeleton
                .as_ref()
                .and_then(|s| s.tasks.iter().find(|t| t.task_id == task.canonical_task_id))
                .and_then(|t| t.requirement_states.get(id))
                .map(|d| d.reason.as_str())
                .unwrap_or("record_unavailable");
            let entry = serde_json::json!({"task":task.canonical_task_id,"id":id,"reason":reason});
            match state.as_str() {
                "done" => done.push(entry),
                "undecided" => undecided.push(entry),
                "inspection" => inspection.push(entry),
                _ => {}
            }
        }
    }
    present.then(|| serde_json::json!({"done":done,"undecided":undecided,"inspection":inspection,"kept":kept}))
}

//! Runtime-aligned graph-shape validation for portable task skeletons.

use std::collections::BTreeSet;

use crate::task_skeleton::{TaskSetFinding, TaskSkeleton};
use crate::task_universe::WorkflowV2TaskUniverseTask;

pub(super) fn graph_shape_finding(skeleton: &TaskSkeleton) -> Option<TaskSetFinding> {
    let ids = skeleton
        .tasks
        .iter()
        .map(|task| task.task_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut tasks = skeleton
        .tasks
        .iter()
        .map(|task| WorkflowV2TaskUniverseTask {
            canonical_task_id: task.task_id.clone(),
            source_path: task.file_name.clone(),
            dependency_ids: task
                .depends_on
                .iter()
                .filter(|dependency| ids.contains(dependency.task_id.as_str()))
                .map(|dependency| dependency.task_id.clone())
                .collect(),
            dependencies: task
                .depends_on
                .iter()
                .filter(|dependency| ids.contains(dependency.task_id.as_str()))
                .cloned()
                .collect(),
            blocks_ids: task
                .blocks
                .iter()
                .filter(|blocked| ids.contains(blocked.as_str()))
                .cloned()
                .collect(),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    if let Err(error) = crate::task_universe::reconcile_blocks_into_dependencies(&mut tasks) {
        return Some(TaskSetFinding {
            field: "dependency graph".into(),
            message: format!(
                "{error}; keep one direction for the named pair by removing the contradictory depends_on or blocks declaration"
            ),
        });
    }
    crate::task_universe::validate_task_dependency_graph(&tasks)
        .err()
        .map(|error| TaskSetFinding {
            field: "dependency graph".into(),
            message: format!(
                "{error}; remove or reverse at least one named depends_on/blocks edge before freezing"
            ),
        })
}

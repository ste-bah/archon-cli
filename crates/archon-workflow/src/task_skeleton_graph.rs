//! Complete graph-shape diagnostics for portable task skeletons.
use crate::defect::DeterministicDefect;
use crate::task_skeleton::{TaskSetFinding, TaskSkeleton};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn graph_shape_findings(skeleton: &TaskSkeleton) -> Vec<TaskSetFinding> {
    let tasks: BTreeMap<_, _> = skeleton
        .tasks
        .iter()
        .map(|task| (task.task_id.as_str(), task))
        .collect();
    let mut graph: BTreeMap<&str, BTreeSet<&str>> =
        tasks.keys().map(|id| (*id, BTreeSet::new())).collect();
    let mut findings = Vec::new();
    for (slot, task) in skeleton.tasks.iter().enumerate() {
        for dependency in &task.depends_on {
            if tasks.contains_key(dependency.task_id.as_str()) {
                graph
                    .entry(&task.task_id)
                    .or_default()
                    .insert(&dependency.task_id);
            }
        }
        for (index, blocked) in task.blocks.iter().enumerate() {
            if !tasks.contains_key(blocked.as_str()) {
                continue;
            }
            graph.entry(blocked).or_default().insert(&task.task_id);
            let defect = if blocked == &task.task_id {
                Some((
                    "self_block",
                    format!("task {} declares that it blocks itself", task.task_id),
                ))
            } else if task.depends_on.iter().any(|dep| &dep.task_id == blocked) {
                Some((
                    "contradictory_edge",
                    format!("task {} both blocks and depends_on {blocked}", task.task_id),
                ))
            } else if tasks
                .get(blocked.as_str())
                .is_some_and(|other| other.blocks.contains(&task.task_id))
            {
                Some((
                    "mutual_blocks",
                    format!(
                        "tasks {} and {blocked} each declare that they block the other",
                        task.task_id
                    ),
                ))
            } else {
                None
            };
            if let Some((code, message)) = defect {
                findings.push(TaskSetFinding {
                    identity: DeterministicDefect::new(code, super::skeleton_subject(&task.task_id, slot), format!("blocks/{index}")),
                    field: "dependency graph".into(),
                    message: format!("{message} in {}; keep one direction for the named pair by removing the contradictory depends_on or blocks declaration", task.file_name),
                });
            }
        }
    }
    // Each cyclic node is one stable defect. Traversals are iterative so an
    // authored graph cannot exhaust the Rust stack, and disjoint cycles are
    // all reported instead of being hidden behind the first DFS error.
    for start in graph.keys() {
        let mut pending: Vec<_> = graph.get(start).into_iter().flatten().copied().collect();
        let mut visited = BTreeSet::new();
        while let Some(node) = pending.pop() {
            if node == *start {
                findings.push(TaskSetFinding {
                    identity: DeterministicDefect::new("dependency_cycle", super::skeleton_subject(start, skeleton.tasks.iter().position(|task| task.task_id == *start).unwrap_or_default()), "dependency graph"),
                    field: "dependency graph".into(),
                    message: format!("task dependency cycle detected through {start} ({}); remove or reverse at least one named depends_on/blocks edge before freezing", tasks.get(start).map(|task| task.file_name.as_str()).unwrap_or("")),
                });
                break;
            }
            if visited.insert(node) {
                pending.extend(graph.get(node).into_iter().flatten().copied());
            }
        }
    }
    findings
}

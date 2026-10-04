//! Complete graph-shape diagnostics for portable task skeletons.
//!
//! A *contradiction* -- a task blocking itself, a pair that both blocks and
//! depends_on, or two tasks each blocking the other -- is the authoring
//! mistake. Folding it into the graph manufactures a cycle that names a graph
//! shape instead of the mistake, so, as the runtime's
//! `reconcile_blocks_into_dependencies` intends, a contradiction is named
//! alone for its pair: once, and without the cycle its own edges manufacture.
//! Every other cycle in the graph is still reported: each task on one is a
//! stable defect whose message names the cycle.
use crate::defect::DeterministicDefect;
use crate::task_skeleton::{TaskSetFinding, TaskSkeleton};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const CONTRADICTION_REMEDY: &str = "keep one direction for the named pair by removing the contradictory depends_on or blocks declaration";

pub(super) fn graph_shape_findings(skeleton: &TaskSkeleton) -> Vec<TaskSetFinding> {
    let tasks: BTreeMap<_, _> = skeleton
        .tasks
        .iter()
        .map(|task| (task.task_id.as_str(), task))
        .collect();
    let (mut findings, pairs) = contradiction_findings(skeleton, &tasks);
    let contradictory = |a: &str, b: &str| pairs.contains(&ordered(a, b));
    let mut graph: BTreeMap<&str, BTreeSet<&str>> =
        tasks.keys().map(|id| (*id, BTreeSet::new())).collect();
    for task in &skeleton.tasks {
        for dependency in &task.depends_on {
            if tasks.contains_key(dependency.task_id.as_str())
                && !contradictory(&task.task_id, &dependency.task_id)
            {
                graph
                    .entry(&task.task_id)
                    .or_default()
                    .insert(&dependency.task_id);
            }
        }
        for blocked in &task.blocks {
            if tasks.contains_key(blocked.as_str()) && !contradictory(&task.task_id, blocked) {
                graph.entry(blocked).or_default().insert(&task.task_id);
            }
        }
    }
    for start in graph.keys() {
        let Some(path) = cycle_through(&graph, start) else {
            continue;
        };
        let described = path
            .iter()
            .map(|id| match tasks.get(id) {
                Some(task) => format!("{id} ({})", task.file_name),
                None => (*id).to_string(),
            })
            .collect::<Vec<_>>()
            .join(" -> ");
        let slot = skeleton
            .tasks
            .iter()
            .position(|task| task.task_id == *start)
            .unwrap_or_default();
        findings.push(TaskSetFinding {
            identity: DeterministicDefect::new(
                "dependency_cycle",
                super::skeleton_subject(start, slot),
                "dependency graph",
            ),
            field: "dependency graph".into(),
            message: format!(
                "task dependency cycle detected through {start}: {described}; remove or reverse at least one named depends_on/blocks edge before freezing"
            ),
        });
    }
    findings
}

fn ordered<'a>(a: &'a str, b: &'a str) -> (&'a str, &'a str) {
    if a <= b { (a, b) } else { (b, a) }
}

/// The contradictions, and the unordered pairs whose edges they involve.
fn contradiction_findings<'a>(
    skeleton: &'a TaskSkeleton,
    tasks: &BTreeMap<&str, &crate::task_skeleton::FrozenTask>,
) -> (Vec<TaskSetFinding>, BTreeSet<(&'a str, &'a str)>) {
    let mut findings = Vec::new();
    let mut pairs = BTreeSet::new();
    for (slot, task) in skeleton.tasks.iter().enumerate() {
        for (index, blocked) in task.blocks.iter().enumerate() {
            let Some(other) = tasks.get(blocked.as_str()) else {
                continue;
            };
            let id = &task.task_id;
            let defect = if blocked == id {
                Some((
                    "self_block",
                    format!(
                        "task {id} declares that it blocks itself in {}",
                        task.file_name
                    ),
                ))
            } else if task.depends_on.iter().any(|dep| &dep.task_id == blocked) {
                Some((
                    "contradictory_edge",
                    format!(
                        "task {id} both blocks and depends_on {blocked} in {}",
                        task.file_name
                    ),
                ))
            } else if other.blocks.contains(id) && id < blocked {
                // One finding per pair, named by its first task.
                Some((
                    "mutual_blocks",
                    format!(
                        "tasks {id} and {blocked} each declare that they block the other ({} / {})",
                        task.file_name, other.file_name
                    ),
                ))
            } else {
                None
            };
            if let Some((code, message)) = defect {
                pairs.insert(ordered(id, blocked));
                findings.push(TaskSetFinding {
                    identity: DeterministicDefect::new(
                        code,
                        super::skeleton_subject(id, slot),
                        format!("blocks/{index}"),
                    ),
                    field: "dependency graph".into(),
                    message: format!("{message}; {CONTRADICTION_REMEDY}"),
                });
            }
        }
    }
    (findings, pairs)
}

/// The shortest cycle from `start` back to itself, as task ids beginning and
/// ending with `start`. Breadth-first and iterative, so an authored graph
/// cannot exhaust the stack.
fn cycle_through<'a>(
    graph: &BTreeMap<&'a str, BTreeSet<&'a str>>,
    start: &'a str,
) -> Option<Vec<&'a str>> {
    let mut parent: BTreeMap<&str, &str> = BTreeMap::new();
    let mut pending = VecDeque::from([start]);
    while let Some(node) = pending.pop_front() {
        for next in graph.get(node).into_iter().flatten().copied() {
            if next == start {
                let mut path = vec![node];
                let mut cursor = node;
                while cursor != start {
                    cursor = parent.get(cursor).copied()?;
                    path.push(cursor);
                }
                path.reverse();
                path.push(start);
                return Some(path);
            }
            if !parent.contains_key(next) {
                parent.insert(next, node);
                pending.push_back(next);
            }
        }
    }
    None
}

use super::events::Subtask;
use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::HashMap;

/// Build execution waves from a list of subtasks with dependencies.
/// Returns a Vec of waves; tasks in the same wave can execute concurrently.
/// Errors on cycles or unknown dependency IDs.
pub fn build_dag_waves(subtasks: &[Subtask]) -> anyhow::Result<Vec<Vec<String>>> {
    let mut graph: DiGraph<String, ()> = DiGraph::new();
    let mut id_to_node: HashMap<String, NodeIndex> = HashMap::new();

    for task in subtasks {
        let node = graph.add_node(task.id.clone());
        id_to_node.insert(task.id.clone(), node);
    }

    for task in subtasks {
        let to = id_to_node[&task.id];
        for dep in &task.dependencies {
            let from = id_to_node.get(dep).ok_or_else(|| {
                anyhow::anyhow!("subtask '{}' depends on unknown subtask '{}'", task.id, dep)
            })?;
            graph.add_edge(*from, to, ());
        }
    }

    toposort(&graph, None)
        .map_err(|_| anyhow::anyhow!("dependency cycle detected in subtask graph"))?;

    // Compute depth (wave number) for each node via recursive memoization from roots
    let mut depth: HashMap<String, usize> = HashMap::new();
    for task in subtasks {
        compute_depth(task, subtasks, &mut depth);
    }

    let max_depth = depth.values().copied().max().unwrap_or(0);
    let mut waves: Vec<Vec<String>> = vec![Vec::new(); max_depth + 1];
    for task in subtasks {
        let d = depth[&task.id];
        waves[d].push(task.id.clone());
    }
    waves.retain(|w| !w.is_empty());

    Ok(waves)
}

fn compute_depth(task: &Subtask, all: &[Subtask], memo: &mut HashMap<String, usize>) {
    if memo.contains_key(&task.id) {
        return;
    }
    if task.dependencies.is_empty() {
        memo.insert(task.id.clone(), 0);
        return;
    }
    let max_dep = task
        .dependencies
        .iter()
        .map(|dep_id| {
            let dep = all.iter().find(|t| &t.id == dep_id);
            if let Some(dep) = dep {
                compute_depth(dep, all, memo);
                memo.get(dep_id).copied().unwrap_or(0)
            } else {
                0
            }
        })
        .max()
        .unwrap_or(0);
    memo.insert(task.id.clone(), max_dep + 1);
}

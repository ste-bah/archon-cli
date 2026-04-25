//! DAG builder — topological sort and level assignment for pipeline steps.

use std::collections::HashMap;

use petgraph::Direction;
use petgraph::algo::{tarjan_scc, toposort};
use petgraph::graphmap::DiGraphMap;

use crate::error::PipelineError;
use crate::spec::PipelineSpec;

/// Validated DAG representing the execution plan.
/// `levels[i]` contains step IDs that can run in parallel at wave `i`.
#[derive(Debug, Clone)]
pub struct Dag {
    /// Each entry is a wave of step IDs that can run concurrently.
    pub levels: Vec<Vec<String>>,
    /// Maps each step ID to its level index.
    pub step_index: HashMap<String, usize>,
}

/// Builds a [`Dag`] from a [`PipelineSpec`].
pub struct DagBuilder;

impl DagBuilder {
    /// Construct a validated [`Dag`] from the given pipeline specification.
    ///
    /// # Errors
    /// * [`PipelineError::MissingStep`] — a step references a dependency that
    ///   does not exist in the spec.
    /// * [`PipelineError::CycleDetected`] — the dependency graph contains a
    ///   cycle (including self-loops).
    pub fn build(spec: &PipelineSpec) -> Result<Dag, PipelineError> {
        // Collect all known step IDs for fast lookup.
        let known_ids: HashMap<&str, ()> = spec.steps.iter().map(|s| (s.id.as_str(), ())).collect();

        // ----- 1. Build the directed graph -----
        let mut graph = DiGraphMap::<&str, ()>::new();

        // Add every step as a node (even if it has no edges).
        for step in &spec.steps {
            graph.add_node(step.id.as_str());
        }

        // Add edges: for every dependency, edge goes from dep -> step
        // (dep must finish before step starts).
        for step in &spec.steps {
            for dep in &step.depends_on {
                // ----- 2. Validate dependency exists -----
                if !known_ids.contains_key(dep.as_str()) {
                    return Err(PipelineError::MissingStep(dep.clone()));
                }
                graph.add_edge(dep.as_str(), step.id.as_str(), ());
            }
        }

        // ----- 3. Detect cycles via toposort -----
        if let Err(_cycle) = toposort(&graph, None) {
            // Use Tarjan's SCC to find a descriptive cycle.
            let sccs = tarjan_scc(&graph);
            let cycle_ids: Vec<String> = sccs
                .into_iter()
                .find(|scc| scc.len() > 1)
                // A self-loop yields an SCC of size 1, but toposort still
                // rejects it.  Fall back to reporting the single node.
                .or_else(|| {
                    // Find the self-loop node directly.
                    spec.steps.iter().find_map(|s| {
                        if s.depends_on.contains(&s.id) {
                            Some(vec![s.id.as_str()])
                        } else {
                            None
                        }
                    })
                })
                .unwrap_or_default()
                .into_iter()
                .map(|n| n.to_string())
                .collect();
            return Err(PipelineError::CycleDetected(cycle_ids));
        }

        // ----- 4. Assign levels via Kahn's algorithm -----
        let levels = Self::assign_levels(&graph, spec);

        // ----- 5. Build step_index -----
        let mut step_index = HashMap::new();
        for (level_num, level) in levels.iter().enumerate() {
            for id in level {
                step_index.insert(id.clone(), level_num);
            }
        }

        Ok(Dag { levels, step_index })
    }

    /// Assign levels using Kahn's algorithm.
    ///
    /// Level 0 = all nodes with in-degree 0. For each level, decrement
    /// neighbor in-degrees; collect new zero-indegree nodes as the next level.
    /// Step IDs within each level are sorted alphabetically for determinism.
    fn assign_levels(graph: &DiGraphMap<&str, ()>, spec: &PipelineSpec) -> Vec<Vec<String>> {
        // Compute in-degree for each node.
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        for step in &spec.steps {
            in_degree.insert(step.id.as_str(), 0);
        }
        for step in &spec.steps {
            for _dep in &step.depends_on {
                // Edge dep -> step.id, so step.id gets +1 in-degree.
                *in_degree.entry(step.id.as_str()).or_insert(0) += 1;
            }
        }

        let mut levels: Vec<Vec<String>> = Vec::new();

        // Seed with in-degree-0 nodes.
        let mut current: Vec<&str> = in_degree
            .iter()
            .filter(|(_, deg)| **deg == 0)
            .map(|(id, _)| *id)
            .collect();
        current.sort();

        while !current.is_empty() {
            levels.push(current.iter().map(|s| s.to_string()).collect());

            let mut next: Vec<&str> = Vec::new();
            for &node in &current {
                for succ in graph.neighbors_directed(node, Direction::Outgoing) {
                    let deg = in_degree.get_mut(succ).expect("node must exist");
                    *deg -= 1;
                    if *deg == 0 {
                        next.push(succ);
                    }
                }
            }
            next.sort();
            current = next;
        }

        levels
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{BackoffKind, OnFailurePolicy, PipelineSpec, RetrySpec, StepSpec};

    /// Helper to build a [`PipelineSpec`] from `(id, agent, depends_on)` triples.
    fn make_spec(steps: Vec<(&str, &str, Vec<&str>)>) -> PipelineSpec {
        PipelineSpec {
            name: "test".to_string(),
            version: "1.0".to_string(),
            global_timeout_secs: 3600,
            max_parallelism: 5,
            steps: steps
                .into_iter()
                .map(|(id, agent, deps)| StepSpec {
                    id: id.to_string(),
                    agent: agent.to_string(),
                    input: serde_json::Value::Object(serde_json::Map::new()),
                    depends_on: deps.into_iter().map(|d| d.to_string()).collect(),
                    retry: RetrySpec {
                        max_attempts: 1,
                        backoff: BackoffKind::Exponential,
                        base_delay_ms: 1000,
                    },
                    timeout_secs: 1800,
                    condition: None,
                    on_failure: OnFailurePolicy::Rollback,
                })
                .collect(),
        }
    }

    #[test]
    fn linear_chain_three_levels() {
        // A -> B -> C
        let spec = make_spec(vec![
            ("A", "agent", vec![]),
            ("B", "agent", vec!["A"]),
            ("C", "agent", vec!["B"]),
        ]);
        let dag = DagBuilder::build(&spec).expect("should succeed");
        assert_eq!(dag.levels, vec![vec!["A"], vec!["B"], vec!["C"]],);
        assert_eq!(dag.step_index["A"], 0);
        assert_eq!(dag.step_index["B"], 1);
        assert_eq!(dag.step_index["C"], 2);
    }

    #[test]
    fn diamond_shape_three_levels() {
        //   A
        //  / \
        // B   C
        //  \ /
        //   D
        let spec = make_spec(vec![
            ("A", "agent", vec![]),
            ("B", "agent", vec!["A"]),
            ("C", "agent", vec!["A"]),
            ("D", "agent", vec!["B", "C"]),
        ]);
        let dag = DagBuilder::build(&spec).expect("should succeed");
        assert_eq!(dag.levels, vec![vec!["A"], vec!["B", "C"], vec!["D"]],);
        assert_eq!(dag.step_index["A"], 0);
        assert_eq!(dag.step_index["B"], 1);
        assert_eq!(dag.step_index["C"], 1);
        assert_eq!(dag.step_index["D"], 2);
    }

    #[test]
    fn three_independent_single_level() {
        let spec = make_spec(vec![
            ("A", "agent", vec![]),
            ("B", "agent", vec![]),
            ("C", "agent", vec![]),
        ]);
        let dag = DagBuilder::build(&spec).expect("should succeed");
        assert_eq!(dag.levels, vec![vec!["A", "B", "C"]]);
        assert_eq!(dag.step_index["A"], 0);
        assert_eq!(dag.step_index["B"], 0);
        assert_eq!(dag.step_index["C"], 0);
    }

    #[test]
    fn cycle_rejected() {
        // A depends_on B, B depends_on A
        let spec = make_spec(vec![("A", "agent", vec!["B"]), ("B", "agent", vec!["A"])]);
        let err = DagBuilder::build(&spec).expect_err("should detect cycle");
        match err {
            PipelineError::CycleDetected(ids) => {
                assert!(ids.contains(&"A".to_string()), "cycle should contain A");
                assert!(ids.contains(&"B".to_string()), "cycle should contain B");
            }
            other => panic!("expected CycleDetected, got: {other:?}"),
        }
    }

    #[test]
    fn missing_dependency_rejected() {
        let spec = make_spec(vec![("A", "agent", vec!["ghost"])]);
        let err = DagBuilder::build(&spec).expect_err("should detect missing dep");
        match err {
            PipelineError::MissingStep(id) => assert_eq!(id, "ghost"),
            other => panic!("expected MissingStep, got: {other:?}"),
        }
    }

    #[test]
    fn self_loop_rejected() {
        let spec = make_spec(vec![("A", "agent", vec!["A"])]);
        let err = DagBuilder::build(&spec).expect_err("should detect self-loop");
        match err {
            PipelineError::CycleDetected(ids) => {
                assert!(
                    ids.contains(&"A".to_string()),
                    "self-loop cycle should contain A, got: {ids:?}"
                );
            }
            other => panic!("expected CycleDetected, got: {other:?}"),
        }
    }
}

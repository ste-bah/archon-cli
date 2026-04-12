//! TASK-AGS-504: CompositeAgentPattern — hierarchical delegation + cycle detection.
//!
//! Enables hierarchical agent composition: a parent delegates to child agents
//! in DAG topological order. Cycles are detected at build time (EC-ARCH-005)
//! using `petgraph::algo::is_cyclic_directed`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use petgraph::algo;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{Pattern, PatternCtx, PatternError, PatternKind, PatternRegistry};

// ---------------------------------------------------------------------------
// CompositeConfig
// ---------------------------------------------------------------------------

/// Configuration for a composite (DAG) pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompositeConfig {
    /// Nodes in the DAG.
    pub nodes: Vec<CompositeNode>,
    /// Edges connecting nodes.
    pub edges: Vec<CompositeEdge>,
    /// Name of the root-sink node whose output is the composite's output.
    pub root: String,
}

/// A node in the composite DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompositeNode {
    /// Unique name within this composite.
    pub name: String,
    /// Agent to invoke for this node.
    pub agent: String,
}

/// A directed edge in the composite DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompositeEdge {
    /// Source node name.
    pub from: String,
    /// Target node name.
    pub to: String,
    /// Optional dotted-key path to extract from the source's output
    /// as input to the target.
    pub input_path: Option<String>,
}

// ---------------------------------------------------------------------------
// CompositeAgentPattern — builder
// ---------------------------------------------------------------------------

/// Builder entry point for composite patterns.
pub struct CompositeAgentPattern;

impl CompositeAgentPattern {
    /// Build and validate a composite DAG from config.
    ///
    /// Returns `PatternError::CompositeCycle` if the graph contains a cycle
    /// (EC-ARCH-005 / TC-PAT-05: detected at composition time, not execution).
    pub fn build(cfg: &CompositeConfig) -> Result<CompiledComposite, PatternError> {
        let mut graph = DiGraph::<String, Option<String>>::new();
        let mut name_to_idx: HashMap<String, NodeIndex> = HashMap::new();

        // Add nodes.
        for node in &cfg.nodes {
            let idx = graph.add_node(node.name.clone());
            name_to_idx.insert(node.name.clone(), idx);
        }

        // Add edges.
        for edge in &cfg.edges {
            let from = name_to_idx.get(&edge.from).ok_or_else(|| {
                PatternError::Execution(format!(
                    "composite edge references unknown node '{}'",
                    edge.from
                ))
            })?;
            let to = name_to_idx.get(&edge.to).ok_or_else(|| {
                PatternError::Execution(format!(
                    "composite edge references unknown node '{}'",
                    edge.to
                ))
            })?;
            graph.add_edge(*from, *to, edge.input_path.clone());
        }

        // Cycle detection (EC-ARCH-005).
        if algo::is_cyclic_directed(&graph) {
            let path = find_cycle_path(&graph);
            return Err(PatternError::CompositeCycle { path });
        }

        // Topological sort.
        let order = algo::toposort(&graph, None).map_err(|e| {
            PatternError::CompositeCycle {
                path: vec![graph[e.node_id()].clone()],
            }
        })?;

        // Build node->agent map.
        let mut node_agents: HashMap<String, String> = HashMap::new();
        for node in &cfg.nodes {
            node_agents.insert(node.name.clone(), node.agent.clone());
        }

        Ok(CompiledComposite {
            order,
            graph,
            name_to_idx,
            node_agents,
            root: cfg.root.clone(),
        })
    }
}

/// Find a cycle path in a directed graph using DFS.
fn find_cycle_path(graph: &DiGraph<String, Option<String>>) -> Vec<String> {
    use petgraph::visit::{depth_first_search, DfsEvent, Control};

    let mut path: Vec<NodeIndex> = Vec::new();
    let mut on_stack: Vec<bool> = vec![false; graph.node_count()];

    let result = depth_first_search(graph, graph.node_indices(), |event| {
        match event {
            DfsEvent::Discover(n, _) => {
                path.push(n);
                on_stack[n.index()] = true;
                Control::Continue
            }
            DfsEvent::BackEdge(_, target) => {
                // Found cycle — extract the path from target back to target.
                let mut cycle: Vec<String> = Vec::new();
                let mut found_start = false;
                for &node in &path {
                    if node == target {
                        found_start = true;
                    }
                    if found_start {
                        cycle.push(graph[node].clone());
                    }
                }
                cycle.push(graph[target].clone()); // close the cycle
                Control::Break(cycle)
            }
            DfsEvent::Finish(n, _) => {
                if let Some(pos) = path.iter().rposition(|&x| x == n) {
                    path.truncate(pos);
                }
                on_stack[n.index()] = false;
                Control::Continue
            }
            _ => Control::Continue,
        }
    });

    match result {
        Control::Break(cycle) => cycle,
        _ => vec!["unknown cycle".into()],
    }
}

// ---------------------------------------------------------------------------
// CompiledComposite — the executable pattern
// ---------------------------------------------------------------------------

/// A validated, topologically-sorted composite DAG ready for execution.
#[derive(Debug)]
pub struct CompiledComposite {
    order: Vec<NodeIndex>,
    graph: DiGraph<String, Option<String>>,
    name_to_idx: HashMap<String, NodeIndex>,
    node_agents: HashMap<String, String>,
    root: String,
}

#[async_trait]
impl Pattern for CompiledComposite {
    fn kind(&self) -> PatternKind {
        PatternKind::Composite
    }

    async fn execute(
        &self,
        input: Value,
        ctx: PatternCtx,
    ) -> Result<Value, PatternError> {
        let mut outputs: HashMap<NodeIndex, Value> = HashMap::new();

        for &node_idx in &self.order {
            let node_name = &self.graph[node_idx];
            let agent = self.node_agents.get(node_name).ok_or_else(|| {
                PatternError::Execution(format!(
                    "no agent mapped for node '{node_name}'"
                ))
            })?;

            // Gather input from predecessors or use composite input.
            let node_input = {
                let predecessors: Vec<_> = self
                    .graph
                    .edges_directed(node_idx, petgraph::Direction::Incoming)
                    .collect();

                if predecessors.is_empty() {
                    // Source node — receives the composite's input.
                    input.clone()
                } else if predecessors.len() == 1 {
                    let pred = predecessors[0];
                    let pred_output = outputs
                        .get(&pred.source())
                        .cloned()
                        .unwrap_or(Value::Null);
                    extract_path(&pred_output, pred.weight())
                } else {
                    // Multiple predecessors — merge their outputs.
                    let mut merged = serde_json::Map::new();
                    for pred in predecessors {
                        let pred_name = &self.graph[pred.source()];
                        let pred_output = outputs
                            .get(&pred.source())
                            .cloned()
                            .unwrap_or(Value::Null);
                        let extracted = extract_path(&pred_output, pred.weight());
                        merged.insert(pred_name.clone(), extracted);
                    }
                    Value::Object(merged)
                }
            };

            let result = ctx.task_service.submit(agent, node_input).await?;
            outputs.insert(node_idx, result);
        }

        // Return the root-sink node's output.
        let root_idx = self.name_to_idx.get(&self.root).ok_or_else(|| {
            PatternError::Execution(format!(
                "root node '{}' not found in composite",
                self.root
            ))
        })?;

        outputs
            .remove(root_idx)
            .ok_or_else(|| {
                PatternError::Execution("root node produced no output".into())
            })
    }
}

/// Simple dotted-key path extractor. If `path` is None, returns the whole value.
fn extract_path(value: &Value, path: &Option<String>) -> Value {
    match path {
        None => value.clone(),
        Some(p) => {
            let mut current = value;
            for key in p.split('.') {
                match current.get(key) {
                    Some(v) => current = v,
                    None => return Value::Null,
                }
            }
            current.clone()
        }
    }
}

/// Register a composite pattern factory under name "composite".
pub fn register(reg: &PatternRegistry) {
    // Composite patterns are built per-config via CompositeAgentPattern::build(),
    // so the registry entry is a placeholder that documents the name.
    // Real composite instances are constructed at composition time.
    struct CompositeFactory;

    #[async_trait]
    impl Pattern for CompositeFactory {
        fn kind(&self) -> PatternKind {
            PatternKind::Composite
        }

        async fn execute(
            &self,
            input: Value,
            _ctx: PatternCtx,
        ) -> Result<Value, PatternError> {
            Err(PatternError::Execution(
                "composite factory: use CompositeAgentPattern::build() to create instances".into(),
            ))
        }
    }

    reg.register("composite", Arc::new(CompositeFactory));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use serde_json::json;

    use crate::patterns::{PatternRegistry, TaskServiceHandle};

    // Recording task service: tracks execution order.
    struct RecordingTaskService {
        calls: Mutex<Vec<String>>,
    }

    impl RecordingTaskService {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
            }
        }

        fn call_order(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl TaskServiceHandle for RecordingTaskService {
        async fn submit(
            &self,
            agent: &str,
            input: Value,
        ) -> Result<Value, PatternError> {
            self.calls.lock().unwrap().push(agent.to_string());
            Ok(json!({"from": agent, "input": input}))
        }
    }

    fn make_ctx(svc: Arc<dyn TaskServiceHandle>) -> PatternCtx {
        PatternCtx {
            task_service: svc,
            registry: Arc::new(PatternRegistry::new()),
            trace_id: "test".into(),
            deadline: None,
        }
    }

    #[test]
    fn test_composite_cycle_a_b_a_detected_at_build() {
        // TC-PAT-05, EC-ARCH-005: 2-node cycle A->B->A.
        let cfg = CompositeConfig {
            nodes: vec![
                CompositeNode { name: "A".into(), agent: "agent-a".into() },
                CompositeNode { name: "B".into(), agent: "agent-b".into() },
            ],
            edges: vec![
                CompositeEdge { from: "A".into(), to: "B".into(), input_path: None },
                CompositeEdge { from: "B".into(), to: "A".into(), input_path: None },
            ],
            root: "B".into(),
        };

        let err = CompositeAgentPattern::build(&cfg).unwrap_err();
        match err {
            PatternError::CompositeCycle { path } => {
                assert!(
                    path.contains(&"A".to_string()) && path.contains(&"B".to_string()),
                    "cycle path must contain A and B: {path:?}"
                );
                // Path should close the cycle (last == first).
                assert_eq!(
                    path.first(),
                    path.last(),
                    "cycle path must be closed: {path:?}"
                );
            }
            other => panic!("expected CompositeCycle, got: {other}"),
        }
    }

    #[test]
    fn test_composite_cycle_a_b_c_a_detected_at_build() {
        // 3-node cycle A->B->C->A.
        let cfg = CompositeConfig {
            nodes: vec![
                CompositeNode { name: "A".into(), agent: "a".into() },
                CompositeNode { name: "B".into(), agent: "b".into() },
                CompositeNode { name: "C".into(), agent: "c".into() },
            ],
            edges: vec![
                CompositeEdge { from: "A".into(), to: "B".into(), input_path: None },
                CompositeEdge { from: "B".into(), to: "C".into(), input_path: None },
                CompositeEdge { from: "C".into(), to: "A".into(), input_path: None },
            ],
            root: "C".into(),
        };

        let err = CompositeAgentPattern::build(&cfg).unwrap_err();
        assert!(matches!(err, PatternError::CompositeCycle { .. }));
    }

    #[tokio::test]
    async fn test_composite_dag_executes_in_topo_order() {
        // DAG: A->B, A->C, B->D, C->D. Valid topo: A before B,C; B,C before D.
        let cfg = CompositeConfig {
            nodes: vec![
                CompositeNode { name: "A".into(), agent: "agent-a".into() },
                CompositeNode { name: "B".into(), agent: "agent-b".into() },
                CompositeNode { name: "C".into(), agent: "agent-c".into() },
                CompositeNode { name: "D".into(), agent: "agent-d".into() },
            ],
            edges: vec![
                CompositeEdge { from: "A".into(), to: "B".into(), input_path: None },
                CompositeEdge { from: "A".into(), to: "C".into(), input_path: None },
                CompositeEdge { from: "B".into(), to: "D".into(), input_path: None },
                CompositeEdge { from: "C".into(), to: "D".into(), input_path: None },
            ],
            root: "D".into(),
        };

        let compiled = CompositeAgentPattern::build(&cfg).unwrap();
        let svc = Arc::new(RecordingTaskService::new());
        let ctx = make_ctx(svc.clone());

        let _result = compiled.execute(json!({"start": true}), ctx).await.unwrap();

        let order = svc.call_order();
        assert_eq!(order.len(), 4);

        // A must come before B and C.
        let pos_a = order.iter().position(|x| x == "agent-a").unwrap();
        let pos_b = order.iter().position(|x| x == "agent-b").unwrap();
        let pos_c = order.iter().position(|x| x == "agent-c").unwrap();
        let pos_d = order.iter().position(|x| x == "agent-d").unwrap();

        assert!(pos_a < pos_b, "A must precede B");
        assert!(pos_a < pos_c, "A must precede C");
        assert!(pos_b < pos_d, "B must precede D");
        assert!(pos_c < pos_d, "C must precede D");
    }

    #[tokio::test]
    async fn test_composite_diamond_merges_inputs_to_sink() {
        // Diamond: A->B, A->C, B->D, C->D. D receives merged B+C outputs.
        let cfg = CompositeConfig {
            nodes: vec![
                CompositeNode { name: "A".into(), agent: "a".into() },
                CompositeNode { name: "B".into(), agent: "b".into() },
                CompositeNode { name: "C".into(), agent: "c".into() },
                CompositeNode { name: "D".into(), agent: "d".into() },
            ],
            edges: vec![
                CompositeEdge { from: "A".into(), to: "B".into(), input_path: None },
                CompositeEdge { from: "A".into(), to: "C".into(), input_path: None },
                CompositeEdge { from: "B".into(), to: "D".into(), input_path: None },
                CompositeEdge { from: "C".into(), to: "D".into(), input_path: None },
            ],
            root: "D".into(),
        };

        let compiled = CompositeAgentPattern::build(&cfg).unwrap();
        let svc = Arc::new(RecordingTaskService::new());
        let ctx = make_ctx(svc.clone());

        let result = compiled.execute(json!({"x": 1}), ctx).await.unwrap();

        // D's input should contain both B and C outputs as merged keys.
        let d_input = &result["input"];
        assert!(
            d_input.get("B").is_some() && d_input.get("C").is_some(),
            "D must receive both B and C outputs: {d_input}"
        );
    }

    #[tokio::test]
    async fn test_composite_empty_graph_returns_input() {
        // No nodes → still need a root, but empty graph is an edge case.
        // With no nodes, build should still succeed but execute would fail
        // on missing root. Use a single-node graph as the minimal case.
        let cfg = CompositeConfig {
            nodes: vec![
                CompositeNode { name: "A".into(), agent: "a".into() },
            ],
            edges: vec![],
            root: "A".into(),
        };

        let compiled = CompositeAgentPattern::build(&cfg).unwrap();
        let svc = Arc::new(RecordingTaskService::new());
        let ctx = make_ctx(svc.clone());

        let result = compiled.execute(json!({"passthrough": true}), ctx).await.unwrap();

        // Single node receives input directly and returns its output.
        assert_eq!(result["input"], json!({"passthrough": true}));
    }
}

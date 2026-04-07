//! CausalMemory + CausalHypergraph — causal reasoning subsystem (TASK-PIPE-F04).
//!
//! Provides a directed acyclic hypergraph where edges can have multiple sources
//! (multi-cause relationships like A+B → C). The `CausalMemory` facade wraps the
//! graph with traversal, cycle detection, and inference helpers consumed by
//! `ReasoningBank::reason_causal`.

use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Unique identifier for a node in the causal graph.
pub type NodeID = String;

/// A node in the causal hypergraph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalNode {
    pub id: NodeID,
    pub label: String,
    pub node_type: String,
    pub metadata: HashMap<String, String>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// A directed hyperedge: one or more source nodes cause a single target node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalLink {
    pub id: String,
    pub source_ids: Vec<NodeID>,
    pub target_id: NodeID,
    pub strength: f64,
    pub evidence: String,
    pub created_at: u64,
}

/// Result of a causal inference query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceResult {
    pub causes: Vec<NodeID>,
    pub effects: Vec<NodeID>,
    pub chain: Vec<NodeID>,
    pub confidence: f64,
}

/// Summary statistics for the causal graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalGraphStats {
    pub node_count: usize,
    pub link_count: usize,
    pub max_fan_in: usize,
    pub max_fan_out: usize,
}

/// Options controlling graph traversal behaviour.
pub struct TraversalOptions {
    pub max_hops: usize,
    pub direction: TraversalDirection,
}

/// Direction of traversal through the causal graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraversalDirection {
    Forward,
    Backward,
}

// ---------------------------------------------------------------------------
// CausalHypergraph
// ---------------------------------------------------------------------------

/// Directed acyclic hypergraph storing causal relationships.
///
/// Maintains bidirectional indices so that both forward (effects) and backward
/// (causes) traversal are O(degree) per hop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalHypergraph {
    nodes: HashMap<NodeID, CausalNode>,
    links: HashMap<String, CausalLink>,
    /// node → set of outgoing link IDs (node is a source of these links).
    forward_index: HashMap<NodeID, HashSet<String>>,
    /// node → set of incoming link IDs (node is the target of these links).
    backward_index: HashMap<NodeID, HashSet<String>>,
}

impl CausalHypergraph {
    /// Create an empty hypergraph.
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            links: HashMap::new(),
            forward_index: HashMap::new(),
            backward_index: HashMap::new(),
        }
    }

    /// Add a node to the graph. If the node ID already exists it is replaced.
    pub fn add_node(&mut self, node: CausalNode) {
        let id = node.id.clone();
        self.nodes.insert(id.clone(), node);
        self.forward_index.entry(id.clone()).or_default();
        self.backward_index.entry(id).or_default();
    }

    /// Insert a hyperedge after verifying it would not create a cycle.
    ///
    /// Returns `Err` if the link would introduce a cycle or if any referenced
    /// node does not exist in the graph.
    pub fn add_link(&mut self, link: CausalLink) -> Result<(), String> {
        // Validate nodes exist.
        for src in &link.source_ids {
            if !self.nodes.contains_key(src) {
                return Err(format!("Source node '{src}' not found in graph"));
            }
        }
        if !self.nodes.contains_key(&link.target_id) {
            return Err(format!(
                "Target node '{}' not found in graph",
                link.target_id
            ));
        }

        // Cycle check.
        if CycleDetector::would_create_cycle(self, &link.source_ids, &link.target_id) {
            return Err(format!(
                "Adding link '{}' would create a cycle",
                link.id
            ));
        }

        // Update indices.
        for src in &link.source_ids {
            self.forward_index
                .entry(src.clone())
                .or_default()
                .insert(link.id.clone());
        }
        self.backward_index
            .entry(link.target_id.clone())
            .or_default()
            .insert(link.id.clone());

        self.links.insert(link.id.clone(), link);
        Ok(())
    }

    /// Retrieve a node by ID.
    pub fn get_node(&self, id: &str) -> Option<&CausalNode> {
        self.nodes.get(id)
    }

    /// Retrieve a link by ID.
    pub fn get_link(&self, id: &str) -> Option<&CausalLink> {
        self.links.get(id)
    }

    /// Return summary statistics.
    pub fn get_stats(&self) -> CausalGraphStats {
        let max_fan_out = self
            .forward_index
            .values()
            .map(|s| s.len())
            .max()
            .unwrap_or(0);

        let max_fan_in = self
            .backward_index
            .values()
            .map(|s| s.len())
            .max()
            .unwrap_or(0);

        CausalGraphStats {
            node_count: self.nodes.len(),
            link_count: self.links.len(),
            max_fan_in,
            max_fan_out,
        }
    }

    /// Serialize the graph to a JSON string.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Deserialize a graph from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    // -- internal accessors used by traversal / cycle detection ---------------

    /// Outgoing link IDs for a node.
    fn outgoing_links(&self, node_id: &str) -> impl Iterator<Item = &CausalLink> {
        self.forward_index
            .get(node_id)
            .into_iter()
            .flat_map(|ids| ids.iter().filter_map(|lid| self.links.get(lid)))
    }

    /// Incoming link IDs for a node.
    fn incoming_links(&self, node_id: &str) -> impl Iterator<Item = &CausalLink> {
        self.backward_index
            .get(node_id)
            .into_iter()
            .flat_map(|ids| ids.iter().filter_map(|lid| self.links.get(lid)))
    }

    /// All node IDs.
    pub fn node_ids(&self) -> impl Iterator<Item = &NodeID> {
        self.nodes.keys()
    }
}

impl Default for CausalHypergraph {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// CausalTraversal
// ---------------------------------------------------------------------------

/// BFS-based traversal over the causal hypergraph.
pub struct CausalTraversal;

impl CausalTraversal {
    /// Return all nodes reachable from `start_node` following outgoing links
    /// (i.e. effects / downstream consequences), up to `max_hops` levels deep.
    pub fn traverse_forward(
        graph: &CausalHypergraph,
        start_node: &str,
        max_hops: usize,
    ) -> Vec<NodeID> {
        Self::bfs(graph, start_node, max_hops, TraversalDirection::Forward)
    }

    /// Return all nodes reachable from `start_node` following incoming links
    /// (i.e. causes / upstream ancestors), up to `max_hops` levels deep.
    pub fn traverse_backward(
        graph: &CausalHypergraph,
        start_node: &str,
        max_hops: usize,
    ) -> Vec<NodeID> {
        Self::bfs(graph, start_node, max_hops, TraversalDirection::Backward)
    }

    fn bfs(
        graph: &CausalHypergraph,
        start_node: &str,
        max_hops: usize,
        direction: TraversalDirection,
    ) -> Vec<NodeID> {
        let mut visited = HashSet::new();
        let mut result = Vec::new();
        let mut queue: VecDeque<(String, usize)> = VecDeque::new();

        visited.insert(start_node.to_string());
        queue.push_back((start_node.to_string(), 0));

        while let Some((current, depth)) = queue.pop_front() {
            if depth >= max_hops {
                continue;
            }

            let neighbours: Vec<NodeID> = match direction {
                TraversalDirection::Forward => {
                    graph
                        .outgoing_links(&current)
                        .map(|link| link.target_id.clone())
                        .collect()
                }
                TraversalDirection::Backward => {
                    graph
                        .incoming_links(&current)
                        .flat_map(|link| link.source_ids.clone())
                        .collect()
                }
            };

            for neighbour in neighbours {
                if visited.insert(neighbour.clone()) {
                    result.push(neighbour.clone());
                    queue.push_back((neighbour, depth + 1));
                }
            }
        }

        result
    }
}

// ---------------------------------------------------------------------------
// CycleDetector
// ---------------------------------------------------------------------------

/// Detects whether adding a proposed hyperedge would create a cycle.
pub struct CycleDetector;

impl CycleDetector {
    /// Returns `true` if adding an edge from `source_ids → target_id` would
    /// introduce a cycle (i.e. any source is reachable from the target via
    /// existing forward links).
    pub fn would_create_cycle(
        graph: &CausalHypergraph,
        source_ids: &[NodeID],
        target_id: &str,
    ) -> bool {
        let source_set: HashSet<&str> = source_ids.iter().map(|s| s.as_str()).collect();

        // DFS forward from target_id; if we reach any source, it's a cycle.
        let mut stack = vec![target_id.to_string()];
        let mut visited = HashSet::new();
        visited.insert(target_id.to_string());

        while let Some(current) = stack.pop() {
            if source_set.contains(current.as_str()) {
                return true;
            }
            for link in graph.outgoing_links(&current) {
                if visited.insert(link.target_id.clone()) {
                    stack.push(link.target_id.clone());
                }
            }
        }

        false
    }
}

// ---------------------------------------------------------------------------
// CausalMemory — high-level façade
// ---------------------------------------------------------------------------

/// High-level API for causal reasoning, wrapping `CausalHypergraph`,
/// `CausalTraversal`, and `CycleDetector`.
pub struct CausalMemory {
    graph: CausalHypergraph,
    next_link_id: u64,
}

impl CausalMemory {
    /// Create an empty causal memory.
    pub fn new() -> Self {
        Self {
            graph: CausalHypergraph::new(),
            next_link_id: 0,
        }
    }

    /// Access the underlying graph (e.g. for serialization).
    pub fn graph(&self) -> &CausalHypergraph {
        &self.graph
    }

    /// Register a causal node. Creates the node if it doesn't already exist.
    pub fn ensure_node(&mut self, id: &str, label: &str, node_type: &str) {
        if self.graph.get_node(id).is_none() {
            let now = now_epoch();
            self.graph.add_node(CausalNode {
                id: id.to_string(),
                label: label.to_string(),
                node_type: node_type.to_string(),
                metadata: HashMap::new(),
                created_at: now,
                updated_at: now,
            });
        }
    }

    /// Add a causal link. Automatically creates placeholder nodes for any IDs
    /// that do not yet exist in the graph.
    pub fn add_causal_link(
        &mut self,
        source_ids: &[&str],
        target_id: &str,
        strength: f64,
        evidence: &str,
    ) -> Result<String, String> {
        // Auto-create missing nodes.
        for src in source_ids {
            self.ensure_node(src, src, "auto");
        }
        self.ensure_node(target_id, target_id, "auto");

        let link_id = format!("link-{}", self.next_link_id);
        self.next_link_id += 1;

        let link = CausalLink {
            id: link_id.clone(),
            source_ids: source_ids.iter().map(|s| s.to_string()).collect(),
            target_id: target_id.to_string(),
            strength: strength.clamp(0.0, 1.0),
            evidence: evidence.to_string(),
            created_at: now_epoch(),
        };

        self.graph.add_link(link)?;
        Ok(link_id)
    }

    /// Find causes (upstream ancestors) of `node_id` within `max_hops`.
    pub fn find_causes(&self, node_id: &str, max_hops: usize) -> Vec<NodeID> {
        CausalTraversal::traverse_backward(&self.graph, node_id, max_hops)
    }

    /// Find effects (downstream descendants) of `node_id` within `max_hops`.
    pub fn find_effects(&self, node_id: &str, max_hops: usize) -> Vec<NodeID> {
        CausalTraversal::traverse_forward(&self.graph, node_id, max_hops)
    }

    /// Infer causation for a query node: collects causes and effects within
    /// 5 hops and computes an aggregate confidence from link strengths.
    pub fn infer_causation(&self, query_node: &str) -> InferenceResult {
        let default_hops = 5;
        let causes = self.find_causes(query_node, default_hops);
        let effects = self.find_effects(query_node, default_hops);

        // Build the chain: causes → query → effects.
        let mut chain: Vec<NodeID> = causes.iter().rev().cloned().collect();
        chain.push(query_node.to_string());
        chain.extend(effects.iter().cloned());

        // Aggregate confidence from incoming link strengths to the query node.
        let incoming_strengths: Vec<f64> = self
            .graph
            .incoming_links(query_node)
            .map(|l| l.strength)
            .collect();

        let confidence = if incoming_strengths.is_empty() {
            if effects.is_empty() {
                0.0
            } else {
                // Node has outgoing effects but no incoming — treat as root cause.
                let outgoing_strengths: Vec<f64> = self
                    .graph
                    .outgoing_links(query_node)
                    .map(|l| l.strength)
                    .collect();
                if outgoing_strengths.is_empty() {
                    0.0
                } else {
                    outgoing_strengths.iter().sum::<f64>() / outgoing_strengths.len() as f64
                }
            }
        } else {
            incoming_strengths.iter().sum::<f64>() / incoming_strengths.len() as f64
        };

        InferenceResult {
            causes,
            effects,
            chain,
            confidence,
        }
    }

    /// Return summary statistics for the causal graph.
    pub fn get_stats(&self) -> CausalGraphStats {
        self.graph.get_stats()
    }
}

impl Default for CausalMemory {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(id: &str) -> CausalNode {
        CausalNode {
            id: id.to_string(),
            label: id.to_string(),
            node_type: "test".to_string(),
            metadata: HashMap::new(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn make_link(id: &str, sources: &[&str], target: &str, strength: f64) -> CausalLink {
        CausalLink {
            id: id.to_string(),
            source_ids: sources.iter().map(|s| s.to_string()).collect(),
            target_id: target.to_string(),
            strength,
            evidence: "test evidence".to_string(),
            created_at: 0,
        }
    }

    // 1. Add nodes and links, verify traversal A->B->C in 2 hops
    #[test]
    fn test_traversal_a_b_c_in_two_hops() {
        let mut graph = CausalHypergraph::new();
        graph.add_node(make_node("A"));
        graph.add_node(make_node("B"));
        graph.add_node(make_node("C"));

        graph.add_link(make_link("l1", &["A"], "B", 0.8)).unwrap();
        graph.add_link(make_link("l2", &["B"], "C", 0.9)).unwrap();

        let reachable = CausalTraversal::traverse_forward(&graph, "A", 2);
        assert!(reachable.contains(&"B".to_string()));
        assert!(reachable.contains(&"C".to_string()));
        assert_eq!(reachable.len(), 2);
    }

    // 2. Cycle detection: A->B->C, adding C->A rejected
    #[test]
    fn test_cycle_detection_rejects_cycle() {
        let mut graph = CausalHypergraph::new();
        graph.add_node(make_node("A"));
        graph.add_node(make_node("B"));
        graph.add_node(make_node("C"));

        graph.add_link(make_link("l1", &["A"], "B", 0.8)).unwrap();
        graph.add_link(make_link("l2", &["B"], "C", 0.9)).unwrap();

        // C -> A should be rejected.
        let result = graph.add_link(make_link("l3", &["C"], "A", 0.7));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cycle"));
    }

    // 3. 5-hop deep chain traversal works
    #[test]
    fn test_five_hop_chain_traversal() {
        let mut graph = CausalHypergraph::new();
        let ids: Vec<String> = (0..6).map(|i| format!("N{i}")).collect();
        for id in &ids {
            graph.add_node(make_node(id));
        }
        // N0 -> N1 -> N2 -> N3 -> N4 -> N5
        for i in 0..5 {
            graph
                .add_link(make_link(
                    &format!("l{i}"),
                    &[&ids[i]],
                    &ids[i + 1],
                    0.5,
                ))
                .unwrap();
        }

        let reachable = CausalTraversal::traverse_forward(&graph, "N0", 5);
        assert_eq!(reachable.len(), 5); // N1..N5
        assert!(reachable.contains(&"N5".to_string()));
    }

    // 4. Multi-cause hyperedge: A+B->C stored and traversable
    #[test]
    fn test_multi_cause_hyperedge() {
        let mut graph = CausalHypergraph::new();
        graph.add_node(make_node("A"));
        graph.add_node(make_node("B"));
        graph.add_node(make_node("C"));

        graph
            .add_link(make_link("l1", &["A", "B"], "C", 0.9))
            .unwrap();

        // Forward from A reaches C.
        let from_a = CausalTraversal::traverse_forward(&graph, "A", 1);
        assert!(from_a.contains(&"C".to_string()));

        // Forward from B also reaches C.
        let from_b = CausalTraversal::traverse_forward(&graph, "B", 1);
        assert!(from_b.contains(&"C".to_string()));

        // Backward from C reaches both A and B.
        let causes = CausalTraversal::traverse_backward(&graph, "C", 1);
        assert!(causes.contains(&"A".to_string()));
        assert!(causes.contains(&"B".to_string()));
    }

    // 5. Bidirectional indices correct (forward and backward)
    #[test]
    fn test_bidirectional_indices() {
        let mut graph = CausalHypergraph::new();
        graph.add_node(make_node("X"));
        graph.add_node(make_node("Y"));
        graph.add_node(make_node("Z"));

        graph.add_link(make_link("l1", &["X"], "Y", 0.5)).unwrap();
        graph.add_link(make_link("l2", &["Y"], "Z", 0.6)).unwrap();

        // Forward index: X has outgoing l1, Y has outgoing l2.
        assert!(graph.forward_index["X"].contains("l1"));
        assert!(graph.forward_index["Y"].contains("l2"));

        // Backward index: Y has incoming l1, Z has incoming l2.
        assert!(graph.backward_index["Y"].contains("l1"));
        assert!(graph.backward_index["Z"].contains("l2"));

        // X has no incoming links, Z has no outgoing links.
        assert!(graph.backward_index["X"].is_empty());
        assert!(graph.forward_index["Z"].is_empty());
    }

    // 6. Serialization roundtrip preserves graph state
    #[test]
    fn test_serialization_roundtrip() {
        let mut graph = CausalHypergraph::new();
        graph.add_node(make_node("A"));
        graph.add_node(make_node("B"));
        graph.add_link(make_link("l1", &["A"], "B", 0.75)).unwrap();

        let json = graph.to_json().unwrap();
        let restored = CausalHypergraph::from_json(&json).unwrap();

        assert_eq!(restored.nodes.len(), 2);
        assert_eq!(restored.links.len(), 1);
        assert!(restored.get_node("A").is_some());
        assert!(restored.get_node("B").is_some());
        assert!(restored.get_link("l1").is_some());
        assert_eq!(restored.get_link("l1").unwrap().strength, 0.75);

        // Forward/backward indices preserved.
        assert!(restored.forward_index["A"].contains("l1"));
        assert!(restored.backward_index["B"].contains("l1"));
    }

    // 7. CausalMemory.infer_causation returns real results
    #[test]
    fn test_causal_memory_infer_causation() {
        let mut mem = CausalMemory::new();
        mem.add_causal_link(&["A"], "B", 0.8, "A causes B").unwrap();
        mem.add_causal_link(&["B"], "C", 0.9, "B causes C").unwrap();

        let result = mem.infer_causation("B");

        // B has cause A, effect C.
        assert!(result.causes.contains(&"A".to_string()));
        assert!(result.effects.contains(&"C".to_string()));
        // Chain: A -> B -> C.
        assert_eq!(result.chain.len(), 3);
        assert!(result.confidence > 0.0);
    }

    // 8. get_stats reports correct counts
    #[test]
    fn test_get_stats() {
        let mut mem = CausalMemory::new();
        mem.add_causal_link(&["A"], "B", 0.8, "ev1").unwrap();
        mem.add_causal_link(&["B"], "C", 0.7, "ev2").unwrap();
        mem.add_causal_link(&["A"], "C", 0.6, "ev3").unwrap();

        let stats = mem.get_stats();
        assert_eq!(stats.node_count, 3);
        assert_eq!(stats.link_count, 3);
        // C has max fan_in of 2 (from A and B).
        assert_eq!(stats.max_fan_in, 2);
        // A has max fan_out of 2 (to B and C).
        assert_eq!(stats.max_fan_out, 2);
    }

    // 9. find_causes on leaf node returns empty
    #[test]
    fn test_find_causes_leaf_node_empty() {
        let mut mem = CausalMemory::new();
        mem.add_causal_link(&["A"], "B", 0.8, "ev").unwrap();

        // A is a root — it has no causes.
        let causes = mem.find_causes("A", 5);
        assert!(causes.is_empty());
    }

    // 10. find_effects on root node returns all descendants
    #[test]
    fn test_find_effects_root_returns_all_descendants() {
        let mut mem = CausalMemory::new();
        mem.add_causal_link(&["R"], "A", 0.8, "ev1").unwrap();
        mem.add_causal_link(&["A"], "B", 0.7, "ev2").unwrap();
        mem.add_causal_link(&["A"], "C", 0.6, "ev3").unwrap();
        mem.add_causal_link(&["B"], "D", 0.5, "ev4").unwrap();

        let effects = mem.find_effects("R", 5);
        assert_eq!(effects.len(), 4); // A, B, C, D
        assert!(effects.contains(&"A".to_string()));
        assert!(effects.contains(&"B".to_string()));
        assert!(effects.contains(&"C".to_string()));
        assert!(effects.contains(&"D".to_string()));
    }
}

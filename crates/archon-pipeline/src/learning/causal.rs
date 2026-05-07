//! CausalMemory + CausalHypergraph - causal reasoning subsystem (TASK-PIPE-F04).
//!
//! Provides a directed acyclic hypergraph where edges can have multiple sources
//! (multi-cause relationships like A+B -> C). The `CausalMemory` facade wraps the
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
    /// node -> set of outgoing link IDs (node is a source of these links).
    forward_index: HashMap<NodeID, HashSet<String>>,
    /// node -> set of incoming link IDs (node is the target of these links).
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
            return Err(format!("Adding link '{}' would create a cycle", link.id));
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
                TraversalDirection::Forward => graph
                    .outgoing_links(&current)
                    .map(|link| link.target_id.clone())
                    .collect(),
                TraversalDirection::Backward => graph
                    .incoming_links(&current)
                    .flat_map(|link| link.source_ids.clone())
                    .collect(),
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
    /// Returns `true` if adding an edge from `source_ids -> target_id` would
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
// CausalMemory - high-level facade
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

        // Build the chain: causes -> query -> effects.
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
                // Node has outgoing effects but no incoming - treat as root cause.
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

#[cfg(test)]
mod tests;

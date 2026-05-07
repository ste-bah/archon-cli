use std::collections::{HashMap, HashSet};

use super::GnnEnhancer;
use super::math::{add_vectors, attention_score, normalize, project, softmax, weighted_aggregate};
use super::types::{GraphEnhancementResult, TrajectoryEdge, TrajectoryGraph, TrajectoryNode};

impl GnnEnhancer {
    /// Enhance an embedding using graph context (neighborhood aggregation).
    pub fn enhance_with_graph(
        &self,
        embedding: &[f32],
        graph: &TrajectoryGraph,
    ) -> GraphEnhancementResult {
        let start = std::time::Instant::now();

        let pruned = self.prune_graph(graph);
        let feature_matrix = self.build_feature_matrix(&pruned);
        let adjacency = self.build_adjacency_matrix(&pruned);
        let aggregated = self.aggregate_neighborhood(embedding, &feature_matrix, &adjacency);

        // Recursively enhance the aggregated embedding
        let result = self.enhance(&aggregated, None, None, false);

        GraphEnhancementResult {
            enhanced: result.enhanced,
            processing_time_ms: start.elapsed().as_micros() as f64 / 1000.0,
            node_count: pruned.nodes.len(),
            edge_count: pruned.edges.len(),
        }
    }

    // -----------------------------------------------------------------------
    // Private: graph path
    // -----------------------------------------------------------------------

    /// Prune graph to max_nodes by edge-weight importance.
    pub(crate) fn prune_graph(&self, graph: &TrajectoryGraph) -> TrajectoryGraph {
        if graph.nodes.len() <= self.config.max_nodes {
            return graph.clone();
        }

        let mut node_scores: HashMap<String, f32> =
            graph.nodes.iter().map(|n| (n.id.clone(), 0.0)).collect();

        for edge in &graph.edges {
            *node_scores.entry(edge.source.clone()).or_insert(0.0) += edge.weight;
            *node_scores.entry(edge.target.clone()).or_insert(0.0) += edge.weight;
        }

        let mut sorted_nodes = graph.nodes.clone();
        sorted_nodes.sort_by(|a, b| {
            let sa = node_scores.get(&a.id).copied().unwrap_or(0.0);
            let sb = node_scores.get(&b.id).copied().unwrap_or(0.0);
            sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
        });

        let pruned_nodes: Vec<TrajectoryNode> = sorted_nodes
            .into_iter()
            .take(self.config.max_nodes)
            .collect();
        let node_set: HashSet<&str> = pruned_nodes.iter().map(|n| n.id.as_str()).collect();

        let pruned_edges: Vec<TrajectoryEdge> = graph
            .edges
            .iter()
            .filter(|e| {
                node_set.contains(e.source.as_str()) && node_set.contains(e.target.as_str())
            })
            .cloned()
            .collect();

        TrajectoryGraph {
            nodes: pruned_nodes,
            edges: pruned_edges,
        }
    }

    /// Build feature matrix from graph node embeddings.
    pub(super) fn build_feature_matrix(&self, graph: &TrajectoryGraph) -> Vec<Vec<f32>> {
        graph
            .nodes
            .iter()
            .map(|node| {
                if node.embedding.len() != self.config.input_dim {
                    let w = self.weights.get_weights("feature_projection");
                    project(&node.embedding, &w, self.config.input_dim)
                } else {
                    node.embedding.clone()
                }
            })
            .collect()
    }

    #[allow(clippy::needless_range_loop)]
    /// Build symmetric adjacency matrix from edges.
    pub(super) fn build_adjacency_matrix(&self, graph: &TrajectoryGraph) -> Vec<Vec<f32>> {
        let n = graph.nodes.len();
        let node_index: HashMap<&str, usize> = graph
            .nodes
            .iter()
            .enumerate()
            .map(|(i, node)| (node.id.as_str(), i))
            .collect();

        if graph.edges.is_empty() {
            // Fully-connected fallback with uniform weights
            let mut matrix = vec![vec![0.0; n]; n];
            for i in 0..n {
                for j in 0..n {
                    if i != j {
                        matrix[i][j] = 1.0 / (n - 1) as f32;
                    }
                }
            }
            return matrix;
        }

        let mut matrix = vec![vec![0.0; n]; n];
        for edge in &graph.edges {
            if let (Some(&si), Some(&ti)) = (
                node_index.get(edge.source.as_str()),
                node_index.get(edge.target.as_str()),
            ) {
                matrix[si][ti] = edge.weight;
                matrix[ti][si] = edge.weight; // undirected
            }
        }
        matrix
    }

    /// Aggregate neighborhood using graph attention (5 steps).
    pub(crate) fn aggregate_neighborhood(
        &self,
        center: &[f32],
        features: &[Vec<f32>],
        adjacency: &[Vec<f32>],
    ) -> Vec<f32> {
        if features.is_empty() {
            return center.to_vec();
        }

        let n = features.len();

        // Step 1: Node importance = sum of all edge weights
        let mut importance = vec![0.0f32; n];
        for i in 0..n {
            let mut total = 0.0;
            for j in 0..adjacency.len().min(n) {
                total += adjacency[i].get(j).copied().unwrap_or(0.0);
                total += adjacency
                    .get(j)
                    .and_then(|row| row.get(i))
                    .copied()
                    .unwrap_or(0.0);
            }
            importance[i] = total;
        }

        // Step 2: Raw scores = attention_score(center, node[j]) + log(importance[j] + 1)
        let mut raw_scores = Vec::with_capacity(n);
        for j in 0..n {
            let base = attention_score(center, &features[j], None);
            let bonus = (importance[j] + 1.0).ln();
            raw_scores.push(base + bonus);
        }

        // Step 3: Softmax
        let attention_weights = softmax(&raw_scores);

        // Step 4: Weighted aggregate
        let aggregated = weighted_aggregate(features, &attention_weights);

        // Step 5: center + aggregated, then normalize
        let result = add_vectors(center, &aggregated);
        normalize(&result)
    }
}

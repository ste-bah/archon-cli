//! GNN Enhancer — 3-layer graph attention network for embedding enhancement.
//!
//! Ported from root archon TS gnn-enhancer.ts. Round-trip architecture
//! preserves dimensionality: 1536→1024→1280→1536 with graph attention,
//! residual connections, and layer normalization.
//!
//! Layer architecture:
//!   input_projection: input_dim → input_dim (for non-standard input sizes)
//!   layer1: input_dim → 1024 (compress)
//!   layer2: 1024 → 1280 (expand)
//!   layer3: 1280 → output_dim (restore to 1536)

pub mod auto_trainer;
// Reference: auto_trainer_runtime.rs (build/spawn helpers used by session.rs + pipeline.rs)
pub mod auto_trainer_runtime;
pub mod backprop;
pub mod cache;
pub mod ewc;
pub mod history;
pub mod loss;
pub mod math;
pub mod optimizer;
pub mod trainer;
pub mod weights;

use crate::learning::gnn::cache::{CacheConfig, CacheStats, GnnCacheManager};
use crate::learning::gnn::math::{
    ActivationType, add_vectors, apply_activation, attention_score, normalize, project, softmax,
    weighted_aggregate, zero_pad,
};
use crate::learning::gnn::weights::{Initialization, WeightStore};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tracing::warn;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// GNN configuration matching root TS DEFAULT_GNN_CONFIG.
#[derive(Debug, Clone)]
pub struct GnnConfig {
    pub input_dim: usize,       // 1536
    pub output_dim: usize,      // 1536 (round-trip)
    pub num_layers: usize,      // 3
    pub attention_heads: usize, // 12
    pub dropout: f32,           // 0.1
    pub max_nodes: usize,       // 50
    pub use_residual: bool,     // true
    pub use_layer_norm: bool,   // true
    pub activation: ActivationType,
}

impl Default for GnnConfig {
    fn default() -> Self {
        Self {
            input_dim: 1536,
            output_dim: 1536,
            num_layers: 3,
            attention_heads: 12,
            dropout: 0.1,
            max_nodes: 50,
            use_residual: true,
            use_layer_norm: true,
            activation: ActivationType::Relu,
        }
    }
}

/// A node in a trajectory graph.
#[derive(Debug, Clone)]
pub struct TrajectoryNode {
    pub id: String,
    pub embedding: Vec<f32>,
}

/// An edge in a trajectory graph.
#[derive(Debug, Clone)]
pub struct TrajectoryEdge {
    pub source: String,
    pub target: String,
    pub weight: f32,
}

/// A trajectory graph for GNN enhancement.
#[derive(Debug, Clone, Default)]
pub struct TrajectoryGraph {
    pub nodes: Vec<TrajectoryNode>,
    pub edges: Vec<TrajectoryEdge>,
}

/// Cached activations from a single layer's forward pass (used for backprop).
#[derive(Debug, Clone)]
pub struct LayerActivationCache {
    pub layer_id: String,
    pub input: Vec<f32>,
    pub pre_activation: Vec<f32>,
    /// Output of the activation function (before residual / layer norm).
    pub true_post_activation: Vec<f32>,
    /// Final layer output (after residual + layer norm).
    pub post_activation: Vec<f32>,
    pub weights: Arc<Vec<Vec<f32>>>,
}

/// Result of a GNN forward pass.
#[derive(Debug, Clone)]
pub struct ForwardResult {
    pub enhanced: Vec<f32>,
    pub original: Vec<f32>,
    pub cached: bool,
    pub enhancement_time_ms: f64,
    pub node_count: Option<usize>,
    pub activation_cache: Vec<LayerActivationCache>,
}

/// Result of graph-based enhancement.
#[derive(Debug, Clone)]
pub struct GraphEnhancementResult {
    pub enhanced: Vec<f32>,
    pub processing_time_ms: f64,
    pub node_count: usize,
    pub edge_count: usize,
}

// ---------------------------------------------------------------------------
// Legacy types — retained for backward compatibility with PR 2 submodules
// ---------------------------------------------------------------------------

/// Weights for a single linear layer (legacy, used by backprop/optimizer/trainer).
/// Will be removed in PR 2 when those modules are replaced.
#[derive(Debug, Clone)]
pub struct LayerWeights {
    pub w: Vec<Vec<f32>>,
    pub bias: Vec<f32>,
}

impl LayerWeights {
    pub fn random(in_dim: usize, out_dim: usize) -> Self {
        use rand::Rng;
        let mut rng = rand::rng();
        let scale = (2.0 / (in_dim + out_dim) as f32).sqrt();
        let w: Vec<Vec<f32>> = (0..out_dim)
            .map(|_| {
                (0..in_dim)
                    .map(|_| rng.random_range(-scale..scale))
                    .collect()
            })
            .collect();
        let bias = vec![0.0; out_dim];
        Self { w, bias }
    }

    pub fn zeros(in_dim: usize, out_dim: usize) -> Self {
        Self {
            w: vec![vec![0.0; in_dim]; out_dim],
            bias: vec![0.0; out_dim],
        }
    }
}

/// Legacy type alias for backward compatibility with trainer.rs.
#[allow(non_camel_case_types)]
pub type GNNEnhancer = GnnEnhancer;

// Legacy methods on GnnEnhancer for backward compatibility with PR 2 submodules.
// These will be removed when trainer.rs, optimizer.rs, and backprop.rs are replaced in PR 2.
impl GnnEnhancer {
    /// Legacy: get weights as LayerWeights tuple (used by trainer.rs).
    pub fn get_weights(&self) -> (LayerWeights, LayerWeights, LayerWeights) {
        let w1 = self.weights.get_weights("layer1");
        let w2 = self.weights.get_weights("layer2");
        let w3 = self.weights.get_weights("layer3");
        let b1 = self.weights.get_bias("layer1");
        let b2 = self.weights.get_bias("layer2");
        let b3 = self.weights.get_bias("layer3");
        (
            LayerWeights {
                w: (*w1).clone(),
                bias: (*b1).clone(),
            },
            LayerWeights {
                w: (*w2).clone(),
                bias: (*b2).clone(),
            },
            LayerWeights {
                w: (*w3).clone(),
                bias: (*b3).clone(),
            },
        )
    }

    /// Legacy: set weights from LayerWeights tuple (used by trainer.rs).
    pub fn set_weights(&self, l1: LayerWeights, l2: LayerWeights, l3: LayerWeights) {
        self.weights.set_weights("layer1", l1.w, l1.bias);
        self.weights.set_weights("layer2", l2.w, l2.bias);
        self.weights.set_weights("layer3", l3.w, l3.bias);
    }

    /// Legacy: 1-arg enhance for backward compat with trainer.rs validate().
    /// Redirects to the new 4-arg signature.
    pub fn enhance_legacy(&self, embedding: &[f32]) -> ForwardResult {
        self.enhance(embedding, None, None, false)
    }
}

// ---------------------------------------------------------------------------
// GnnEnhancer
// ---------------------------------------------------------------------------

/// 3-layer graph attention network for embedding enhancement.
///
/// Uses graph attention (scaled dot-product + adjacency-weighted softmax),
/// residual connections, and layer normalization. Matches root TS behaviour.
pub struct GnnEnhancer {
    config: GnnConfig,
    cache: GnnCacheManager,
    weights: WeightStore,
    weight_seed: u64,
    weights_loaded: AtomicBool,
    total_enhancements: AtomicU64,
    total_cache_hits: AtomicU64,
    total_enhancement_micros: AtomicU64,
}

impl GnnEnhancer {
    /// Create a new GnnEnhancer with the given config, cache config, seed, and weight store.
    pub fn new(
        config: GnnConfig,
        cache_config: CacheConfig,
        weight_seed: u64,
        weights: WeightStore,
    ) -> Self {
        let enhancer = Self {
            config,
            cache: GnnCacheManager::new(cache_config),
            weights,
            weight_seed,
            weights_loaded: AtomicBool::new(false),
            total_enhancements: AtomicU64::new(0),
            total_cache_hits: AtomicU64::new(0),
            total_enhancement_micros: AtomicU64::new(0),
        };
        enhancer.initialize_layer_weights();
        enhancer
    }

    /// Create a GnnEnhancer with in-memory weights (for tests that don't need persistence).
    pub fn with_in_memory_weights(
        config: GnnConfig,
        cache_config: CacheConfig,
        weight_seed: u64,
    ) -> Self {
        Self::new(
            config,
            cache_config,
            weight_seed,
            WeightStore::with_in_memory(),
        )
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Enhance an embedding through the GNN.
    ///
    /// If `collect_activations` is true, captures per-layer pre/post activation
    /// values for backprop (unused in PR 1, wired for PR 2).
    pub fn enhance(
        &self,
        embedding: &[f32],
        graph: Option<&TrajectoryGraph>,
        hyperedges: Option<&[String]>,
        collect_activations: bool,
    ) -> ForwardResult {
        let start = std::time::Instant::now();
        self.total_enhancements.fetch_add(1, Ordering::Relaxed);
        let original = embedding.to_vec();
        let mut activation_cache = Vec::new();

        // Graph-based enhancement path
        if let Some(g) = graph {
            let graph_result = self.enhance_with_graph(embedding, g);
            return ForwardResult {
                enhanced: graph_result.enhanced,
                original,
                cached: false,
                enhancement_time_ms: graph_result.processing_time_ms,
                node_count: Some(graph_result.node_count),
                activation_cache: vec![],
            };
        }

        // Check cache (skip when collecting activations — cache doesn't store them)
        if !collect_activations {
            let hyperedges_slice = hyperedges.unwrap_or(&[]);
            let cache_key = self.cache.smart_cache_key(embedding, hyperedges_slice);
            if let Some(cached) = self.cache.get(&cache_key) {
                self.total_cache_hits.fetch_add(1, Ordering::Relaxed);
                let elapsed = start.elapsed().as_micros() as f64 / 1000.0;
                self.total_enhancement_micros
                    .fetch_add(start.elapsed().as_micros() as u64, Ordering::Relaxed);
                return ForwardResult {
                    enhanced: cached.embedding,
                    original,
                    cached: true,
                    enhancement_time_ms: elapsed,
                    node_count: None,
                    activation_cache: vec![],
                };
            }
        }

        // Forward pass
        let enhanced =
            match self.forward_pass(embedding, collect_activations, &mut activation_cache) {
                Ok(result) => result,
                Err(_) => {
                    let elapsed = start.elapsed().as_micros() as f64 / 1000.0;
                    return ForwardResult {
                        enhanced: zero_pad(embedding, self.config.output_dim),
                        original,
                        cached: false,
                        enhancement_time_ms: elapsed,
                        node_count: None,
                        activation_cache: vec![],
                    };
                }
            };

        // Cache the result
        if !collect_activations {
            let hyperedges_slice = hyperedges.unwrap_or(&[]);
            let cache_key = self.cache.smart_cache_key(embedding, hyperedges_slice);
            let he_hash = if hyperedges_slice.is_empty() {
                "none".to_string()
            } else {
                hyperedges_slice.join(":")
            };
            self.cache.put(cache_key, enhanced.clone(), he_hash);
        }

        let elapsed = start.elapsed().as_micros() as f64 / 1000.0;
        self.total_enhancement_micros
            .fetch_add(start.elapsed().as_micros() as u64, Ordering::Relaxed);

        ForwardResult {
            enhanced,
            original,
            cached: false,
            enhancement_time_ms: elapsed,
            node_count: None,
            activation_cache,
        }
    }

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

    /// Get cache statistics.
    pub fn cache_stats(&self) -> CacheStats {
        self.cache.stats()
    }

    /// Invalidate cache entries for specific node IDs.
    pub fn invalidate_nodes(&self, node_ids: &[String]) -> usize {
        self.cache.invalidate_nodes(node_ids)
    }

    /// Invalidate all cache entries.
    pub fn invalidate_all(&self) -> usize {
        self.cache.invalidate_all()
    }

    /// Clear cache and reset metrics.
    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    /// Get total enhancements count.
    pub fn total_enhancements(&self) -> u64 {
        self.total_enhancements.load(Ordering::Relaxed)
    }

    /// Get total cache hits.
    pub fn total_cache_hits(&self) -> u64 {
        self.total_cache_hits.load(Ordering::Relaxed)
    }

    /// Reset metrics counters.
    pub fn reset_metrics(&self) {
        self.total_enhancements.store(0, Ordering::Relaxed);
        self.total_cache_hits.store(0, Ordering::Relaxed);
        self.total_enhancement_micros.store(0, Ordering::Relaxed);
        self.cache.reset_metrics();
    }

    /// Get the weight seed used for initialization.
    pub fn weight_seed(&self) -> u64 {
        self.weight_seed
    }

    /// Get reference to the weight store.
    pub fn weight_store(&self) -> &WeightStore {
        &self.weights
    }

    /// Get the GNN config.
    pub fn config(&self) -> &GnnConfig {
        &self.config
    }

    // -----------------------------------------------------------------------
    // Private: forward pass
    // -----------------------------------------------------------------------

    /// Run the 3-layer forward pass. Returns L2-normalized output.
    fn forward_pass(
        &self,
        embedding: &[f32],
        collect_activations: bool,
        activation_cache: &mut Vec<LayerActivationCache>,
    ) -> Result<Vec<f32>, ()> {
        let normalized = self.prepare_input(embedding);
        let mut current = normalized;

        let intermediate_dim1 = 1536 * 2 / 3; // 1024
        let intermediate_dim2 = 1536 * 5 / 6; // 1280

        if collect_activations {
            let (out, cache) = self.apply_layer_with_cache(&current, intermediate_dim1, 1);
            activation_cache.push(cache);
            current = out;

            let (out, cache) = self.apply_layer_with_cache(&current, intermediate_dim2, 2);
            activation_cache.push(cache);
            current = out;

            let (out, cache) = self.apply_layer_with_cache(&current, self.config.output_dim, 3);
            activation_cache.push(cache);
            current = out;
        } else {
            current = self.apply_layer(&current, intermediate_dim1, 1);
            current = self.apply_layer(&current, intermediate_dim2, 2);
            current = self.apply_layer(&current, self.config.output_dim, 3);
        }

        // NaN check
        if current.iter().any(|v| v.is_nan() || v.is_infinite()) {
            warn!("NaN/Inf detected in GNN forward pass, falling back");
            return Err(());
        }

        Ok(normalize(&current))
    }

    /// Prepare input: project to input_dim if needed, then normalize.
    fn prepare_input(&self, embedding: &[f32]) -> Vec<f32> {
        let prepared = if embedding.len() != self.config.input_dim {
            let w = self.weights.get_weights("input_projection");
            project(embedding, &w, self.config.input_dim)
        } else {
            embedding.to_vec()
        };
        normalize(&prepared)
    }

    /// Apply a single GNN layer.
    fn apply_layer(&self, input: &[f32], output_dim: usize, layer_num: usize) -> Vec<f32> {
        let layer_id = format!("layer{}", layer_num);
        let weights = self.weights.get_weights(&layer_id);

        // Project
        let mut output = project(input, &weights, output_dim);

        // Activation
        output = apply_activation(&output, self.config.activation);

        // Residual connection
        if self.config.use_residual && input.len() == output.len() {
            output = add_vectors(&output, input);
            output = normalize(&output);
        }

        // Layer norm
        if self.config.use_layer_norm {
            output = normalize(&output);
        }

        output
    }

    /// Apply a single GNN layer and capture activations for backprop.
    fn apply_layer_with_cache(
        &self,
        input: &[f32],
        output_dim: usize,
        layer_num: usize,
    ) -> (Vec<f32>, LayerActivationCache) {
        let layer_id = format!("layer{}", layer_num);
        let weights = self.weights.get_weights(&layer_id);

        // Pre-activation
        let pre_activation = project(input, &weights, output_dim);

        // Post-activation
        let post_activation = apply_activation(&pre_activation, self.config.activation);
        let true_post_activation = post_activation.clone();

        // Residual + layer norm
        let mut output = post_activation;
        if self.config.use_residual && input.len() == output.len() {
            output = add_vectors(&output, input);
            output = normalize(&output);
        }
        if self.config.use_layer_norm {
            output = normalize(&output);
        }

        let cache = LayerActivationCache {
            layer_id,
            input: input.to_vec(),
            pre_activation,
            true_post_activation,
            post_activation: output.clone(),
            weights: Arc::clone(&weights),
        };

        (output, cache)
    }

    // -----------------------------------------------------------------------
    // Private: graph path
    // -----------------------------------------------------------------------

    /// Prune graph to max_nodes by edge-weight importance.
    fn prune_graph(&self, graph: &TrajectoryGraph) -> TrajectoryGraph {
        if graph.nodes.len() <= self.config.max_nodes {
            return graph.clone();
        }

        let mut node_scores: std::collections::HashMap<String, f32> =
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
        let node_set: std::collections::HashSet<&str> =
            pruned_nodes.iter().map(|n| n.id.as_str()).collect();

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
    fn build_feature_matrix(&self, graph: &TrajectoryGraph) -> Vec<Vec<f32>> {
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
    fn build_adjacency_matrix(&self, graph: &TrajectoryGraph) -> Vec<Vec<f32>> {
        let n = graph.nodes.len();
        let node_index: std::collections::HashMap<&str, usize> = graph
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
    fn aggregate_neighborhood(
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

    // -----------------------------------------------------------------------
    // Private: weight initialization
    // -----------------------------------------------------------------------

    fn initialize_layer_weights(&self) {
        let init = match self.config.activation {
            ActivationType::Relu | ActivationType::LeakyRelu => Initialization::He,
            ActivationType::Tanh | ActivationType::Sigmoid => Initialization::Xavier,
        };

        // Intermediate dimensions
        let intermediate_dim1 = 1536 * 2 / 3; // 1024
        let intermediate_dim2 = 1536 * 5 / 6; // 1280

        let layers: &[(&str, usize, usize)] = &[
            (
                "input_projection",
                self.config.input_dim,
                self.config.input_dim,
            ),
            ("layer1", self.config.input_dim, intermediate_dim1),
            ("layer2", intermediate_dim1, intermediate_dim2),
            ("layer3", intermediate_dim2, self.config.output_dim),
            (
                "feature_projection",
                self.config.input_dim,
                self.config.input_dim,
            ),
        ];

        for (i, &(id, in_dim, out_dim)) in layers.iter().enumerate() {
            self.weights
                .initialize(id, in_dim, out_dim, init, self.weight_seed + i as u64);
        }

        self.weights_loaded.store(true, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> GnnConfig {
        GnnConfig::default()
    }

    fn test_cache_config() -> CacheConfig {
        CacheConfig::default()
    }

    #[test]
    fn test_forward_pass_dimensions() {
        let enhancer = GnnEnhancer::with_in_memory_weights(test_config(), test_cache_config(), 42);
        let input = vec![0.5f32; 1536];
        let result = enhancer.enhance(&input, None, None, false);
        assert_eq!(
            result.enhanced.len(),
            1536,
            "Output should be 1536D (round-trip)"
        );
        assert_eq!(result.original.len(), 1536, "Original preserved");
        assert!(!result.cached, "First call should not be cached");
    }

    #[test]
    fn test_forward_pass_short_input() {
        let enhancer = GnnEnhancer::with_in_memory_weights(test_config(), test_cache_config(), 42);
        let input = vec![1.0f32; 768];
        let result = enhancer.enhance(&input, None, None, false);
        assert_eq!(
            result.enhanced.len(),
            1536,
            "Output should still be 1536D via input_projection"
        );
    }

    #[test]
    fn test_nan_handling_falls_back() {
        // Create a weight store with NaN-producing weights
        let store = WeightStore::with_in_memory();
        // Override layer1 with huge weights that will produce Inf/NaN through relu
        let huge: Vec<Vec<f32>> = vec![vec![f32::MAX; 1536]; 1024];
        store.set_weights("layer1", huge.clone(), vec![0.0; 1024]);
        store.set_weights("layer2", huge.clone(), vec![0.0; 1024]);
        store.set_weights("layer3", huge.clone(), vec![0.0; 1024]);
        store.initialize("input_projection", 1536, 1536, Initialization::Xavier, 42);
        store.initialize("feature_projection", 1536, 1536, Initialization::Xavier, 42);

        let enhancer = GnnEnhancer::new(test_config(), test_cache_config(), 42, store);
        let input = vec![1.0f32; 1536];
        let result = enhancer.enhance(&input, None, None, false);
        assert_eq!(result.enhanced.len(), 1536);
        assert!(
            !result
                .enhanced
                .iter()
                .any(|v| v.is_nan() || v.is_infinite()),
            "Output must not contain NaN or Inf"
        );
    }

    #[test]
    fn test_cache_hit() {
        let enhancer = GnnEnhancer::with_in_memory_weights(test_config(), test_cache_config(), 42);
        let input = vec![0.42f32; 1536];

        let r1 = enhancer.enhance(&input, None, None, false);
        assert!(!r1.cached, "First call should miss cache");

        let r2 = enhancer.enhance(&input, None, None, false);
        assert!(r2.cached, "Second call should hit cache");
    }

    #[test]
    fn test_different_seeds_produce_different_outputs() {
        let e1 = GnnEnhancer::with_in_memory_weights(test_config(), test_cache_config(), 42);
        let e2 = GnnEnhancer::with_in_memory_weights(test_config(), test_cache_config(), 99);

        let input = vec![0.5f32; 1536];
        let r1 = e1.enhance(&input, None, None, false);
        let r2 = e2.enhance(&input, None, None, false);

        // Outputs should differ (different weight seeds)
        let diff = r1
            .enhanced
            .iter()
            .zip(r2.enhanced.iter())
            .any(|(a, b)| (a - b).abs() > 1e-6);
        assert!(diff, "Different seeds should produce different outputs");
    }

    #[test]
    fn test_graph_enhancement_basic() {
        let enhancer = GnnEnhancer::with_in_memory_weights(test_config(), test_cache_config(), 42);

        let graph = TrajectoryGraph {
            nodes: vec![
                TrajectoryNode {
                    id: "A".into(),
                    embedding: vec![0.1; 1536],
                },
                TrajectoryNode {
                    id: "B".into(),
                    embedding: vec![0.2; 1536],
                },
            ],
            edges: vec![TrajectoryEdge {
                source: "A".into(),
                target: "B".into(),
                weight: 0.5,
            }],
        };

        let center = vec![0.1; 1536];
        let result = enhancer.enhance_with_graph(&center, &graph);
        assert_eq!(result.node_count, 2);
        assert_eq!(result.edge_count, 1);
        assert_eq!(result.enhanced.len(), 1536);
    }

    #[test]
    fn test_prune_graph_respects_max_nodes() {
        let mut cfg = test_config();
        cfg.max_nodes = 2;
        let enhancer = GnnEnhancer::with_in_memory_weights(cfg, test_cache_config(), 42);

        let graph = TrajectoryGraph {
            nodes: (0..5)
                .map(|i| TrajectoryNode {
                    id: format!("N{}", i),
                    embedding: vec![0.0; 1536],
                })
                .collect(),
            edges: vec![
                TrajectoryEdge {
                    source: "N0".into(),
                    target: "N1".into(),
                    weight: 0.9,
                },
                TrajectoryEdge {
                    source: "N1".into(),
                    target: "N2".into(),
                    weight: 0.1,
                },
            ],
        };

        let pruned = enhancer.prune_graph(&graph);
        assert!(pruned.nodes.len() <= 2);
    }

    #[test]
    fn test_aggregate_neighborhood_no_nan() {
        let enhancer = GnnEnhancer::with_in_memory_weights(test_config(), test_cache_config(), 42);

        let center = vec![0.5; 4];
        let features = vec![vec![0.5; 4], vec![0.1; 4], vec![0.9; 4]];
        let adjacency = vec![
            vec![0.0, 0.9, 0.1],
            vec![0.9, 0.0, 0.0],
            vec![0.1, 0.0, 0.0],
        ];

        let agg = enhancer.aggregate_neighborhood(&center, &features, &adjacency);
        assert_eq!(agg.len(), 4);
        assert!(!agg.iter().any(|v| v.is_nan() || v.is_infinite()));
    }

    #[test]
    fn test_activation_cache_when_collecting() {
        let enhancer = GnnEnhancer::with_in_memory_weights(test_config(), test_cache_config(), 42);
        let input = vec![0.5; 1536];
        let result = enhancer.enhance(&input, None, None, true);
        assert_eq!(
            result.activation_cache.len(),
            3,
            "Should have 3 layer caches"
        );
        assert_eq!(result.activation_cache[0].layer_id, "layer1");
        assert_eq!(result.activation_cache[1].layer_id, "layer2");
        assert_eq!(result.activation_cache[2].layer_id, "layer3");
    }

    #[test]
    fn test_enhanced_embedding_is_normalized() {
        let enhancer = GnnEnhancer::with_in_memory_weights(test_config(), test_cache_config(), 42);
        let input = vec![0.5; 1536];
        let result = enhancer.enhance(&input, None, None, false);
        let norm: f32 = result.enhanced.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 0.01,
            "Output should be L2-normalized, got {}",
            norm
        );
    }

    #[test]
    fn test_softmax_sums_to_one() {
        let scores = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let sm = math::softmax(&scores);
        let sum: f32 = sm.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_activation_types() {
        let input = vec![-2.0, -1.0, 0.0, 1.0, 2.0];
        assert_eq!(
            apply_activation(&input, ActivationType::Relu),
            vec![0.0, 0.0, 0.0, 1.0, 2.0]
        );
        let lrelu = apply_activation(&input, ActivationType::LeakyRelu);
        assert!((lrelu[0] - (-0.02)).abs() < 1e-6);
        assert!((apply_activation(&[0.0], ActivationType::Tanh)[0] - 0.0).abs() < 1e-6);
        assert!((apply_activation(&[0.0], ActivationType::Sigmoid)[0] - 0.5).abs() < 1e-6);
    }
}

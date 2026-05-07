use std::sync::Arc;

use super::math::ActivationType;

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

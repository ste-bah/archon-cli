//! GNN attention tests — verify graph attention path correctness.

use archon_pipeline::learning::gnn::cache::CacheConfig;
use archon_pipeline::learning::gnn::math::cosine_similarity;
use archon_pipeline::learning::gnn::{
    GnnConfig, GnnEnhancer, TrajectoryEdge, TrajectoryGraph, TrajectoryNode,
};

fn enhancer(seed: u64) -> GnnEnhancer {
    GnnEnhancer::with_in_memory_weights(GnnConfig::default(), CacheConfig::default(), seed)
}

/// Build a 3-node graph: A<->B (weight 0.9), B<->C (weight 0.1).
/// Embeddings use distinct directions so cosine similarity is meaningful.
fn triangle_graph() -> (Vec<f32>, TrajectoryGraph) {
    // A/center: alternating +1/-1 — clear directional signal
    let center: Vec<f32> = (0..1536)
        .map(|i| if i % 2 == 0 { 1.0_f32 } else { -1.0_f32 })
        .collect();
    // B (strong edge 0.9): similar to center but perturbed
    let emb_b: Vec<f32> = center.iter().map(|v| v * 1.1 + 0.05).collect();
    // C (weak edge 0.1): opposite direction from center
    let emb_c: Vec<f32> = center.iter().map(|v| -v).collect();
    let graph = TrajectoryGraph {
        nodes: vec![
            TrajectoryNode {
                id: "A".into(),
                embedding: center.clone(),
            },
            TrajectoryNode {
                id: "B".into(),
                embedding: emb_b,
            },
            TrajectoryNode {
                id: "C".into(),
                embedding: emb_c,
            },
        ],
        edges: vec![
            TrajectoryEdge {
                source: "A".into(),
                target: "B".into(),
                weight: 0.9,
            },
            TrajectoryEdge {
                source: "B".into(),
                target: "C".into(),
                weight: 0.1,
            },
        ],
    };
    (center, graph)
}

#[test]
fn enhance_with_graph_closer_to_ab_than_c() {
    let gnn = enhancer(42);
    let (center, graph) = triangle_graph();

    let result = gnn.enhance_with_graph(&center, &graph);

    let sim_a = cosine_similarity(&result.enhanced, &center);
    let sim_b = cosine_similarity(&result.enhanced, &graph.nodes[1].embedding);
    let sim_c = cosine_similarity(&result.enhanced, &graph.nodes[2].embedding);

    // Enhanced result should be more similar to A and B (strongly connected)
    // than to C (weakly connected)
    assert!(
        sim_a > sim_c || sim_b > sim_c,
        "Enhanced embedding should be closer to strongly-connected nodes (A/B) than weakly-connected node (C). sim_a={:.4} sim_b={:.4} sim_c={:.4}",
        sim_a,
        sim_b,
        sim_c
    );
}

#[test]
fn empty_graph_returns_input_unchanged_dimension() {
    let gnn = enhancer(42);
    let graph = TrajectoryGraph {
        nodes: vec![],
        edges: vec![],
    };
    let center = vec![0.5; 1536];

    let result = gnn.enhance_with_graph(&center, &graph);
    assert_eq!(result.enhanced.len(), 1536);
    assert!(!result.enhanced.iter().any(|v| v.is_nan()));
}

#[test]
fn single_node_graph_no_nan() {
    let gnn = enhancer(42);
    let graph = TrajectoryGraph {
        nodes: vec![TrajectoryNode {
            id: "only".into(),
            embedding: vec![0.3; 1536],
        }],
        edges: vec![],
    };
    let center = vec![0.5; 1536];

    let result = gnn.enhance_with_graph(&center, &graph);
    assert!(
        !result
            .enhanced
            .iter()
            .any(|v| v.is_nan() || v.is_infinite())
    );
}

#[test]
fn graph_enhancement_has_correct_metadata() {
    let gnn = enhancer(42);
    let (center, graph) = triangle_graph();

    let result = gnn.enhance_with_graph(&center, &graph);
    assert_eq!(result.node_count, 3);
    assert_eq!(result.edge_count, 2);
}

#[test]
fn softmax_weights_sum_to_one() {
    use archon_pipeline::learning::gnn::math::softmax;
    let scores = vec![1.0, 2.0, 3.0, 4.0, 5.0];
    let sm = softmax(&scores);
    let sum: f32 = sm.iter().sum();
    assert!(
        (sum - 1.0).abs() < 1e-5,
        "Softmax should sum to 1.0, got {}",
        sum
    );
}

#[test]
fn enhance_collects_no_cache_by_default() {
    let gnn = enhancer(42);
    let input = vec![0.5; 1536];
    let result = gnn.enhance(&input, None, None, false);
    assert!(result.activation_cache.is_empty());
}

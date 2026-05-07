use super::cache::CacheConfig;
use super::math::{self, ActivationType, apply_activation};
use super::weights::{Initialization, WeightStore};
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

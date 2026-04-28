//! GNN dimension tests — verify correct output shapes across input sizes.

use archon_pipeline::learning::gnn::cache::CacheConfig;
use archon_pipeline::learning::gnn::{GnnConfig, GnnEnhancer};

fn enhancer(seed: u64) -> GnnEnhancer {
    GnnEnhancer::with_in_memory_weights(GnnConfig::default(), CacheConfig::default(), seed)
}

#[test]
fn enhance_input_1536d_returns_1536d() {
    let gnn = enhancer(42);
    let input = vec![0.5f32; 1536];
    let result = gnn.enhance(&input, None, None, false);
    assert_eq!(result.enhanced.len(), 1536);
    assert_eq!(result.original.len(), 1536);
}

#[test]
fn enhance_input_768d_projects_to_1536d() {
    let gnn = enhancer(42);
    let input = vec![0.5f32; 768];
    let result = gnn.enhance(&input, None, None, false);
    assert_eq!(result.enhanced.len(), 1536);
}

#[test]
fn enhance_input_2000d_truncates_to_1536d() {
    let gnn = enhancer(42);
    let input = vec![0.1f32; 2000];
    let result = gnn.enhance(&input, None, None, false);
    assert_eq!(result.enhanced.len(), 1536);
}

#[test]
fn layer_cache_shapes_when_collecting_activations() {
    let gnn = enhancer(42);
    let input = vec![0.5f32; 1536];
    let result = gnn.enhance(&input, None, None, true);

    assert_eq!(result.activation_cache.len(), 3);

    // Layer 1: 1536 -> 1024
    assert_eq!(result.activation_cache[0].layer_id, "layer1");
    assert_eq!(result.activation_cache[0].input.len(), 1536);
    assert_eq!(result.activation_cache[0].pre_activation.len(), 1024);

    // Layer 2: 1024 -> 1280
    assert_eq!(result.activation_cache[1].layer_id, "layer2");
    assert_eq!(result.activation_cache[1].input.len(), 1024);
    assert_eq!(result.activation_cache[1].pre_activation.len(), 1280);

    // Layer 3: 1280 -> 1536
    assert_eq!(result.activation_cache[2].layer_id, "layer3");
    assert_eq!(result.activation_cache[2].input.len(), 1280);
    assert_eq!(result.activation_cache[2].pre_activation.len(), 1536);
}

#[test]
fn output_is_l2_normalized() {
    let gnn = enhancer(42);
    let input = vec![0.5f32; 1536];
    let result = gnn.enhance(&input, None, None, false);
    let norm: f32 = result.enhanced.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!(
        (norm - 1.0).abs() < 0.01,
        "Output L2 norm should be ~1.0, got {}",
        norm
    );
}

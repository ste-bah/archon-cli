use std::sync::Arc;
use std::sync::atomic::Ordering;

use tracing::warn;

use super::GnnEnhancer;
use super::math::{add_vectors, apply_activation, normalize, project, zero_pad};
use super::types::{ForwardResult, LayerActivationCache, TrajectoryGraph};

impl GnnEnhancer {
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
}

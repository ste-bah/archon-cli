//! Adam optimizer for GNN weight updates.

use super::LayerWeights;

/// Configuration for the Adam optimizer.
#[derive(Debug, Clone)]
pub struct AdamConfig {
    pub learning_rate: f32,
    pub beta1: f32,
    pub beta2: f32,
    pub epsilon: f32,
    pub weight_decay: f32,
}

impl Default for AdamConfig {
    fn default() -> Self {
        Self {
            learning_rate: 0.001,
            beta1: 0.9,
            beta2: 0.999,
            epsilon: 1e-8,
            weight_decay: 0.0,
        }
    }
}

/// Per-parameter Adam state (first and second moments).
struct ParamState {
    m: Vec<Vec<f32>>, // First moment (mean of gradients)
    v: Vec<Vec<f32>>, // Second moment (mean of squared gradients)
    m_bias: Vec<f32>,
    v_bias: Vec<f32>,
}

impl ParamState {
    fn zeros(out_dim: usize, in_dim: usize) -> Self {
        Self {
            m: vec![vec![0.0; in_dim]; out_dim],
            v: vec![vec![0.0; in_dim]; out_dim],
            m_bias: vec![0.0; out_dim],
            v_bias: vec![0.0; out_dim],
        }
    }
}

/// Minimum value for bias-corrected second moment to prevent division by zero.
const MIN_V_HAT: f32 = 1e-10;

/// Adam optimizer maintaining per-layer state.
pub struct AdamOptimizer {
    config: AdamConfig,
    states: Vec<ParamState>,
    step_count: u64,
}

impl AdamOptimizer {
    /// Create a new Adam optimizer for 3 layers.
    pub fn new(config: AdamConfig, layers: &[&LayerWeights]) -> Self {
        let states = layers
            .iter()
            .map(|l| ParamState::zeros(l.w.len(), l.w.first().map(|r| r.len()).unwrap_or(0)))
            .collect();
        Self {
            config,
            states,
            step_count: 0,
        }
    }

    /// Perform one Adam update step given gradients for each layer.
    ///
    /// `grads` is a slice of (dw, db) tuples, one per layer.
    pub fn step(&mut self, layers: &mut [LayerWeights], grads: &[(Vec<Vec<f32>>, Vec<f32>)]) {
        self.step_count += 1;
        let t = self.step_count as f32;
        let beta1 = self.config.beta1;
        let beta2 = self.config.beta2;
        let lr = self.config.learning_rate;
        let eps = self.config.epsilon;
        let wd = self.config.weight_decay;

        // Bias correction factors
        let bc1 = 1.0 - beta1.powf(t);
        let bc2 = 1.0 - beta2.powf(t);

        for (idx, (layer, (dw, db))) in layers.iter_mut().zip(grads.iter()).enumerate() {
            let state = &mut self.states[idx];

            // Update weights
            for (i, row) in layer.w.iter_mut().enumerate() {
                for (j, w) in row.iter_mut().enumerate() {
                    let g = dw[i][j] + wd * *w;

                    // NaN/Inf guard: skip parameter if gradient is not finite
                    if !g.is_finite() {
                        continue;
                    }

                    state.m[i][j] = beta1 * state.m[i][j] + (1.0 - beta1) * g;
                    state.v[i][j] = beta2 * state.v[i][j] + (1.0 - beta2) * g * g;
                    let m_hat = state.m[i][j] / bc1;
                    let v_hat = (state.v[i][j] / bc2).max(MIN_V_HAT);

                    let new_w = *w - lr * m_hat / (v_hat.sqrt() + eps);

                    // NaN guard on output: keep original weight if result is not finite
                    if new_w.is_finite() {
                        *w = new_w;
                    }
                }
            }

            // Update biases
            for (i, b) in layer.bias.iter_mut().enumerate() {
                let g = db[i];

                // NaN/Inf guard
                if !g.is_finite() {
                    continue;
                }

                state.m_bias[i] = beta1 * state.m_bias[i] + (1.0 - beta1) * g;
                state.v_bias[i] = beta2 * state.v_bias[i] + (1.0 - beta2) * g * g;
                let m_hat = state.m_bias[i] / bc1;
                let v_hat = (state.v_bias[i] / bc2).max(MIN_V_HAT);

                let new_b = *b - lr * m_hat / (v_hat.sqrt() + eps);

                if new_b.is_finite() {
                    *b = new_b;
                }
            }
        }
    }

    /// Get the current step count.
    pub fn step_count(&self) -> u64 {
        self.step_count
    }

    /// Get the current learning rate.
    pub fn learning_rate(&self) -> f32 {
        self.config.learning_rate
    }

    /// Set the learning rate (for LR scheduling).
    pub fn set_learning_rate(&mut self, lr: f32) {
        self.config.learning_rate = lr;
    }

    /// Reset optimizer state — clears all moment estimates and resets step count.
    pub fn reset(&mut self) {
        for state in &mut self.states {
            for row in &mut state.m {
                row.fill(0.0);
            }
            for row in &mut state.v {
                row.fill(0.0);
            }
            state.m_bias.fill(0.0);
            state.v_bias.fill(0.0);
        }
        self.step_count = 0;
    }

    /// Flatten all layer weights into a single vector.
    pub fn flatten_weights(layers: &[LayerWeights]) -> Vec<f32> {
        let mut flat = Vec::new();
        for layer in layers {
            for row in &layer.w {
                flat.extend(row);
            }
            flat.extend(&layer.bias);
        }
        flat
    }

    /// Unflatten a vector back into layer weights.
    /// `shapes` is a slice of (out_dim, in_dim) tuples for each layer.
    pub fn unflatten_weights(flat: &[f32], shapes: &[(usize, usize)]) -> Vec<LayerWeights> {
        let mut offset = 0;
        let mut layers = Vec::new();
        for &(out_dim, in_dim) in shapes {
            let w: Vec<Vec<f32>> = (0..out_dim)
                .map(|_| {
                    let row = flat[offset..offset + in_dim].to_vec();
                    offset += in_dim;
                    row
                })
                .collect();
            let bias = flat[offset..offset + out_dim].to_vec();
            offset += out_dim;
            layers.push(LayerWeights { w, bias });
        }
        layers
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_weights() -> Vec<LayerWeights> {
        vec![LayerWeights {
            w: vec![vec![0.1; 4]; 3],
            bias: vec![0.01; 3],
        }]
    }

    fn simple_grads() -> Vec<(Vec<Vec<f32>>, Vec<f32>)> {
        vec![(vec![vec![0.01; 4]; 3], vec![0.001; 3])]
    }

    #[test]
    fn test_adam_step_moves_weights() {
        let config = AdamConfig::default();
        let sw = simple_weights();
        let layers_ref: Vec<&LayerWeights> = sw.iter().collect();
        let mut opt = AdamOptimizer::new(config, &layers_ref);

        let mut layers = simple_weights();
        let grads = simple_grads();
        let original = layers[0].w[0][0];

        opt.step(&mut layers, &grads);
        assert_ne!(
            layers[0].w[0][0], original,
            "Weights should change after Adam step"
        );
        assert_eq!(opt.step_count(), 1);
    }

    #[test]
    fn test_nan_gradient_skipped() {
        let config = AdamConfig::default();
        let sw = simple_weights();
        let layers_ref: Vec<&LayerWeights> = sw.iter().collect();
        let mut opt = AdamOptimizer::new(config, &layers_ref);

        let mut layers = simple_weights();
        let dw_nan = vec![vec![f32::NAN; 4]; 3];
        let grads = vec![(dw_nan, vec![0.001; 3])];
        let original_w = layers[0].w.clone();

        opt.step(&mut layers, &grads);
        assert_eq!(
            layers[0].w, original_w,
            "NaN gradients should not modify weights"
        );
    }

    #[test]
    fn test_inf_gradient_skipped() {
        let config = AdamConfig::default();
        let sw = simple_weights();
        let layers_ref: Vec<&LayerWeights> = sw.iter().collect();
        let mut opt = AdamOptimizer::new(config, &layers_ref);

        let mut layers = simple_weights();
        let dw_inf = vec![vec![f32::INFINITY; 4]; 3];
        let grads = vec![(dw_inf, vec![0.001; 3])];
        let original_w = layers[0].w.clone();

        opt.step(&mut layers, &grads);
        assert_eq!(
            layers[0].w, original_w,
            "Inf gradients should not modify weights"
        );
    }

    #[test]
    fn test_nan_bias_gradient_skipped() {
        let config = AdamConfig::default();
        let sw = simple_weights();
        let layers_ref: Vec<&LayerWeights> = sw.iter().collect();
        let mut opt = AdamOptimizer::new(config, &layers_ref);

        let mut layers = simple_weights();
        let grads = vec![(vec![vec![0.01; 4]; 3], vec![f32::NAN; 3])];
        let original_bias = layers[0].bias.clone();

        opt.step(&mut layers, &grads);
        assert_eq!(
            layers[0].bias, original_bias,
            "NaN bias gradients should not modify biases"
        );
    }

    #[test]
    fn test_reset_zeros_state() {
        let config = AdamConfig {
            learning_rate: 0.01,
            ..AdamConfig::default()
        };
        let sw = simple_weights();
        let layers_ref: Vec<&LayerWeights> = sw.iter().collect();
        let mut opt = AdamOptimizer::new(config, &layers_ref);

        let mut layers = simple_weights();
        let grads = simple_grads();
        opt.step(&mut layers, &grads);
        assert_eq!(opt.step_count(), 1);

        opt.reset();
        assert_eq!(opt.step_count(), 0);

        // After reset, another step with same grads should produce same result
        let mut layers2 = simple_weights();
        opt.step(&mut layers2, &grads);
        assert_eq!(
            layers[0].w, layers2[0].w,
            "After reset, weights should match first step"
        );
        assert_eq!(
            layers[0].bias, layers2[0].bias,
            "After reset, biases should match first step"
        );
    }

    #[test]
    fn test_flatten_unflatten_roundtrip() {
        let layers = vec![
            LayerWeights {
                w: vec![vec![1.0, 2.0]; 2],
                bias: vec![0.1; 2],
            },
            LayerWeights {
                w: vec![vec![3.0, 4.0, 5.0]],
                bias: vec![0.2],
            },
        ];
        let shapes = [(2, 2), (1, 3)];
        let flat = AdamOptimizer::flatten_weights(&layers);
        let restored = AdamOptimizer::unflatten_weights(&flat, &shapes);

        assert_eq!(restored.len(), layers.len());
        for (orig, rest) in layers.iter().zip(restored.iter()) {
            assert_eq!(orig.w, rest.w, "Weight matrices must match roundtrip");
            assert_eq!(orig.bias, rest.bias, "Bias vectors must match roundtrip");
        }
    }

    #[test]
    fn test_lr_schedule_changes_step_magnitude() {
        let sw = simple_weights();
        let layers_ref: Vec<&LayerWeights> = sw.iter().collect();
        let mut opt_low = AdamOptimizer::new(
            AdamConfig {
                learning_rate: 0.001,
                ..AdamConfig::default()
            },
            &layers_ref,
        );
        let mut opt_high = AdamOptimizer::new(
            AdamConfig {
                learning_rate: 0.1,
                ..AdamConfig::default()
            },
            &layers_ref,
        );

        let mut layers_low = simple_weights();
        let mut layers_high = simple_weights();
        let grads = simple_grads();

        opt_low.step(&mut layers_low, &grads);
        opt_high.step(&mut layers_high, &grads);

        // Higher LR should produce larger weight changes
        let diff_low = (layers_low[0].w[0][0] - 0.1).abs();
        let diff_high = (layers_high[0].w[0][0] - 0.1).abs();
        assert!(
            diff_high > diff_low,
            "Higher learning rate should produce larger weight changes"
        );
    }

    #[test]
    fn test_deterministic_update() {
        let config = AdamConfig::default();
        let sw = simple_weights();
        let layers_ref: Vec<&LayerWeights> = sw.iter().collect();
        let mut opt1 = AdamOptimizer::new(config.clone(), &layers_ref);
        let mut opt2 = AdamOptimizer::new(config, &layers_ref);

        let mut layers1 = simple_weights();
        let mut layers2 = simple_weights();
        let grads = simple_grads();

        opt1.step(&mut layers1, &grads);
        opt2.step(&mut layers2, &grads);

        assert_eq!(
            layers1[0].w, layers2[0].w,
            "Same config + same grads = same output"
        );
        assert_eq!(layers1[0].bias, layers2[0].bias);
    }

    #[test]
    fn test_weight_decay_changes_update() {
        let config_no_wd = AdamConfig {
            weight_decay: 0.0,
            ..AdamConfig::default()
        };
        let config_wd = AdamConfig {
            weight_decay: 0.1,
            learning_rate: 0.1,
            ..AdamConfig::default()
        };
        let sw = simple_weights();
        let layers_ref: Vec<&LayerWeights> = sw.iter().collect();
        let mut opt_no = AdamOptimizer::new(config_no_wd, &layers_ref);
        let mut opt_wd = AdamOptimizer::new(config_wd, &layers_ref);

        let mut layers_no = simple_weights();
        let mut layers_wd = simple_weights();
        let grads = simple_grads();

        opt_no.step(&mut layers_no, &grads);
        opt_wd.step(&mut layers_wd, &grads);

        // Weight decay produces a different update (wd adds wd*w to effective gradient)
        assert_ne!(
            layers_no[0].w[0][0], layers_wd[0].w[0][0],
            "Weight decay should change the effective update"
        );
    }
}

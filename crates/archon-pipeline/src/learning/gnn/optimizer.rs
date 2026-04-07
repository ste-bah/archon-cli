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
    m: Vec<Vec<f32>>,   // First moment (mean of gradients)
    v: Vec<Vec<f32>>,   // Second moment (mean of squared gradients)
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
    pub fn step(
        &mut self,
        layers: &mut [LayerWeights],
        grads: &[(Vec<Vec<f32>>, Vec<f32>)],
    ) {
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
                    state.m[i][j] = beta1 * state.m[i][j] + (1.0 - beta1) * g;
                    state.v[i][j] = beta2 * state.v[i][j] + (1.0 - beta2) * g * g;
                    let m_hat = state.m[i][j] / bc1;
                    let v_hat = state.v[i][j] / bc2;
                    *w -= lr * m_hat / (v_hat.sqrt() + eps);
                }
            }

            // Update biases
            for (i, b) in layer.bias.iter_mut().enumerate() {
                let g = db[i];
                state.m_bias[i] = beta1 * state.m_bias[i] + (1.0 - beta1) * g;
                state.v_bias[i] = beta2 * state.v_bias[i] + (1.0 - beta2) * g * g;
                let m_hat = state.m_bias[i] / bc1;
                let v_hat = state.v_bias[i] / bc2;
                *b -= lr * m_hat / (v_hat.sqrt() + eps);
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

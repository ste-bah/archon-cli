//! Backpropagation for GNN layers.

use super::math::ActivationType;

/// Result of backward pass through a single layer.
pub struct GradientResult {
    /// Weight gradients: [out_dim][in_dim].
    pub dw: Vec<Vec<f32>>,
    /// Input gradients: [in_dim].
    pub dx: Vec<f32>,
    /// Bias gradients: [out_dim].
    pub db: Vec<f32>,
}

/// Gradients for an attention mechanism.
pub struct AttentionGradients {
    pub dq: Vec<f32>,
    pub dk: Vec<f32>,
    pub dv: Vec<f32>,
}

/// Compute gradients through an activation function.
///
/// `output` is the post-activation values; `grad_output` is the upstream gradient.
pub fn activation_backward(
    output: &[f32],
    grad_output: &[f32],
    activation: ActivationType,
) -> Vec<f32> {
    match activation {
        ActivationType::Relu => output
            .iter()
            .zip(grad_output.iter())
            .map(|(o, g)| if *o > 0.0 { *g } else { 0.0 })
            .collect(),
        ActivationType::LeakyRelu => output
            .iter()
            .zip(grad_output.iter())
            .map(|(o, g)| if *o > 0.0 { *g } else { 0.01 * g })
            .collect(),
        ActivationType::Tanh => output
            .iter()
            .zip(grad_output.iter())
            .map(|(o, g)| g * (1.0 - o * o))
            .collect(),
        ActivationType::Sigmoid => output
            .iter()
            .zip(grad_output.iter())
            .map(|(o, g)| g * o * (1.0 - o))
            .collect(),
    }
}

/// Backward pass through a linear layer (no activation).
///
/// Given `input`, `weights`, and `grad_output` (the upstream gradient on the layer output),
/// compute gradients for weights, biases, and the input.
pub fn layer_backward(input: &[f32], weights: &[Vec<f32>], grad_output: &[f32]) -> GradientResult {
    let out_dim = weights.len();
    let in_dim = input.len();

    // dW[i][j] = grad_output[i] * input[j]
    let dw: Vec<Vec<f32>> = (0..out_dim)
        .map(|i| (0..in_dim).map(|j| grad_output[i] * input[j]).collect())
        .collect();

    // dx[j] = sum_i weights[i][j] * grad_output[i]
    let dx: Vec<f32> = (0..in_dim)
        .map(|j| (0..out_dim).map(|i| weights[i][j] * grad_output[i]).sum())
        .collect();

    // db = grad_output
    let db = grad_output.to_vec();

    GradientResult { dw, dx, db }
}

/// Clip gradient vector to a maximum L2 norm.
pub fn clip_gradients(gradients: &mut [f32], max_norm: f32) {
    let norm: f32 = gradients.iter().map(|g| g * g).sum::<f32>().sqrt();
    if norm > max_norm {
        let scale = max_norm / norm;
        for g in gradients.iter_mut() {
            *g *= scale;
        }
    }
}

/// Clip a weight gradient matrix to a maximum L2 norm.
pub fn clip_gradient_matrix(gradients: &mut [Vec<f32>], max_norm: f32) {
    let norm: f32 = gradients
        .iter()
        .flat_map(|row| row.iter())
        .map(|g| g * g)
        .sum::<f32>()
        .sqrt();
    if norm > max_norm {
        let scale = max_norm / norm;
        for row in gradients.iter_mut() {
            for g in row.iter_mut() {
                *g *= scale;
            }
        }
    }
}

/// Compute attention gradients given query, key, value, and upstream gradient.
pub fn attention_backward(
    query: &[f32],
    key: &[f32],
    _value: &[f32],
    grad_output: &[f32],
) -> AttentionGradients {
    let dim = query.len() as f32;
    let scale = if dim > 0.0 { 1.0 / dim.sqrt() } else { 1.0 };

    // Simplified attention backward: dq ~ scale * (grad_output . key), dk ~ scale * (grad_output . query)
    let dq: Vec<f32> = query
        .iter()
        .zip(grad_output.iter())
        .enumerate()
        .map(|(i, (_q, go))| {
            let k_val = key.get(i).copied().unwrap_or(0.0);
            go * k_val * scale
        })
        .collect();

    let dk: Vec<f32> = key
        .iter()
        .zip(grad_output.iter())
        .enumerate()
        .map(|(i, (_k, go))| {
            let q_val = query.get(i).copied().unwrap_or(0.0);
            go * q_val * scale
        })
        .collect();

    let dv = grad_output.to_vec();

    AttentionGradients { dq, dk, dv }
}

/// Compute gradient through softmax function.
///
/// Forward: sigma = softmax(z) where sigma_i = exp(z_i) / sum_j exp(z_j)
/// Backward: dL/dz_i = sigma_i * (dL/dsigma_i - dot(sigma, dL/dsigma))
pub fn softmax_backward(gradient: &[f32], softmax_output: &[f32]) -> Vec<f32> {
    let n = gradient.len().min(softmax_output.len());
    if n == 0 {
        return Vec::new();
    }

    // dot(sigma, dL/dsigma) = sum_j sigma_j * gradient_j
    let dot: f32 = softmax_output[..n]
        .iter()
        .zip(gradient[..n].iter())
        .map(|(s, g)| s * g)
        .sum();

    let mut dz: Vec<f32> = (0..n)
        .map(|i| softmax_output[i] * (gradient[i] - dot))
        .collect();

    // Numerical stability: zero out invalid gradients
    if dz.iter().any(|v| !v.is_finite()) {
        dz.fill(0.0);
    }

    dz
}

/// Full backward pass through the 3-layer GNN.
///
/// Given the activation caches from the forward pass and the loss gradient,
/// returns weight/bias gradients for all 3 layers.
///
/// Handles residual connections: when residual was used in the forward pass,
/// the gradient splits — part flows through the activation+projection path,
/// part flows directly to the input via the skip connection.
pub fn full_backward(
    caches: &[super::LayerActivationCache],
    layer_weights: [&super::LayerWeights; 3],
    loss_grad: &[f32],
    activations: [ActivationType; 3],
) -> Vec<GradientResult> {
    assert!(caches.len() == 3, "Expected 3 layer caches");

    let mut results = Vec::with_capacity(3);
    let mut grad = loss_grad.to_vec();

    // Backward through layers 3, 2, 1
    for i in (0..3).rev() {
        // Save gradient for residual skip connection.
        // Forward: output = activation(linear(input)) + input
        // Backward: gradient splits — part through activation+linear, part directly to input
        let residual_grad = grad.clone();

        // Use pre_activation for ReLU/LeakyRelu (need sign of input to activation)
        // Use true_post_activation for Tanh/Sigmoid (formula depends on output value)
        let act_input = match activations[i] {
            ActivationType::Relu | ActivationType::LeakyRelu => &caches[i].pre_activation,
            ActivationType::Tanh | ActivationType::Sigmoid => &caches[i].true_post_activation,
        };
        let act_grad = activation_backward(act_input, &grad, activations[i]);

        // Through linear layer
        let mut layer_grad = layer_backward(&caches[i].input, &layer_weights[i].w, &act_grad);

        // Add residual gradient to input gradient (skip connection)
        if residual_grad.len() == layer_grad.dx.len() {
            for (dx, rg) in layer_grad.dx.iter_mut().zip(residual_grad.iter()) {
                *dx += rg;
            }
        }

        grad = layer_grad.dx.clone();
        results.push(layer_grad);
    }

    // Reverse so results[0] = layer1, results[1] = layer2, results[2] = layer3
    results.reverse();
    results
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- softmax_backward ----

    #[test]
    fn test_softmax_backward_shape() {
        let grad = vec![0.1, 0.2, 0.3];
        let sm = vec![0.2, 0.3, 0.5];
        let dz = softmax_backward(&grad, &sm);
        assert_eq!(dz.len(), 3);
        for v in &dz {
            assert!(v.is_finite(), "softmax backward must produce finite values");
        }
    }

    #[test]
    fn test_softmax_backward_empty_input() {
        let dz = softmax_backward(&[], &[]);
        assert!(dz.is_empty());
    }

    #[test]
    fn test_softmax_backward_nan_input_produces_zero() {
        let grad = vec![f32::NAN, 0.2, 0.3];
        let sm = vec![0.2, 0.3, 0.5];
        let dz = softmax_backward(&grad, &sm);
        // NaN in gradient => all zero fallback
        for v in &dz {
            assert_eq!(*v, 0.0);
        }
    }

    // ---- activation_backward ----

    #[test]
    fn test_activation_backward_relu_gate() {
        // ReLU: gradient passes where pre_activation > 0
        let pre_act = vec![1.0, -1.0, 0.0, 2.0];
        let grad = vec![0.5; 4];
        let result = activation_backward(&pre_act, &grad, ActivationType::Relu);
        assert_eq!(result[0], 0.5); // positive -> pass
        assert_eq!(result[1], 0.0); // negative -> blocked
        assert_eq!(result[2], 0.0); // zero -> blocked
        assert_eq!(result[3], 0.5); // positive -> pass
    }

    #[test]
    fn test_activation_backward_leaky_relu() {
        let pre_act = vec![1.0, -1.0];
        let grad = vec![1.0; 2];
        let result = activation_backward(&pre_act, &grad, ActivationType::LeakyRelu);
        assert!((result[0] - 1.0).abs() < 1e-6); // positive: full gradient
        assert!((result[1] - 0.01).abs() < 1e-6); // negative: 0.01 * gradient
    }

    #[test]
    fn test_activation_backward_tanh() {
        // tanh backward: g * (1 - output^2)
        let output = vec![0.0, 0.5]; // tanh(0)=0, tanh(~0.55)=0.5
        let grad = vec![1.0; 2];
        let result = activation_backward(&output, &grad, ActivationType::Tanh);
        assert!((result[0] - 1.0).abs() < 1e-6); // 1 - 0^2 = 1
        assert!((result[1] - 0.75).abs() < 1e-6); // 1 - 0.5^2 = 0.75
    }

    #[test]
    fn test_activation_backward_sigmoid() {
        // sigmoid backward: g * output * (1 - output)
        let output = vec![0.5]; // sigmoid(0) = 0.5
        let grad = vec![1.0];
        let result = activation_backward(&output, &grad, ActivationType::Sigmoid);
        assert!((result[0] - 0.25).abs() < 1e-6); // 0.5 * (1-0.5) = 0.25
    }

    // ---- layer_backward ----

    #[test]
    fn test_layer_backward_dimensions() {
        let input = vec![1.0; 4];
        let weights = vec![vec![0.5; 4]; 3];
        let grad_output = vec![0.1; 3];

        let result = layer_backward(&input, &weights, &grad_output);
        assert_eq!(result.dw.len(), 3); // out_dim = 3
        assert_eq!(result.dw[0].len(), 4); // in_dim = 4
        assert_eq!(result.dx.len(), 4);
        assert_eq!(result.db.len(), 3);
    }

    // ---- full_backward ----

    fn make_test_cache(
        input: &[f32],
        pre: &[f32],
        true_post: &[f32],
        post: &[f32],
    ) -> super::super::LayerActivationCache {
        use std::sync::Arc;
        super::super::LayerActivationCache {
            layer_id: "test".to_string(),
            input: input.to_vec(),
            pre_activation: pre.to_vec(),
            true_post_activation: true_post.to_vec(),
            post_activation: post.to_vec(),
            weights: Arc::new(vec![vec![0.5; 4]; 4]),
        }
    }

    #[test]
    fn test_full_backward_produces_three_layers() {
        use super::super::LayerWeights;

        let caches = vec![
            make_test_cache(&[0.1; 4], &[0.2; 4], &[0.3; 4], &[0.4; 4]),
            make_test_cache(&[0.4; 4], &[0.5; 4], &[0.6; 4], &[0.7; 4]),
            make_test_cache(&[0.7; 4], &[0.8; 4], &[0.9; 4], &[1.0; 4]),
        ];
        let lw = LayerWeights { w: vec![vec![0.5; 4]; 4], bias: vec![0.1; 4] };
        let grads = full_backward(
            &caches,
            [&lw, &lw, &lw],
            &[0.1; 4],
            [ActivationType::LeakyRelu, ActivationType::LeakyRelu, ActivationType::Tanh],
        );

        assert_eq!(grads.len(), 3);
        for g in &grads {
            assert_eq!(g.dw.len(), 4);
            assert_eq!(g.dw[0].len(), 4);
            assert_eq!(g.dx.len(), 4);
            assert_eq!(g.db.len(), 4);
        }
    }

    #[test]
    fn test_full_backward_residual_adds_to_dx() {
        use super::super::LayerWeights;

        // With matching dimensions, residual gradient should be added to dx
        let caches = vec![
            make_test_cache(&[0.1; 4], &[0.2; 4], &[0.3; 4], &[0.4; 4]),
            make_test_cache(&[0.4; 4], &[0.5; 4], &[0.6; 4], &[0.7; 4]),
            make_test_cache(&[0.7; 4], &[0.8; 4], &[0.9; 4], &[1.0; 4]),
        ];
        let lw = LayerWeights { w: vec![vec![0.5; 4]; 4], bias: vec![0.1; 4] };
        let grads = full_backward(
            &caches,
            [&lw, &lw, &lw],
            &[0.1; 4],
            [ActivationType::Relu, ActivationType::Relu, ActivationType::Relu],
        );

        // dx should be non-zero (includes residual contribution)
        for g in &grads {
            let dx_norm: f32 = g.dx.iter().map(|v| v * v).sum::<f32>().sqrt();
            assert!(dx_norm > 0.0, "dx should be non-zero with residual connection");
        }
    }

    #[test]
    fn test_full_backward_finite_outputs() {
        use super::super::LayerWeights;

        let caches = vec![
            make_test_cache(&[0.1; 4], &[0.2; 4], &[0.3; 4], &[0.4; 4]),
            make_test_cache(&[0.5; 4], &[0.6; 4], &[0.7; 4], &[0.8; 4]),
            make_test_cache(&[0.9; 4], &[1.0; 4], &[1.1; 4], &[1.2; 4]),
        ];
        let lw = LayerWeights { w: vec![vec![0.5; 4]; 4], bias: vec![0.1; 4] };
        let grads = full_backward(
            &caches,
            [&lw, &lw, &lw],
            &[0.1; 4],
            [ActivationType::LeakyRelu, ActivationType::LeakyRelu, ActivationType::Tanh],
        );

        for g in &grads {
            for row in &g.dw {
                for v in row {
                    assert!(v.is_finite(), "dw must be finite");
                }
            }
            for v in &g.dx {
                assert!(v.is_finite(), "dx must be finite");
            }
            for v in &g.db {
                assert!(v.is_finite(), "db must be finite");
            }
        }
    }
}

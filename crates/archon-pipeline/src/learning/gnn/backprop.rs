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

/// Full backward pass through the 3-layer GNN.
///
/// Given the activation caches from the forward pass and the loss gradient,
/// returns weight/bias gradients for all 3 layers.
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
        // Through activation
        let act_grad = activation_backward(&caches[i].post_activation, &grad, activations[i]);
        // Through linear layer
        let layer_grad = layer_backward(&caches[i].input, &layer_weights[i].w, &act_grad);
        grad = layer_grad.dx.clone();
        results.push(layer_grad);
    }

    // Reverse so results[0] = layer1, results[1] = layer2, results[2] = layer3
    results.reverse();
    results
}

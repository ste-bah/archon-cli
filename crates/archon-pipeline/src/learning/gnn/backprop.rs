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

/// Backward pass through a projection layer (weights only, no activation, no dx).
///
/// Returns `(dw, db)` — weight and bias gradients. Unlike `layer_backward`,
/// this does not compute `dx` since projection layers are terminal (no further
/// gradient chaining).
pub fn project_backward(
    input: &[f32],
    weights: &[Vec<f32>],
    grad_output: &[f32],
) -> (Vec<Vec<f32>>, Vec<f32>) {
    let out_dim = weights.len();
    let in_dim = input.len();

    let dw: Vec<Vec<f32>> = (0..out_dim)
        .map(|i| (0..in_dim).map(|j| grad_output[i] * input[j]).collect())
        .collect();

    let db = grad_output.to_vec();

    (dw, db)
}

/// Backward pass through a graph aggregation step.
///
/// Forward: `aggregated = sum(neighbor_embeddings) / neighbor_count`
/// Backward: `d_neighbor = d_aggregated / neighbor_count` for each neighbor.
pub fn aggregate_backward(d_aggregated: &[f32], neighbor_count: usize) -> Vec<f32> {
    if neighbor_count == 0 {
        return vec![0.0; d_aggregated.len()];
    }
    let scale = 1.0 / neighbor_count as f32;
    d_aggregated.iter().map(|g| g * scale).collect()
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
mod tests;

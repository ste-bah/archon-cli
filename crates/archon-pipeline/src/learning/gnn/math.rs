//! GNN math utilities — activation functions, softmax, projection, normalization.

/// Activation function types supported by the GNN layers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActivationType {
    Relu,
    LeakyRelu,
    Tanh,
    Sigmoid,
}

/// Apply element-wise activation to a vector.
pub fn apply_activation(input: &[f32], activation: ActivationType) -> Vec<f32> {
    match activation {
        ActivationType::Relu => input.iter().map(|x| x.max(0.0)).collect(),
        ActivationType::LeakyRelu => input
            .iter()
            .map(|x| if *x > 0.0 { *x } else { 0.01 * x })
            .collect(),
        ActivationType::Tanh => input.iter().map(|x| x.tanh()).collect(),
        ActivationType::Sigmoid => input.iter().map(|x| 1.0 / (1.0 + (-x).exp())).collect(),
    }
}

/// Compute the derivative of an activation function given the *output* values.
pub fn activation_derivative(output: &[f32], activation: ActivationType) -> Vec<f32> {
    match activation {
        ActivationType::Relu => output.iter().map(|o| if *o > 0.0 { 1.0 } else { 0.0 }).collect(),
        ActivationType::LeakyRelu => output
            .iter()
            .map(|o| if *o > 0.0 { 1.0 } else { 0.01 })
            .collect(),
        ActivationType::Tanh => output.iter().map(|o| 1.0 - o * o).collect(),
        ActivationType::Sigmoid => output.iter().map(|o| o * (1.0 - o)).collect(),
    }
}

/// Matrix-vector multiply: output[i] = sum(weight[i][j] * input[j]) + bias[i].
pub fn project(input: &[f32], weight: &[Vec<f32>], bias: &[f32]) -> Vec<f32> {
    weight
        .iter()
        .zip(bias.iter())
        .map(|(row, b)| {
            let dot: f32 = row.iter().zip(input.iter()).map(|(w, x)| w * x).sum();
            dot + b
        })
        .collect()
}

/// Softmax over a vector. Returns uniform distribution if sum of exponentials is zero.
pub fn softmax(input: &[f32]) -> Vec<f32> {
    if input.is_empty() {
        return vec![];
    }
    let max = input
        .iter()
        .cloned()
        .fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f32> = input.iter().map(|x| (x - max).exp()).collect();
    let sum: f32 = exps.iter().sum();
    if sum == 0.0 {
        return vec![1.0 / input.len() as f32; input.len()];
    }
    exps.iter().map(|e| e / sum).collect()
}

/// L2-normalize a vector. Returns the original vector if norm is zero.
pub fn normalize(input: &[f32]) -> Vec<f32> {
    let norm: f32 = input.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 {
        return input.to_vec();
    }
    input.iter().map(|x| x / norm).collect()
}

/// Pad or truncate a vector to `target_len`.
pub fn zero_pad(input: &[f32], target_len: usize) -> Vec<f32> {
    let mut result = input.to_vec();
    result.resize(target_len, 0.0);
    result.truncate(target_len);
    result
}

/// Element-wise vector addition.
pub fn add_vectors(a: &[f32], b: &[f32]) -> Vec<f32> {
    a.iter().zip(b.iter()).map(|(x, y)| x + y).collect()
}

/// Scaled dot-product attention score between query and key.
pub fn attention_score(query: &[f32], key: &[f32]) -> f32 {
    let dot: f32 = query.iter().zip(key.iter()).map(|(q, k)| q * k).sum();
    let dim = query.len() as f32;
    if dim == 0.0 {
        return 0.0;
    }
    dot / dim.sqrt()
}

/// Weighted sum of vectors.
pub fn weighted_aggregate(vectors: &[Vec<f32>], weights: &[f32]) -> Vec<f32> {
    if vectors.is_empty() {
        return vec![];
    }
    let dim = vectors[0].len();
    let mut result = vec![0.0f32; dim];
    for (vec, &w) in vectors.iter().zip(weights.iter()) {
        for (r, v) in result.iter_mut().zip(vec.iter()) {
            *r += v * w;
        }
    }
    result
}

/// Cosine similarity between two f32 vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    (dot / (mag_a * mag_b)).clamp(-1.0, 1.0)
}

/// Element-wise subtraction: a - b.
pub fn subtract_vectors(a: &[f32], b: &[f32]) -> Vec<f32> {
    a.iter().zip(b.iter()).map(|(x, y)| x - y).collect()
}

/// Dot product of two vectors.
pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Element-wise multiply.
pub fn hadamard(a: &[f32], b: &[f32]) -> Vec<f32> {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).collect()
}

/// Scale a vector by a scalar.
pub fn scale(v: &[f32], s: f32) -> Vec<f32> {
    v.iter().map(|x| x * s).collect()
}

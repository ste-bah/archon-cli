//! GNN math utilities — ported from root archon TS gnn-math.ts.
//!
//! All functions are pure compute on Vec<f32>. No async, no alloc beyond results.

/// Activation function types.
#[derive(Debug, Clone, Copy, PartialEq)]
#[derive(Default)]
pub enum ActivationType {
    #[default]
    Relu,
    Tanh,
    Sigmoid,
    LeakyRelu,
}


#[allow(clippy::needless_range_loop)]
/// Element-wise vector addition. Result length is max(a.len(), b.len()).
/// Missing elements are treated as 0.
pub fn add_vectors(a: &[f32], b: &[f32]) -> Vec<f32> {
    let max_len = a.len().max(b.len());
    let mut result = vec![0.0; max_len];
    for i in 0..max_len {
        let av = a.get(i).copied().unwrap_or(0.0);
        let bv = b.get(i).copied().unwrap_or(0.0);
        result[i] = av + bv;
    }
    result
}

/// Zero-pad or truncate a vector to `target_dim`.
pub fn zero_pad(v: &[f32], target_dim: usize) -> Vec<f32> {
    if v.len() >= target_dim {
        return v[..target_dim].to_vec();
    }
    let mut padded = vec![0.0; target_dim];
    padded[..v.len()].copy_from_slice(v);
    padded
}

/// L2-normalize a vector. Returns input as-is if magnitude is zero.
pub fn normalize(v: &[f32]) -> Vec<f32> {
    let magnitude: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if magnitude == 0.0 {
        return v.to_vec();
    }
    v.iter().map(|x| x / magnitude).collect()
}

/// Cosine similarity between two vectors. Uses min length for comparison.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let min_len = a.len().min(b.len());
    let mut dot = 0.0;
    let mut mag_a = 0.0;
    let mut mag_b = 0.0;
    for i in 0..min_len {
        dot += a[i] * b[i];
        mag_a += a[i] * a[i];
        mag_b += b[i] * b[i];
    }
    mag_a = mag_a.sqrt();
    mag_b = mag_b.sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    dot / (mag_a * mag_b)
}

/// Apply activation function element-wise.
pub fn apply_activation(v: &[f32], activation: ActivationType) -> Vec<f32> {
    v.iter()
        .map(|&x| match activation {
            ActivationType::Relu => x.max(0.0),
            ActivationType::Tanh => x.tanh(),
            ActivationType::Sigmoid => 1.0 / (1.0 + (-x).exp()),
            ActivationType::LeakyRelu => {
                if x > 0.0 {
                    x
                } else {
                    0.01 * x
                }
            }
        })
        .collect()
}

#[allow(clippy::needless_range_loop)]
/// Learned projection: result[o] = sum_i weights[o][i] * input[i].
/// No bias term — matches TS project().
pub fn project(input: &[f32], weights: &[Vec<f32>], output_dim: usize) -> Vec<f32> {
    let mut result = vec![0.0; output_dim];
    for o in 0..output_dim {
        let mut sum = 0.0;
        if let Some(w) = weights.get(o) {
            let limit = input.len().min(w.len());
            for i in 0..limit {
                sum += input[i] * w[i];
            }
        }
        result[o] = sum;
    }
    result
}

/// Matrix-vector multiplication: result[i] = sum_j matrix[i][j] * vector[j].
pub fn mat_vec_mul(matrix: &[Vec<f32>], vector: &[f32]) -> Vec<f32> {
    let rows = matrix.len();
    let mut result = vec![0.0; rows];
    for i in 0..rows {
        let mut sum = 0.0;
        let row = &matrix[i];
        let limit = vector.len().min(row.len());
        for j in 0..limit {
            sum += row[j] * vector[j];
        }
        result[i] = sum;
    }
    result
}

/// Numerically stable softmax. Returns uniform distribution if sum of exp == 0.
pub fn softmax(scores: &[f32]) -> Vec<f32> {
    if scores.is_empty() {
        return vec![];
    }
    let max_score = scores.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let exp_scores: Vec<f32> = scores.iter().map(|&s| (s - max_score).exp()).collect();
    let sum_exp: f32 = exp_scores.iter().sum();
    if sum_exp == 0.0 {
        let uniform = 1.0 / scores.len() as f32;
        return vec![uniform; scores.len()];
    }
    exp_scores.iter().map(|&e| e / sum_exp).collect()
}

/// Scaled dot-product attention score.
/// Default scale = 1 / sqrt(min_len).
pub fn attention_score(query: &[f32], key: &[f32], scale: Option<f32>) -> f32 {
    let min_len = query.len().min(key.len());
    if min_len == 0 {
        return 0.0;
    }
    let mut dot = 0.0;
    for i in 0..min_len {
        dot += query[i] * key[i];
    }
    let scale_factor = scale.unwrap_or_else(|| 1.0 / (min_len as f32).sqrt());
    dot * scale_factor
}

/// Weighted aggregate of feature vectors by attention weights.
pub fn weighted_aggregate(features: &[Vec<f32>], attention_weights: &[f32]) -> Vec<f32> {
    if features.is_empty() || attention_weights.is_empty() {
        return vec![];
    }
    let dim = features[0].len();
    let mut result = vec![0.0; dim];
    let num = features.len().min(attention_weights.len());
    for f in 0..num {
        let weight = attention_weights[f];
        let feature = &features[f];
        let limit = feature.len().min(dim);
        for i in 0..limit {
            result[i] += weight * feature[i];
        }
    }
    result
}

/// Compute neighbor attention scores from adjacency row.
/// Returns (neighbor_indices, raw_scores).
pub fn compute_neighbor_attention(
    center_idx: usize,
    center: &[f32],
    features: &[Vec<f32>],
    adjacency_row: &[f32],
) -> (Vec<usize>, Vec<f32>) {
    let mut neighbor_indices = Vec::new();
    let mut raw_scores = Vec::new();

    let limit = adjacency_row.len().min(features.len());
    for j in 0..limit {
        let edge_weight = adjacency_row[j];
        if edge_weight > 0.0 && j != center_idx {
            neighbor_indices.push(j);
            let raw_score = attention_score(center, &features[j], None);
            raw_scores.push(raw_score * edge_weight);
        }
    }

    (neighbor_indices, raw_scores)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_vectors_max_length() {
        let a = vec![1.0, 2.0];
        let b = vec![10.0, 20.0, 30.0];
        let result = add_vectors(&a, &b);
        assert_eq!(result.len(), 3);
        assert_eq!(result, vec![11.0, 22.0, 30.0]);
    }

    #[test]
    fn test_zero_pad_truncate() {
        let v = vec![1.0, 2.0, 3.0];
        assert_eq!(zero_pad(&v, 2), vec![1.0, 2.0]);
    }

    #[test]
    fn test_zero_pad_extend() {
        let v = vec![1.0, 2.0];
        assert_eq!(zero_pad(&v, 4), vec![1.0, 2.0, 0.0, 0.0]);
    }

    #[test]
    fn test_normalize_unit() {
        let v = vec![3.0, 4.0];
        let n = normalize(&v);
        assert!((n[0] - 0.6).abs() < 1e-6);
        assert!((n[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn test_normalize_zero() {
        let v = vec![0.0, 0.0];
        assert_eq!(normalize(&v), v);
    }

    #[test]
    fn test_softmax_sums_to_one() {
        let scores = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let sm = softmax(&scores);
        let sum: f32 = sm.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_softmax_empty() {
        assert!(softmax(&[]).is_empty());
    }

    #[test]
    fn test_attention_score_scaled() {
        let q = vec![1.0, 0.0];
        let k = vec![1.0, 0.0];
        let score = attention_score(&q, &k, None);
        // dot=1, scale=1/sqrt(2), so score=1/sqrt(2)
        assert!((score - 1.0 / 2.0f32.sqrt()).abs() < 1e-6);
    }

    #[test]
    fn test_activation_relu() {
        let v = vec![-2.0, -1.0, 0.0, 1.0, 2.0];
        assert_eq!(
            apply_activation(&v, ActivationType::Relu),
            vec![0.0, 0.0, 0.0, 1.0, 2.0]
        );
    }

    #[test]
    fn test_activation_leaky_relu() {
        let v = vec![-2.0, 0.0, 2.0];
        let out = apply_activation(&v, ActivationType::LeakyRelu);
        assert!((out[0] - (-0.02)).abs() < 1e-6);
        assert_eq!(out[1], 0.0);
        assert_eq!(out[2], 2.0);
    }

    #[test]
    fn test_activation_tanh() {
        let v = vec![0.0];
        let out = apply_activation(&v, ActivationType::Tanh);
        assert!((out[0] - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_activation_sigmoid() {
        let v = vec![0.0];
        let out = apply_activation(&v, ActivationType::Sigmoid);
        assert!((out[0] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_project_no_bias() {
        // Simple identity-like projection
        let weights: Vec<Vec<f32>> = vec![vec![1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0]];
        let input = vec![2.0, 3.0, 5.0];
        let out = project(&input, &weights, 2);
        assert_eq!(out[0], 2.0);
        assert_eq!(out[1], 3.0);
    }

    #[test]
    fn test_weighted_aggregate() {
        let features = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let weights = vec![0.7, 0.3];
        let result = weighted_aggregate(&features, &weights);
        assert!((result[0] - 0.7).abs() < 1e-6);
        assert!((result[1] - 0.3).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!((cosine_similarity(&a, &b) - 0.0).abs() < 1e-6);

        let c = vec![1.0, 0.0];
        assert!((cosine_similarity(&a, &c) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_compute_neighbor_attention() {
        let center = vec![1.0, 0.0, 0.0];
        let features = vec![
            center.clone(),      // node 0 = center
            vec![0.5, 0.5, 0.0], // node 1 = neighbor
            vec![0.0, 1.0, 0.0], // node 2 = neighbor
        ];
        let adj_row = vec![0.0, 0.9, 0.1, 0.0]; // edge to node 1 weight 0.9, node 2 weight 0.1
        let (indices, scores) = compute_neighbor_attention(0, &center, &features, &adj_row);
        assert_eq!(indices.len(), 2);
        assert!(!scores.iter().any(|s| s.is_nan()));
    }
}

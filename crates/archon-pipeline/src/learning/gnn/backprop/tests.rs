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
    let lw = LayerWeights {
        w: vec![vec![0.5; 4]; 4],
        bias: vec![0.1; 4],
    };
    let grads = full_backward(
        &caches,
        [&lw, &lw, &lw],
        &[0.1; 4],
        [
            ActivationType::LeakyRelu,
            ActivationType::LeakyRelu,
            ActivationType::Tanh,
        ],
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
    let lw = LayerWeights {
        w: vec![vec![0.5; 4]; 4],
        bias: vec![0.1; 4],
    };
    let grads = full_backward(
        &caches,
        [&lw, &lw, &lw],
        &[0.1; 4],
        [
            ActivationType::Relu,
            ActivationType::Relu,
            ActivationType::Relu,
        ],
    );

    // dx should be non-zero (includes residual contribution)
    for g in &grads {
        let dx_norm: f32 = g.dx.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!(
            dx_norm > 0.0,
            "dx should be non-zero with residual connection"
        );
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
    let lw = LayerWeights {
        w: vec![vec![0.5; 4]; 4],
        bias: vec![0.1; 4],
    };
    let grads = full_backward(
        &caches,
        [&lw, &lw, &lw],
        &[0.1; 4],
        [
            ActivationType::LeakyRelu,
            ActivationType::LeakyRelu,
            ActivationType::Tanh,
        ],
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

// ---- project_backward ----

#[test]
fn test_project_backward_dimensions() {
    let input = vec![1.0; 4];
    let weights = vec![vec![0.5; 4]; 3];
    let grad_output = vec![0.1; 3];

    let (dw, db) = project_backward(&input, &weights, &grad_output);
    assert_eq!(dw.len(), 3);
    assert_eq!(dw[0].len(), 4);
    assert_eq!(db.len(), 3);
}

#[test]
fn test_project_backward_no_dx() {
    // project_backward does NOT return dx — it's for terminal projections
    let input = vec![2.0; 4];
    let weights = vec![vec![0.25; 4]; 2];
    let grad_output = vec![0.5; 2];

    let (dw, db) = project_backward(&input, &weights, &grad_output);
    // dw[i][j] = grad_output[i] * input[j] = 0.5 * 2.0 = 1.0
    for row in &dw {
        for &v in row {
            assert!((v - 1.0).abs() < 1e-6);
        }
    }
    // db = grad_output
    assert!((db[0] - 0.5).abs() < 1e-6);
    assert!((db[1] - 0.5).abs() < 1e-6);
}

// ---- aggregate_backward ----

#[test]
fn test_aggregate_backward_equal_split() {
    let d_agg = vec![3.0, 6.0, 9.0];
    let result = aggregate_backward(&d_agg, 3);
    // Each neighbor gets 1/3 of the gradient
    assert!((result[0] - 1.0).abs() < 1e-6);
    assert!((result[1] - 2.0).abs() < 1e-6);
    assert!((result[2] - 3.0).abs() < 1e-6);
}

#[test]
fn test_aggregate_backward_single_neighbor() {
    let d_agg = vec![5.0, 10.0];
    let result = aggregate_backward(&d_agg, 1);
    // Single neighbor gets all gradient
    assert!((result[0] - 5.0).abs() < 1e-6);
    assert!((result[1] - 10.0).abs() < 1e-6);
}

#[test]
fn test_aggregate_backward_zero_neighbors() {
    let d_agg = vec![1.0, 2.0];
    let result = aggregate_backward(&d_agg, 0);
    // Zero neighbors -> zero gradient
    assert_eq!(result, vec![0.0, 0.0]);
}

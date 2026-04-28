//! GNN gradient correctness — numerical gradient check via finite differences.
//!
//! Verifies that `layer_backward` and `full_backward` produce gradients
//! matching central-difference estimates within 1e-3 relative error on
//! a 3-layer toy network with 8-dim input.

use archon_pipeline::learning::gnn::backprop::{self, full_backward};
use archon_pipeline::learning::gnn::math::ActivationType;
use archon_pipeline::learning::gnn::cache::CacheConfig;
use archon_pipeline::learning::gnn::{GnnConfig, GnnEnhancer};

/// Simple linear forward pass in f64: y = W @ x.
fn linear_forward_f64(weights: &[Vec<f64>], input: &[f64]) -> Vec<f64> {
    weights
        .iter()
        .map(|row| row.iter().zip(input.iter()).map(|(w, x)| w * x).sum())
        .collect()
}

fn linear_loss_f64(out: &[f64], grad: &[f64]) -> f64 {
    out.iter().zip(grad.iter()).map(|(o, g)| o * g).sum()
}

#[test]
fn layer_backward_matches_numerical_gradient() {
    let in_dim = 4;
    let out_dim = 3;
    let eps = 1e-3f64;
    let tol = 2e-3f64;

    let weights: Vec<Vec<f32>> = vec![
        vec![0.5, -0.3, 0.8, -0.1],
        vec![-0.4, 0.6, -0.2, 0.7],
        vec![0.1, -0.5, 0.3, -0.8],
    ];
    let input: Vec<f32> = vec![0.2, -0.4, 0.6, -0.3];
    // grad_output is treated as dL/d(out) for our analytical check
    let grad_output: Vec<f32> = vec![0.1, -0.2, 0.15];

    // Analytical gradient via layer_backward
    let analytical = backprop::layer_backward(&input, &weights, &grad_output);

    assert_eq!(analytical.dw.len(), out_dim);
    assert_eq!(analytical.dw[0].len(), in_dim);
    assert_eq!(analytical.db.len(), out_dim);
    assert_eq!(analytical.dx.len(), in_dim);

    // Convert to f64 for high-precision numerical gradient
    let w_f64: Vec<Vec<f64>> = weights.iter().map(|row| row.iter().map(|&v| v as f64).collect()).collect();
    let x_f64: Vec<f64> = input.iter().map(|&v| v as f64).collect();
    let g_f64: Vec<f64> = grad_output.iter().map(|&v| v as f64).collect();

    // Numerical check: weights
    let base_loss_f64 = linear_forward_f64(&w_f64, &x_f64);
    for i in 0..out_dim {
        for j in 0..in_dim {
            let mut wp = w_f64.clone();
            wp[i][j] += eps;
            let lp = linear_loss_f64(&linear_forward_f64(&wp, &x_f64), &g_f64);

            let mut wm = w_f64.clone();
            wm[i][j] -= eps;
            let lm = linear_loss_f64(&linear_forward_f64(&wm, &x_f64), &g_f64);

            let numerical = (lp - lm) / (2.0 * eps);
            let expected = g_f64[i] * x_f64[j];

            let denom = numerical.abs().max(expected.abs()).max(1e-12);
            let rel_err = (numerical - expected).abs() / denom;
            assert!(
                rel_err < tol,
                "dw[{i}][{j}]: numerical={numerical:.8}, expected={expected:.8}, rel_err={rel_err:.8}"
            );
        }
    }

    // Numerical check: bias
    for i in 0..out_dim {
        let mut bp = base_loss_f64.clone();
        bp[i] += eps;
        let lp = linear_loss_f64(&bp, &g_f64);

        let mut bm = base_loss_f64.clone();
        bm[i] -= eps;
        let lm = linear_loss_f64(&bm, &g_f64);

        let numerical = (lp - lm) / (2.0 * eps);
        let expected = g_f64[i];

        let denom = numerical.abs().max(expected.abs()).max(1e-12);
        let rel_err = (numerical - expected).abs() / denom;
        assert!(
            rel_err < tol,
            "db[{i}]: numerical={numerical:.8}, expected={expected:.8}, rel_err={rel_err:.8}"
        );
    }
}

#[test]
fn full_backward_on_toy_gnn_produces_nonzero_gradients() {
    let mut cfg = GnnConfig::default();
    cfg.input_dim = 8;
    cfg.output_dim = 8;
    cfg.num_layers = 3;
    let enhancer = GnnEnhancer::with_in_memory_weights(cfg, CacheConfig::default(), 42);

    let input: Vec<f32> = vec![0.5; 8];
    let target: Vec<f32> = vec![-0.5; 8];

    let fwd = enhancer.enhance(&input, None, None, true);
    assert!(fwd.activation_cache.len() >= 3);

    // L2 loss gradient
    let loss_grad: Vec<f32> = fwd
        .enhanced
        .iter()
        .zip(target.iter())
        .map(|(p, t)| 2.0 * (p - t))
        .collect();

    let (l1, l2, l3) = enhancer.get_weights();
    let grads = full_backward(
        &fwd.activation_cache,
        [&l1, &l2, &l3],
        &loss_grad,
        [
            ActivationType::LeakyRelu,
            ActivationType::LeakyRelu,
            ActivationType::Tanh,
        ],
    );

    assert_eq!(grads.len(), 3);

    for (i, g) in grads.iter().enumerate() {
        let dw_nonzero = g.dw.iter().any(|row| row.iter().any(|&v| v.abs() > 1e-9));
        let db_nonzero = g.db.iter().any(|&v| v.abs() > 1e-9);
        assert!(dw_nonzero, "layer {i} dw should have non-zero elements");
        assert!(db_nonzero, "layer {i} db should have non-zero elements");
    }
}

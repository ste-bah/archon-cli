use super::super::GnnEnhancer;

/// Compute per-layer L2 weight norms and NaN/Inf status.
pub(super) fn compute_layer_norms(enhancer: &GnnEnhancer) -> Vec<(String, f32, bool)> {
    let (l1, l2, l3) = enhancer.get_weights();
    let layers = [("layer1", &l1), ("layer2", &l2), ("layer3", &l3)];
    layers
        .iter()
        .map(|(name, lw)| {
            let mut sum_sq = 0.0f64;
            let mut has_nan = false;
            for row in &lw.w {
                for &v in row {
                    if v.is_nan() || v.is_infinite() {
                        has_nan = true;
                    }
                    sum_sq += (v as f64) * (v as f64);
                }
            }
            for &v in &lw.bias {
                if v.is_nan() || v.is_infinite() {
                    has_nan = true;
                }
                sum_sq += (v as f64) * (v as f64);
            }
            (name.to_string(), (sum_sq.sqrt()) as f32, has_nan)
        })
        .collect()
}

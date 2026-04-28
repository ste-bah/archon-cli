//! Elastic Weight Consolidation (EWC) regularizer.
//!
//! Prevents catastrophic forgetting by penalizing changes to weights deemed
//! important by the Fisher information matrix diagonal.  Operates per-layer
//! with online Fisher accumulation (decay factor 0.9).

use std::collections::HashMap;

/// EWC regularizer with per-layer Fisher diagonals and anchor weights.
///
/// After a successful training run, call `update_anchor` to freeze the current
/// weights as the new reference.  During subsequent training, the EWC penalty
/// term discourages large changes to weights with high Fisher importance.
#[derive(Debug, Clone)]
pub struct EwcRegularizer {
    /// Regularization strength (default 0.1).
    pub lambda: f32,
    /// Per-layer Fisher information diagonal (importance of each weight).
    fisher: HashMap<String, Vec<Vec<f32>>>,
    /// Per-layer anchor weights (reference point for penalty).
    anchor: HashMap<String, Vec<Vec<f32>>>,
}

impl EwcRegularizer {
    /// Create a new EWC regularizer with the given lambda.
    pub fn new(lambda: f32) -> Self {
        Self {
            lambda,
            fisher: HashMap::new(),
            anchor: HashMap::new(),
        }
    }

    // -----------------------------------------------------------------------
    // Per-layer API (used by trainer)
    // -----------------------------------------------------------------------

    /// Compute total EWC penalty across all layers.
    ///
    /// `penalty = lambda/2 * sum_layers sum_ij F_ij * (w_ij - a_ij)^2`
    pub fn penalty(&self, current_weights: &HashMap<String, Vec<Vec<f32>>>) -> f32 {
        if self.anchor.is_empty() || self.fisher.is_empty() {
            return 0.0;
        }

        let mut total = 0.0_f32;
        for (layer_id, cur_w) in current_weights {
            if let (Some(anchor_w), Some(fisher_w)) =
                (self.anchor.get(layer_id), self.fisher.get(layer_id))
            {
                for (row_cur, row_anchor, row_fisher) in cur_w
                    .iter()
                    .zip(anchor_w.iter())
                    .zip(fisher_w.iter())
                    .map(|((c, a), f)| (c, a, f))
                {
                    for (cur, anchor, fisher) in row_cur
                        .iter()
                        .zip(row_anchor.iter())
                        .zip(row_fisher.iter())
                        .map(|((c, a), f)| (c, a, f))
                    {
                        let diff = cur - anchor;
                        total += fisher * diff * diff;
                    }
                }
            }
        }

        self.lambda * 0.5 * total
    }

    /// Compute EWC penalty gradient for a single layer.
    ///
    /// `d(penalty)/dw_ij = lambda * F_ij * (w_ij - a_ij)`
    pub fn penalty_gradient(&self, layer_id: &str, current: &[Vec<f32>]) -> Vec<Vec<f32>> {
        let anchor_w = match self.anchor.get(layer_id) {
            Some(a) => a,
            None => return vec![vec![0.0; current[0].len()]; current.len()],
        };
        let fisher_w = match self.fisher.get(layer_id) {
            Some(f) => f,
            None => return vec![vec![0.0; current[0].len()]; current.len()],
        };

        let mut grad = current.to_vec();
        for (row_idx, row_cur) in current.iter().enumerate() {
            for (col_idx, cur) in row_cur.iter().enumerate() {
                let anchor = anchor_w[row_idx][col_idx];
                let fisher = fisher_w[row_idx][col_idx];
                grad[row_idx][col_idx] = self.lambda * fisher * (cur - anchor);
            }
        }
        grad
    }

    /// Update Fisher information for a single layer with online exponential decay.
    ///
    /// `new_F = 0.9 * old_F + 0.1 * grad^2`
    pub fn update_fisher(&mut self, layer_id: &str, sample_grads: &[Vec<f32>]) {
        let decay = 0.9_f32;
        let complement = 1.0 - decay; // 0.1

        let entry = self
            .fisher
            .entry(layer_id.to_string())
            .or_insert_with(|| vec![vec![0.0; sample_grads[0].len()]; sample_grads.len()]);

        for (row_f, row_g) in entry.iter_mut().zip(sample_grads.iter()) {
            for (f, g) in row_f.iter_mut().zip(row_g.iter()) {
                *f = decay * *f + complement * g * g;
            }
        }
    }

    /// Save current weights as the new anchor (after successful training run).
    ///
    /// Called post-rollback check: only update anchor when training was
    /// successful (loss decreased, no NaN).
    pub fn update_anchor(&mut self, weights: &HashMap<String, Vec<Vec<f32>>>) {
        self.anchor.clear();
        for (layer_id, w) in weights {
            self.anchor.insert(layer_id.clone(), w.clone());
        }
    }

    // -----------------------------------------------------------------------
    // Flat-vector API (backward compat)
    // -----------------------------------------------------------------------

    /// Compute EWC penalty from flat weight vector (legacy).
    pub fn compute_penalty(&self, current_weights: &[f32]) -> f32 {
        if self.anchor.is_empty() || self.fisher.is_empty() {
            return 0.0;
        }

        // Flatten anchor and fisher for comparison
        let flat_anchor: Vec<f32> = self
            .anchor
            .values()
            .flat_map(|w| w.iter().flat_map(|row| row.iter()))
            .copied()
            .collect();
        let flat_fisher: Vec<f32> = self
            .fisher
            .values()
            .flat_map(|w| w.iter().flat_map(|row| row.iter()))
            .copied()
            .collect();

        if flat_anchor.len() != current_weights.len() {
            return 0.0;
        }

        let penalty: f32 = current_weights
            .iter()
            .zip(flat_anchor.iter())
            .zip(flat_fisher.iter())
            .map(|((cur, saved), fisher)| {
                let diff = cur - saved;
                fisher * diff * diff
            })
            .sum();

        self.lambda * 0.5 * penalty
    }

    /// Compute EWC penalty gradient from flat vectors (legacy).
    pub fn compute_penalty_gradient(&self, current_weights: &[f32]) -> Vec<f32> {
        let flat_anchor: Vec<f32> = self
            .anchor
            .values()
            .flat_map(|w| w.iter().flat_map(|row| row.iter()))
            .copied()
            .collect();
        let flat_fisher: Vec<f32> = self
            .fisher
            .values()
            .flat_map(|w| w.iter().flat_map(|row| row.iter()))
            .copied()
            .collect();

        if flat_anchor.is_empty() || flat_anchor.len() != current_weights.len() {
            return vec![0.0; current_weights.len()];
        }

        current_weights
            .iter()
            .zip(flat_anchor.iter())
            .zip(flat_fisher.iter())
            .map(|((cur, saved), fisher)| self.lambda * fisher * (cur - saved))
            .collect()
    }

    /// Save flat weights + gradients as Fisher (legacy).
    pub fn update_fisher_information(&mut self, weights: &[f32], gradients: &[f32]) {
        // Store under a single "all" layer key for backward compat
        let n = weights.len();
        let w: Vec<Vec<f32>> = vec![weights.to_vec()]; // 1×n
        let g: Vec<Vec<f32>> = vec![gradients.to_vec()]; // 1×n
        self.update_fisher("all", &g);
        self.anchor
            .entry("all".to_string())
            .or_insert_with(|| w.clone());
        if self.anchor["all"].len() == 1 && self.anchor["all"][0].len() == n {
            self.anchor.insert("all".to_string(), w);
        }
    }

    // -----------------------------------------------------------------------
    // Accessors
    // -----------------------------------------------------------------------

    /// Set the lambda (regularization strength).
    pub fn set_lambda(&mut self, lambda: f32) {
        self.lambda = lambda;
    }

    /// Get the current lambda.
    pub fn lambda(&self) -> f32 {
        self.lambda
    }

    /// Check if EWC has been initialized with saved weights.
    pub fn is_initialized(&self) -> bool {
        !self.anchor.is_empty()
    }

    /// Number of layers with Fisher information stored.
    pub fn layer_count(&self) -> usize {
        self.fisher.len()
    }
}

impl Default for EwcRegularizer {
    fn default() -> Self {
        Self::new(0.1)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_layer_weights(out_dim: usize, in_dim: usize, val: f32) -> Vec<Vec<f32>> {
        vec![vec![val; in_dim]; out_dim]
    }

    fn make_weights_map() -> HashMap<String, Vec<Vec<f32>>> {
        let mut m = HashMap::new();
        m.insert("layer1".to_string(), make_layer_weights(2, 2, 0.5));
        m.insert("layer2".to_string(), make_layer_weights(2, 2, -0.3));
        m
    }

    #[test]
    fn test_penalty_zero_when_no_anchor() {
        let ewc = EwcRegularizer::new(0.1);
        let weights = make_weights_map();
        assert_eq!(ewc.penalty(&weights), 0.0);
    }

    #[test]
    fn test_penalty_zero_at_anchor() {
        let mut ewc = EwcRegularizer::new(0.1);
        let weights = make_weights_map();
        ewc.update_anchor(&weights);
        // Fisher not set — penalty still zero
        assert_eq!(ewc.penalty(&weights), 0.0);
    }

    #[test]
    fn test_penalty_positive_when_deviating() {
        let mut ewc = EwcRegularizer::new(1.0);
        let weights = make_weights_map();

        // Set Fisher directly: all ones for layer1, all zeros for layer2
        let mut fisher_map = HashMap::new();
        fisher_map.insert(
            "layer1".to_string(),
            vec![vec![1.0; 2]; 2],
        );
        fisher_map.insert(
            "layer2".to_string(),
            vec![vec![0.0; 2]; 2],
        );
        ewc.fisher = fisher_map;
        ewc.update_anchor(&weights);

        // Deviate layer1: 0.5 -> 0.6 (+0.1 each weight)
        let mut deviated = weights.clone();
        deviated.insert("layer1".to_string(), make_layer_weights(2, 2, 0.6));

        let penalty = ewc.penalty(&deviated);
        // lambda=1.0, fisher=1.0, diff=0.1: per weight = 0.5 * 1.0 * 0.01 = 0.005
        // 4 weights = 0.02
        assert!(penalty > 0.0, "Penalty should be positive when deviating from anchor, got {}", penalty);
        assert!((penalty - 0.02).abs() < 1e-6, "Expected ~0.02, got {}", penalty);
    }

    #[test]
    fn test_update_fisher_accumulates() {
        let mut ewc = EwcRegularizer::new(0.1);
        let grads1 = vec![vec![0.5; 3]; 2];
        let grads2 = vec![vec![1.0; 3]; 2];

        ewc.update_fisher("l1", &grads1);
        let after_first = ewc.fisher["l1"].clone();

        ewc.update_fisher("l1", &grads2);
        let after_second = ewc.fisher["l1"].clone();

        // Fisher should change between updates (decay mix)
        assert!(
            after_first != after_second,
            "Fisher should evolve with multiple updates"
        );
    }

    #[test]
    fn test_update_anchor_overwrites() {
        let mut ewc = EwcRegularizer::new(0.1);
        let w1 = make_weights_map();
        let mut w2 = w1.clone();
        w2.insert("layer2".to_string(), make_layer_weights(2, 2, 0.99));

        ewc.update_anchor(&w1);
        ewc.update_anchor(&w2);

        // After update_anchor(&w2), anchor should match w2
        let penalty = ewc.penalty(&w2);
        assert_eq!(penalty, 0.0, "Penalty should be zero at current anchor");
    }

    #[test]
    fn test_penalty_gradient_shape() {
        let mut ewc = EwcRegularizer::new(0.1);
        let weights = make_weights_map();
        let grads = vec![vec![1.0; 2]; 2];
        ewc.update_fisher("layer1", &grads);
        ewc.update_anchor(&weights);

        let pg = ewc.penalty_gradient("layer1", &weights["layer1"]);
        assert_eq!(pg.len(), 2);
        assert_eq!(pg[0].len(), 2);
        // At anchor with zero fisher, gradient should be... actually with Fisher set,
        // at anchor (w == a) gradient is zero since (w-a) = 0
        for row in &pg {
            for &v in row {
                assert!((v - 0.0).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn test_penalty_gradient_unknown_layer() {
        let ewc = EwcRegularizer::new(0.1);
        let current = make_layer_weights(2, 2, 0.5);
        let pg = ewc.penalty_gradient("nonexistent", &current);
        // Should return zeros
        for row in &pg {
            for &v in row {
                assert_eq!(v, 0.0);
            }
        }
    }

    #[test]
    fn test_default_lambda() {
        let ewc = EwcRegularizer::default();
        assert!((ewc.lambda() - 0.1).abs() < 1e-6);
    }

    #[test]
    fn test_is_initialized() {
        let mut ewc = EwcRegularizer::default();
        assert!(!ewc.is_initialized());
        let weights = make_weights_map();
        ewc.update_anchor(&weights);
        assert!(ewc.is_initialized());
    }

    // ---- Flat-vector legacy API tests ----

    #[test]
    fn test_flat_compute_penalty_zero_uninitialized() {
        let ewc = EwcRegularizer::new(0.1);
        assert_eq!(ewc.compute_penalty(&[1.0; 4]), 0.0);
    }

    #[test]
    fn test_flat_compute_penalty_gradient_zero_uninitialized() {
        let ewc = EwcRegularizer::new(0.1);
        let grad = ewc.compute_penalty_gradient(&[1.0; 4]);
        assert!(grad.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn test_set_lambda() {
        let mut ewc = EwcRegularizer::default();
        ewc.set_lambda(0.5);
        assert!((ewc.lambda() - 0.5).abs() < 1e-6);
    }
}

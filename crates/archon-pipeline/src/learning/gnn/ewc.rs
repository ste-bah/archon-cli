//! Elastic Weight Consolidation (EWC) regularizer.
//!
//! Prevents catastrophic forgetting by penalizing changes to important weights.

/// EWC regularizer state.
#[derive(Debug, Clone)]
pub struct EWCRegularizer {
    /// Saved weights from the previous task.
    saved_weights: Vec<f32>,
    /// Fisher information diagonal (importance of each weight).
    fisher_diagonal: Vec<f32>,
    /// Regularization strength.
    lambda: f32,
}

impl EWCRegularizer {
    /// Create a new EWC regularizer.
    pub fn new(lambda: f32) -> Self {
        Self {
            saved_weights: Vec::new(),
            fisher_diagonal: Vec::new(),
            lambda,
        }
    }

    /// Compute the EWC penalty: lambda/2 * sum(F_i * (theta_i - theta*_i)^2).
    pub fn compute_penalty(&self, current_weights: &[f32]) -> f32 {
        if self.saved_weights.is_empty() || self.fisher_diagonal.is_empty() {
            return 0.0;
        }

        let penalty: f32 = current_weights
            .iter()
            .zip(self.saved_weights.iter())
            .zip(self.fisher_diagonal.iter())
            .map(|((cur, saved), fisher)| {
                let diff = cur - saved;
                fisher * diff * diff
            })
            .sum();

        self.lambda * 0.5 * penalty
    }

    /// Compute gradient of EWC penalty w.r.t. current weights.
    pub fn compute_penalty_gradient(&self, current_weights: &[f32]) -> Vec<f32> {
        if self.saved_weights.is_empty() || self.fisher_diagonal.is_empty() {
            return vec![0.0; current_weights.len()];
        }

        current_weights
            .iter()
            .zip(self.saved_weights.iter())
            .zip(self.fisher_diagonal.iter())
            .map(|((cur, saved), fisher)| self.lambda * fisher * (cur - saved))
            .collect()
    }

    /// Save current weights and update Fisher information from accumulated gradients.
    ///
    /// `weights` — current model weights (flattened).
    /// `gradients` — accumulated squared gradients from recent training (used as Fisher approx).
    pub fn update_fisher_information(&mut self, weights: &[f32], gradients: &[f32]) {
        self.saved_weights = weights.to_vec();

        // Fisher information approximated as mean of squared gradients
        if self.fisher_diagonal.is_empty() {
            self.fisher_diagonal = gradients.to_vec();
        } else {
            // Running average: new_F = 0.5 * old_F + 0.5 * new_grads^2
            self.fisher_diagonal = self
                .fisher_diagonal
                .iter()
                .zip(gradients.iter())
                .map(|(old, new)| 0.5 * old + 0.5 * new)
                .collect();
        }
    }

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
        !self.saved_weights.is_empty()
    }
}

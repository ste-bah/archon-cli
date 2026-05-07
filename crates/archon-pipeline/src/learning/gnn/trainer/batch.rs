use std::collections::HashMap;

use super::super::backprop;
use super::super::loss::{self, TrajectoryWithFeedback};
use super::super::math::ActivationType;
use super::super::{GnnEnhancer, LayerActivationCache};
use super::gradients::zero_grads;
use super::types::{BatchResult, GnnTrainer};

impl GnnTrainer {
    #[allow(clippy::type_complexity)]
    pub(super) fn train_batch(
        &self,
        enhancer: &GnnEnhancer,
        samples: &[TrajectoryWithFeedback],
        batch: &[loss::Triplet],
        embeddings: &[Vec<f32>],
    ) -> BatchResult {
        let mut total_loss = 0.0_f32;
        let mut accumulated_grads: Option<Vec<(Vec<Vec<f32>>, Vec<f32>)>> = None;

        // Collect unique indices in this batch for activation-collected forward pass
        let mut unique_indices: Vec<usize> = Vec::new();
        {
            let mut seen = std::collections::HashSet::new();
            for t in batch {
                if seen.insert(t.anchor) {
                    unique_indices.push(t.anchor);
                }
                if seen.insert(t.positive) {
                    unique_indices.push(t.positive);
                }
                if seen.insert(t.negative) {
                    unique_indices.push(t.negative);
                }
            }
        }

        // Forward pass with activation collection for unique samples
        let mut activations: HashMap<usize, Vec<LayerActivationCache>> = HashMap::new();
        for &idx in &unique_indices {
            let fwd = enhancer.enhance(
                &samples[idx].embedding,
                None,
                None,
                true, // collect activations
            );
            activations.insert(idx, fwd.activation_cache);
        }

        for triplet in batch {
            let emb_a = &embeddings[triplet.anchor];
            let emb_p = &embeddings[triplet.positive];
            let emb_n = &embeddings[triplet.negative];

            let loss_result = loss::compute_loss(emb_a, emb_p, emb_n, self.config.margin);

            total_loss += loss_result.loss;
            if loss_result.loss <= 0.0 {
                continue;
            }

            // Backprop through GNN for anchor embedding
            if let Some(caches) = activations.get(&triplet.anchor)
                && caches.len() == 3
            {
                let (l1, l2, l3) = enhancer.get_weights();
                let grads = backprop::full_backward(
                    caches,
                    [&l1, &l2, &l3],
                    &loss_result.grad_anchor,
                    [
                        ActivationType::LeakyRelu,
                        ActivationType::LeakyRelu,
                        ActivationType::Tanh,
                    ],
                );

                let layer_grads: Vec<(Vec<Vec<f32>>, Vec<f32>)> =
                    grads.into_iter().map(|g| (g.dw, g.db)).collect();

                // Accumulate
                match &mut accumulated_grads {
                    Some(acc) => {
                        for (i, (dw, db)) in layer_grads.iter().enumerate() {
                            for (row_a, row_g) in acc[i].0.iter_mut().zip(dw.iter()) {
                                for (a, g) in row_a.iter_mut().zip(row_g.iter()) {
                                    *a += *g;
                                }
                            }
                            for (a, g) in acc[i].1.iter_mut().zip(db.iter()) {
                                *a += *g;
                            }
                        }
                    }
                    None => {
                        accumulated_grads = Some(layer_grads);
                    }
                }
            }
        }

        let batch_size = batch.len() as f32;
        let avg_loss = if batch.is_empty() {
            0.0
        } else {
            total_loss / batch_size
        };

        // Average gradients
        let grads = accumulated_grads
            .map(|acc| {
                acc.into_iter()
                    .map(|(dw, db)| {
                        let dw: Vec<Vec<f32>> = dw
                            .into_iter()
                            .map(|row| row.into_iter().map(|v| v / batch_size).collect())
                            .collect();
                        let db: Vec<f32> = db.into_iter().map(|v| v / batch_size).collect();
                        (dw, db)
                    })
                    .collect()
            })
            .unwrap_or_else(|| zero_grads(enhancer));

        BatchResult {
            loss: avg_loss,
            grads,
        }
    }

    pub(super) fn apply_gradients(
        &mut self,
        enhancer: &GnnEnhancer,
        grads: &[(Vec<Vec<f32>>, Vec<f32>)],
    ) {
        let mut clipped = grads.to_vec();
        for (dw, db) in &mut clipped {
            backprop::clip_gradient_matrix(dw, self.config.max_gradient_norm);
            backprop::clip_gradients(db, self.config.max_gradient_norm);
        }

        // Add EWC penalty gradients
        if self.ewc.is_initialized() {
            let (l1, l2, l3) = enhancer.get_weights();
            let ewc_layers = [
                self.ewc.penalty_gradient("gnn_embed", &l1.w),
                self.ewc.penalty_gradient("gnn_hidden", &l2.w),
                self.ewc.penalty_gradient("gnn_output", &l3.w),
            ];
            for (i, ewc_grad) in ewc_layers.iter().enumerate() {
                if let Some((dw, _db)) = clipped.get_mut(i) {
                    for (row_dw, row_ewc) in dw.iter_mut().zip(ewc_grad.iter()) {
                        for (d, e) in row_dw.iter_mut().zip(row_ewc.iter()) {
                            *d += *e;
                        }
                    }
                }
            }
        }

        // Apply Adam step
        let (l1, l2, l3) = enhancer.get_weights();
        let mut layers = vec![l1.clone(), l2.clone(), l3.clone()];
        self.optimizer.step(&mut layers, &clipped);
        let l1 = layers.remove(0);
        let l2 = layers.remove(0);
        let l3 = layers.remove(0);
        enhancer.set_weights(l1, l2, l3);
    }

    pub(super) fn compute_triplet_loss(
        &self,
        triplets: &[loss::Triplet],
        embeddings: &[Vec<f32>],
    ) -> f32 {
        if triplets.is_empty() {
            return 0.0;
        }
        let total: f32 = triplets
            .iter()
            .map(|t| {
                loss::compute_loss(
                    &embeddings[t.anchor],
                    &embeddings[t.positive],
                    &embeddings[t.negative],
                    self.config.margin,
                )
                .loss
            })
            .sum();
        total / triplets.len() as f32
    }

    pub(super) fn current_ewc_loss(&self, enhancer: &GnnEnhancer) -> f32 {
        let (l1, l2, l3) = enhancer.get_weights();
        let mut weights = HashMap::new();
        weights.insert("gnn_embed".to_string(), l1.w);
        weights.insert("gnn_hidden".to_string(), l2.w);
        weights.insert("gnn_output".to_string(), l3.w);
        self.ewc.penalty(&weights)
    }
}

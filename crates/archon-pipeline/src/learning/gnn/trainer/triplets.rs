use super::super::GnnEnhancer;
use super::super::triplets_loss::{self, TripletBatch};
use super::gradients::{accumulate_embedding_grads, average_grads, pad_gradient, zero_grads};
use super::types::{BatchResult, GnnTrainer};

impl GnnTrainer {
    pub(super) fn train_meaning_triplet_batch(
        &self,
        enhancer: &GnnEnhancer,
        batch: &TripletBatch,
    ) -> BatchResult {
        if batch.triplets.is_empty() {
            return BatchResult {
                loss: 0.0,
                grads: zero_grads(enhancer),
            };
        }

        let (l1, l2, l3) = enhancer.get_weights();
        let mut total_loss = 0.0_f32;
        let mut accumulated_grads: Option<Vec<(Vec<Vec<f32>>, Vec<f32>)>> = None;

        for triplet in &batch.triplets {
            let anchor = enhancer.enhance(&triplet.anchor, None, None, true);
            let positive = enhancer.enhance(&triplet.positive, None, None, true);
            let negative = enhancer.enhance(&triplet.negative, None, None, true);

            let loss_result = triplets_loss::triplet_loss_gradient(
                &anchor.enhanced,
                &positive.enhanced,
                &negative.enhanced,
                &self.triplet_loss_config,
            );
            total_loss += loss_result.loss;
            if loss_result.loss <= 0.0 {
                continue;
            }

            accumulate_embedding_grads(
                &mut accumulated_grads,
                &anchor.activation_cache,
                [&l1, &l2, &l3],
                &pad_gradient(loss_result.grad_anchor, anchor.enhanced.len()),
            );
            accumulate_embedding_grads(
                &mut accumulated_grads,
                &positive.activation_cache,
                [&l1, &l2, &l3],
                &pad_gradient(loss_result.grad_positive, positive.enhanced.len()),
            );
            accumulate_embedding_grads(
                &mut accumulated_grads,
                &negative.activation_cache,
                [&l1, &l2, &l3],
                &pad_gradient(loss_result.grad_negative, negative.enhanced.len()),
            );
        }

        let batch_size = batch.triplets.len() as f32;
        let grads = accumulated_grads
            .map(|acc| average_grads(acc, batch_size))
            .unwrap_or_else(|| zero_grads(enhancer));

        BatchResult {
            loss: total_loss / batch_size,
            grads,
        }
    }

    pub(super) fn compute_meaning_triplet_loss(
        &self,
        enhancer: &GnnEnhancer,
        batch: &TripletBatch,
    ) -> f32 {
        if batch.triplets.is_empty() {
            return 0.0;
        }
        let total = batch
            .triplets
            .iter()
            .map(|triplet| {
                let anchor = enhancer.enhance(&triplet.anchor, None, None, false);
                let positive = enhancer.enhance(&triplet.positive, None, None, false);
                let negative = enhancer.enhance(&triplet.negative, None, None, false);
                triplets_loss::triplet_loss(
                    &anchor.enhanced,
                    &positive.enhanced,
                    &negative.enhanced,
                    &self.triplet_loss_config,
                )
            })
            .sum::<f32>();
        total / batch.triplets.len() as f32
    }
}

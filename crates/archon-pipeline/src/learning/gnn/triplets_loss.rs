//! Margin-based triplet loss for archon-meaning hydrated triplets.
//!
//! L = max(0, margin + d(anchor, positive) - d(anchor, negative))
//!
//! Squared L2 over the GNN output embedding space. The default margin of 0.2
//! nudges positives closer to the anchor than negatives without dominating the
//! existing trajectory-quality signal.

#[derive(Debug, Clone)]
pub struct TripletLossConfig {
    pub margin: f32,
}

impl Default for TripletLossConfig {
    fn default() -> Self {
        Self { margin: 0.2 }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TripletBatch {
    pub triplets: Vec<archon_meaning::HydratedTriplet>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TripletGradient {
    pub loss: f32,
    pub grad_anchor: Vec<f32>,
    pub grad_positive: Vec<f32>,
    pub grad_negative: Vec<f32>,
}

pub fn triplet_loss(
    anchor: &[f32],
    positive: &[f32],
    negative: &[f32],
    config: &TripletLossConfig,
) -> f32 {
    triplet_loss_gradient(anchor, positive, negative, config).loss
}

pub fn batch_triplet_loss(batch: &TripletBatch, config: &TripletLossConfig) -> f32 {
    if batch.triplets.is_empty() {
        return 0.0;
    }
    batch
        .triplets
        .iter()
        .map(|triplet| {
            triplet_loss(
                &triplet.anchor,
                &triplet.positive,
                &triplet.negative,
                config,
            )
        })
        .sum::<f32>()
        / batch.triplets.len() as f32
}

pub fn triplet_loss_gradient(
    anchor: &[f32],
    positive: &[f32],
    negative: &[f32],
    config: &TripletLossConfig,
) -> TripletGradient {
    let dim = anchor.len().min(positive.len()).min(negative.len());
    if dim == 0 {
        return TripletGradient {
            loss: 0.0,
            grad_anchor: Vec::new(),
            grad_positive: Vec::new(),
            grad_negative: Vec::new(),
        };
    }

    let mut dist_ap = 0.0_f32;
    let mut dist_an = 0.0_f32;
    for idx in 0..dim {
        let ap = anchor[idx] - positive[idx];
        let an = anchor[idx] - negative[idx];
        dist_ap += ap * ap;
        dist_an += an * an;
    }

    let raw = config.margin + dist_ap - dist_an;
    if raw <= 0.0 {
        return TripletGradient {
            loss: 0.0,
            grad_anchor: vec![0.0; dim],
            grad_positive: vec![0.0; dim],
            grad_negative: vec![0.0; dim],
        };
    }

    let mut grad_anchor = vec![0.0; dim];
    let mut grad_positive = vec![0.0; dim];
    let mut grad_negative = vec![0.0; dim];
    for idx in 0..dim {
        let ap = anchor[idx] - positive[idx];
        let an = anchor[idx] - negative[idx];
        grad_anchor[idx] = 2.0 * ap - 2.0 * an;
        grad_positive[idx] = -2.0 * ap;
        grad_negative[idx] = 2.0 * an;
    }

    TripletGradient {
        loss: raw,
        grad_anchor,
        grad_positive,
        grad_negative,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triplet_loss_zero_when_positives_already_closer() {
        let config = TripletLossConfig { margin: 0.2 };
        let loss = triplet_loss(&[0.0], &[0.1], &[0.5], &config);
        assert_eq!(loss, 0.0);
    }

    #[test]
    fn triplet_loss_positive_when_margin_violated() {
        let config = TripletLossConfig { margin: 0.2 };
        let loss = triplet_loss(&[0.0], &[0.5], &[0.1], &config);
        assert!(loss > 0.0);
    }

    #[test]
    fn triplet_loss_gradient_pushes_positive_closer_negative_farther() {
        let config = TripletLossConfig { margin: 0.2 };
        let anchor = vec![0.0];
        let mut positive = vec![0.5];
        let mut negative = vec![0.1];
        let before_positive = squared_distance(&anchor, &positive);
        let before_negative = squared_distance(&anchor, &negative);

        let grad = triplet_loss_gradient(&anchor, &positive, &negative, &config);
        let lr = 0.1;
        for (value, grad) in positive.iter_mut().zip(grad.grad_positive.iter()) {
            *value -= lr * grad;
        }
        for (value, grad) in negative.iter_mut().zip(grad.grad_negative.iter()) {
            *value -= lr * grad;
        }

        assert!(squared_distance(&anchor, &positive) < before_positive);
        assert!(squared_distance(&anchor, &negative) > before_negative);
    }

    fn squared_distance(left: &[f32], right: &[f32]) -> f32 {
        left.iter()
            .zip(right.iter())
            .map(|(left, right)| {
                let diff = left - right;
                diff * diff
            })
            .sum()
    }
}

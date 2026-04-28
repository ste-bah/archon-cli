//! Contrastive / triplet loss with hard mining for GNN training.

use super::math;

/// Result of a triplet loss computation.
#[derive(Debug, Clone)]
pub struct TripletLossResult {
    /// The computed loss value.
    pub loss: f32,
    /// Gradient w.r.t. the anchor embedding.
    pub grad_anchor: Vec<f32>,
    /// Gradient w.r.t. the positive embedding.
    pub grad_positive: Vec<f32>,
    /// Gradient w.r.t. the negative embedding.
    pub grad_negative: Vec<f32>,
}

/// A mined triplet: indices into the embedding list.
#[derive(Debug, Clone)]
pub struct Triplet {
    pub anchor: usize,
    pub positive: usize,
    pub negative: usize,
}

/// Compute triplet loss: max(0, ||a - p||^2 - ||a - n||^2 + margin).
///
/// Returns the loss value and gradients for anchor, positive, and negative.
pub fn compute_loss(
    anchor: &[f32],
    positive: &[f32],
    negative: &[f32],
    margin: f32,
) -> TripletLossResult {
    let ap_diff = math::subtract_vectors(anchor, positive);
    let an_diff = math::subtract_vectors(anchor, negative);

    let dist_ap: f32 = ap_diff.iter().map(|x| x * x).sum();
    let dist_an: f32 = an_diff.iter().map(|x| x * x).sum();

    let raw_loss = dist_ap - dist_an + margin;
    let loss = raw_loss.max(0.0);

    // Gradients are zero when loss <= 0 (margin already satisfied)
    if raw_loss <= 0.0 {
        let dim = anchor.len();
        return TripletLossResult {
            loss: 0.0,
            grad_anchor: vec![0.0; dim],
            grad_positive: vec![0.0; dim],
            grad_negative: vec![0.0; dim],
        };
    }

    // d(loss)/d(anchor)   =  2*(anchor - positive) - 2*(anchor - negative)
    // d(loss)/d(positive)  = -2*(anchor - positive)
    // d(loss)/d(negative)  =  2*(anchor - negative)
    let grad_anchor: Vec<f32> = ap_diff
        .iter()
        .zip(an_diff.iter())
        .map(|(ap, an)| 2.0 * ap - 2.0 * an)
        .collect();

    let grad_positive: Vec<f32> = ap_diff.iter().map(|ap| -2.0 * ap).collect();
    let grad_negative: Vec<f32> = an_diff.iter().map(|an| 2.0 * an).collect();

    TripletLossResult {
        loss,
        grad_anchor,
        grad_positive,
        grad_negative,
    }
}

/// Compute average triplet loss over a batch of triplets.
pub fn batch_triplet_loss(embeddings: &[Vec<f32>], triplets: &[Triplet], margin: f32) -> f32 {
    if triplets.is_empty() {
        return 0.0;
    }
    let total: f32 = triplets
        .iter()
        .map(|t| {
            compute_loss(
                &embeddings[t.anchor],
                &embeddings[t.positive],
                &embeddings[t.negative],
                margin,
            )
            .loss
        })
        .sum();
    total / triplets.len() as f32
}

/// Mine hard triplets from a set of embeddings with labels.
///
/// For each anchor, finds the hardest positive (farthest same-label) and
/// hardest negative (closest different-label).
pub fn mine_triplets(embeddings: &[Vec<f32>], labels: &[u32]) -> Vec<Triplet> {
    let n = embeddings.len();
    if n < 2 {
        return vec![];
    }

    let mut triplets = Vec::new();

    for anchor_idx in 0..n {
        let anchor_label = labels[anchor_idx];

        // Find hardest positive: same label, maximum distance
        let mut best_pos_idx: Option<usize> = None;
        let mut best_pos_dist: f32 = f32::NEG_INFINITY;

        // Find hardest negative: different label, minimum distance
        let mut best_neg_idx: Option<usize> = None;
        let mut best_neg_dist: f32 = f32::INFINITY;

        for other_idx in 0..n {
            if other_idx == anchor_idx {
                continue;
            }
            let diff = math::subtract_vectors(&embeddings[anchor_idx], &embeddings[other_idx]);
            let dist: f32 = diff.iter().map(|x| x * x).sum();

            if labels[other_idx] == anchor_label {
                // Same label -> potential positive
                if dist > best_pos_dist {
                    best_pos_dist = dist;
                    best_pos_idx = Some(other_idx);
                }
            } else {
                // Different label -> potential negative
                if dist < best_neg_dist {
                    best_neg_dist = dist;
                    best_neg_idx = Some(other_idx);
                }
            }
        }

        if let (Some(pos), Some(neg)) = (best_pos_idx, best_neg_idx) {
            triplets.push(Triplet {
                anchor: anchor_idx,
                positive: pos,
                negative: neg,
            });
        }
    }

    triplets
}

/// Simple contrastive loss: ||a - b||^2 when same label, max(0, margin - ||a-b||)^2 when different.
pub fn contrastive_loss(a: &[f32], b: &[f32], same_label: bool, margin: f32) -> f32 {
    let diff = math::subtract_vectors(a, b);
    let dist: f32 = diff.iter().map(|x| x * x).sum::<f32>().sqrt();

    if same_label {
        dist * dist
    } else {
        let gap = (margin - dist).max(0.0);
        gap * gap
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_triplet_loss_zero_when_margin_satisfied() {
        // Anchor near positive, far from negative => loss should be 0
        let anchor = vec![0.0, 0.0, 0.0];
        let positive = vec![0.1, 0.0, 0.0];
        let negative = vec![10.0, 0.0, 0.0];
        let margin = 1.0;

        let result = compute_loss(&anchor, &positive, &negative, margin);
        assert_eq!(result.loss, 0.0, "Loss should be zero when negative is far enough");
        assert!(result.grad_anchor.iter().all(|v| *v == 0.0), "Gradients should be zero when no loss");
        assert!(result.grad_positive.iter().all(|v| *v == 0.0));
        assert!(result.grad_negative.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn test_triplet_loss_positive_when_margin_violated() {
        // Anchor close to negative, far from positive => loss > 0
        let anchor = vec![0.0, 0.0, 0.0];
        let positive = vec![10.0, 0.0, 0.0];
        let negative = vec![0.1, 0.0, 0.0];
        let margin = 1.0;

        let result = compute_loss(&anchor, &positive, &negative, margin);
        assert!(result.loss > 0.0, "Loss should be positive when anchor is closer to negative");
    }

    #[test]
    fn test_gradient_signs() {
        // Gradient w.r.t. anchor should point away from positive, toward negative
        let anchor = vec![0.0, 0.0];
        let positive = vec![1.0, 0.0];
        let negative = vec![0.0, 1.0];
        let margin = 0.5;

        let result = compute_loss(&anchor, &positive, &negative, margin);
        // grad_anchor = 2(a-p) - 2(a-n) = -2p + 2n = (-2, 0) + (0, 2) = (-2, 2)
        assert!((result.grad_anchor[0] + 2.0).abs() < 1e-6, "grad_anchor[0] should point away from positive");
        assert!((result.grad_anchor[1] - 2.0).abs() < 1e-6, "grad_anchor[1] should point toward negative");
    }

    #[test]
    fn test_gradient_positive_points_toward_anchor() {
        // Need anchor far from positive, close to negative for loss to be non-zero
        let anchor = vec![0.0, 0.0];
        let positive = vec![5.0, 0.0];
        let negative = vec![0.1, 0.0];
        let margin = 1.0;

        let result = compute_loss(&anchor, &positive, &negative, margin);
        assert!(result.loss > 0.0, "Loss should be positive when margin is violated");
        // grad_positive = -2(a-p) = -2*(-5, 0) = (10, 0) — toward anchor
        assert!((result.grad_positive[0] - 10.0).abs() < 1e-6);
    }

    #[test]
    fn test_batch_triplet_loss_average() {
        let embeddings = vec![
            vec![0.0, 0.0], // anchor 1
            vec![1.0, 0.0], // positive 1
            vec![0.0, 10.0], // negative 1 (far)
            vec![0.0, 0.0], // anchor 2
            vec![0.0, 1.0], // positive 2
            vec![10.0, 0.0], // negative 2 (far)
        ];
        let triplets = vec![
            Triplet { anchor: 0, positive: 1, negative: 2 },
            Triplet { anchor: 3, positive: 4, negative: 5 },
        ];

        let loss = batch_triplet_loss(&embeddings, &triplets, 1.0);
        assert!(loss >= 0.0, "Batch loss should be non-negative");
        assert!(loss.is_finite(), "Batch loss should be finite");
    }

    #[test]
    fn test_empty_triplets_returns_zero() {
        let embeddings: Vec<Vec<f32>> = vec![];
        let triplets: Vec<Triplet> = vec![];
        let loss = batch_triplet_loss(&embeddings, &triplets, 1.0);
        assert_eq!(loss, 0.0);
    }

    #[test]
    fn test_hard_triplet_mining_produces_valid_indices() {
        let embeddings: Vec<Vec<f32>> = (0..10).map(|i| vec![i as f32; 4]).collect();
        let labels: Vec<u32> = vec![0, 1, 0, 1, 0, 1, 0, 1, 0, 1];
        let triplets = mine_triplets(&embeddings, &labels);

        for t in &triplets {
            assert!(t.anchor < embeddings.len());
            assert!(t.positive < embeddings.len());
            assert!(t.negative < embeddings.len());
            // Anchor and positive should share the same label
            assert_eq!(labels[t.anchor], labels[t.positive],
                "Anchor and positive must have same label");
            // Anchor and negative should have different labels
            assert_ne!(labels[t.anchor], labels[t.negative],
                "Anchor and negative must have different labels");
        }
    }

    #[test]
    fn test_triplet_loss_with_exact_vectors() {
        // Known values for exact verification
        let anchor = vec![1.0, 0.0, 0.0];
        let positive = vec![1.0, 2.0, 0.0];
        let negative = vec![0.0, 0.0, 3.0];
        let margin = 1.0;

        let result = compute_loss(&anchor, &positive, &negative, margin);
        // dist_ap = (1-1)^2 + (0-2)^2 + (0-0)^2 = 4
        // dist_an = (1-0)^2 + (0-0)^2 + (0-3)^2 = 10
        // loss = max(0, 4 - 10 + 1) = max(0, -5) = 0
        assert_eq!(result.loss, 0.0);
        assert!(result.grad_anchor.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn test_contrastive_loss_same_label() {
        let a = vec![0.0, 0.0];
        let b = vec![3.0, 4.0];
        // dist = 5, loss = 25
        let loss = contrastive_loss(&a, &b, true, 1.0);
        assert!((loss - 25.0).abs() < 1e-6);
    }

    #[test]
    fn test_contrastive_loss_different_label_within_margin() {
        let a = vec![0.0, 0.0];
        let b = vec![0.0, 0.5];
        let margin = 2.0;
        // dist = 0.5, gap = max(0, 2 - 0.5) = 1.5, loss = 1.5^2 = 2.25
        let loss = contrastive_loss(&a, &b, false, margin);
        assert!((loss - 2.25).abs() < 1e-6);
    }

    #[test]
    fn test_contrastive_loss_different_label_outside_margin() {
        let a = vec![0.0, 0.0];
        let b = vec![3.0, 4.0];
        // dist = 5, margin = 2, gap = max(0, 2-5) = 0
        let loss = contrastive_loss(&a, &b, false, 2.0);
        assert_eq!(loss, 0.0);
    }
}

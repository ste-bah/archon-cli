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

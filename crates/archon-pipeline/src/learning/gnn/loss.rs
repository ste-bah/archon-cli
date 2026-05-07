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
// Gradient batch — returned to trainer for backprop through GNN
// ---------------------------------------------------------------------------

/// Gradients for anchor, positive, and negative embeddings from triplet loss.
///
/// The trainer uses these to backprop through the GNN to update weights.
pub type GradientBatch = TripletLossResult;

// ---------------------------------------------------------------------------
// Contrastive loss configuration
// ---------------------------------------------------------------------------

/// Strategy for selecting triplets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TripletStrategy {
    /// Pick a random positive and random negative.
    Random,
    /// Pick the hardest negative (closest different-quality sample).
    HardestNegative,
    /// Pick a negative within the margin (semi-hard mining).
    SemiHard,
}

/// Configuration for contrastive/triplet loss.
#[derive(Debug, Clone)]
pub struct ContrastiveLossConfig {
    /// Margin for triplet loss: max(0, d(a,p) - d(a,n) + margin).
    pub margin: f32,
    /// Samples with quality >= this are considered "good" (positive candidates).
    pub positive_quality_threshold: f32,
    /// Samples with quality <= this are considered "bad" (negative candidates).
    pub negative_quality_threshold: f32,
    /// Triplet mining strategy.
    pub triplet_strategy: TripletStrategy,
}

impl Default for ContrastiveLossConfig {
    fn default() -> Self {
        Self {
            margin: 0.5,
            positive_quality_threshold: 0.8,
            negative_quality_threshold: 0.3,
            triplet_strategy: TripletStrategy::HardestNegative,
        }
    }
}

// ---------------------------------------------------------------------------
// Trajectory-based triplet construction
// ---------------------------------------------------------------------------

/// A trajectory sample with its embedding and quality score.
#[derive(Debug, Clone)]
pub struct TrajectoryWithFeedback {
    pub trajectory_id: String,
    pub embedding: Vec<f32>,
    /// Quality score in [0, 1].
    pub quality: f32,
}

/// Build triplets from trajectory samples using quality-based thresholds.
///
/// - **Anchor**: any sample.
/// - **Positive**: sample with `quality >= positive_quality_threshold`, same or
///   different trajectory — picks farthest (hardest positive) when
///   `HardestNegative` strategy is used.
/// - **Negative**: sample with `quality <= negative_quality_threshold`, picks
///   closest (hardest negative) or random based on `triplet_strategy`.
///
/// Returns empty vec if not enough candidates to form valid triplets.
pub fn build_triplets(
    samples: &[TrajectoryWithFeedback],
    cfg: &ContrastiveLossConfig,
) -> Vec<Triplet> {
    if samples.len() < 3 {
        return vec![];
    }

    let good: Vec<&TrajectoryWithFeedback> = samples
        .iter()
        .filter(|s| s.quality >= cfg.positive_quality_threshold)
        .collect();
    let bad: Vec<&TrajectoryWithFeedback> = samples
        .iter()
        .filter(|s| s.quality <= cfg.negative_quality_threshold)
        .collect();

    if good.is_empty() || bad.is_empty() {
        return vec![];
    }

    let mut triplets = Vec::new();

    for anchor in samples {
        // Select positive from good set
        let pos = select_positive(anchor, &good, cfg);
        // Select negative from bad set
        let neg = select_negative(anchor, &bad, cfg);

        if let (Some(p), Some(n)) = (pos, neg) {
            triplets.push(Triplet {
                anchor: anchor_idx(samples, anchor),
                positive: anchor_idx(samples, p),
                negative: anchor_idx(samples, n),
            });
        }
    }

    triplets
}

fn select_positive<'a>(
    anchor: &TrajectoryWithFeedback,
    good: &[&'a TrajectoryWithFeedback],
    _cfg: &ContrastiveLossConfig,
) -> Option<&'a TrajectoryWithFeedback> {
    if good.is_empty() {
        return None;
    }

    // Pick the farthest good sample (hardest positive) for HardestNegative/Random
    let mut best: Option<&'a TrajectoryWithFeedback> = None;
    let mut best_dist = f32::NEG_INFINITY;

    for &g in good.iter() {
        let diff = math::subtract_vectors(&anchor.embedding, &g.embedding);
        let dist: f32 = diff.iter().map(|x| x * x).sum();
        if dist > best_dist {
            best_dist = dist;
            best = Some(g);
        }
    }

    best
}

fn select_negative<'a>(
    anchor: &TrajectoryWithFeedback,
    bad: &[&'a TrajectoryWithFeedback],
    cfg: &ContrastiveLossConfig,
) -> Option<&'a TrajectoryWithFeedback> {
    if bad.is_empty() {
        return None;
    }

    match cfg.triplet_strategy {
        TripletStrategy::Random => Some(bad[0]),
        TripletStrategy::HardestNegative | TripletStrategy::SemiHard => {
            // Pick closest bad sample (hardest negative)
            let mut best: Option<&'a TrajectoryWithFeedback> = None;
            let mut best_dist = f32::INFINITY;

            for &b in bad.iter() {
                let diff = math::subtract_vectors(&anchor.embedding, &b.embedding);
                let dist: f32 = diff.iter().map(|x| x * x).sum();
                if dist < best_dist {
                    best_dist = dist;
                    best = Some(b);
                }
            }

            best
        }
    }
}

fn anchor_idx(samples: &[TrajectoryWithFeedback], target: &TrajectoryWithFeedback) -> usize {
    samples
        .iter()
        .position(|s| s.trajectory_id == target.trajectory_id)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;

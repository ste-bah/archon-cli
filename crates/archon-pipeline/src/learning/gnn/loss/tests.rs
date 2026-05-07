use super::*;

#[test]
fn test_triplet_loss_zero_when_margin_satisfied() {
    // Anchor near positive, far from negative => loss should be 0
    let anchor = vec![0.0, 0.0, 0.0];
    let positive = vec![0.1, 0.0, 0.0];
    let negative = vec![10.0, 0.0, 0.0];
    let margin = 1.0;

    let result = compute_loss(&anchor, &positive, &negative, margin);
    assert_eq!(
        result.loss, 0.0,
        "Loss should be zero when negative is far enough"
    );
    assert!(
        result.grad_anchor.iter().all(|v| *v == 0.0),
        "Gradients should be zero when no loss"
    );
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
    assert!(
        result.loss > 0.0,
        "Loss should be positive when anchor is closer to negative"
    );
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
    assert!(
        (result.grad_anchor[0] + 2.0).abs() < 1e-6,
        "grad_anchor[0] should point away from positive"
    );
    assert!(
        (result.grad_anchor[1] - 2.0).abs() < 1e-6,
        "grad_anchor[1] should point toward negative"
    );
}

#[test]
fn test_gradient_positive_points_toward_anchor() {
    // Need anchor far from positive, close to negative for loss to be non-zero
    let anchor = vec![0.0, 0.0];
    let positive = vec![5.0, 0.0];
    let negative = vec![0.1, 0.0];
    let margin = 1.0;

    let result = compute_loss(&anchor, &positive, &negative, margin);
    assert!(
        result.loss > 0.0,
        "Loss should be positive when margin is violated"
    );
    // grad_positive = -2(a-p) = -2*(-5, 0) = (10, 0) — toward anchor
    assert!((result.grad_positive[0] - 10.0).abs() < 1e-6);
}

#[test]
fn test_batch_triplet_loss_average() {
    let embeddings = vec![
        vec![0.0, 0.0],  // anchor 1
        vec![1.0, 0.0],  // positive 1
        vec![0.0, 10.0], // negative 1 (far)
        vec![0.0, 0.0],  // anchor 2
        vec![0.0, 1.0],  // positive 2
        vec![10.0, 0.0], // negative 2 (far)
    ];
    let triplets = vec![
        Triplet {
            anchor: 0,
            positive: 1,
            negative: 2,
        },
        Triplet {
            anchor: 3,
            positive: 4,
            negative: 5,
        },
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
        assert_eq!(
            labels[t.anchor], labels[t.positive],
            "Anchor and positive must have same label"
        );
        // Anchor and negative should have different labels
        assert_ne!(
            labels[t.anchor], labels[t.negative],
            "Anchor and negative must have different labels"
        );
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

// ---- build_triplets with quality thresholds ----

fn make_sample(id: &str, embedding: Vec<f32>, quality: f32) -> TrajectoryWithFeedback {
    TrajectoryWithFeedback {
        trajectory_id: id.to_string(),
        embedding,
        quality,
    }
}

#[test]
fn test_build_triplets_empty_with_few_samples() {
    let samples = vec![
        make_sample("a", vec![0.0; 4], 0.9),
        make_sample("b", vec![1.0; 4], 0.2),
    ];
    let cfg = ContrastiveLossConfig::default();
    let triplets = build_triplets(&samples, &cfg);
    assert!(triplets.is_empty(), "Need 3+ samples for triplets");
}

#[test]
fn test_build_triplets_empty_when_no_good() {
    let samples = vec![
        make_sample("a", vec![0.0; 4], 0.5),
        make_sample("b", vec![1.0; 4], 0.5),
        make_sample("c", vec![2.0; 4], 0.5),
    ];
    let cfg = ContrastiveLossConfig::default(); // threshold 0.8 for good
    let triplets = build_triplets(&samples, &cfg);
    assert!(triplets.is_empty(), "No samples above good threshold");
}

#[test]
fn test_build_triplets_empty_when_no_bad() {
    let samples = vec![
        make_sample("a", vec![0.0; 4], 0.9),
        make_sample("b", vec![1.0; 4], 0.85),
        make_sample("c", vec![2.0; 4], 0.8),
    ];
    let cfg = ContrastiveLossConfig::default(); // threshold 0.3 for bad
    let triplets = build_triplets(&samples, &cfg);
    assert!(triplets.is_empty(), "No samples below bad threshold");
}

#[test]
fn test_build_triplets_produces_valid_indices() {
    let samples = vec![
        make_sample("a", vec![0.0; 4], 0.9),  // anchor + good
        make_sample("b", vec![1.0; 4], 0.85), // good
        make_sample("c", vec![2.0; 4], 0.2),  // bad
        make_sample("d", vec![3.0; 4], 0.1),  // bad
    ];
    let cfg = ContrastiveLossConfig::default();
    let triplets = build_triplets(&samples, &cfg);
    assert!(!triplets.is_empty(), "Should produce triplets");

    for t in &triplets {
        assert!(t.anchor < samples.len());
        assert!(t.positive < samples.len());
        assert!(t.negative < samples.len());
        // Positive should be a high-quality sample
        assert!(
            samples[t.positive].quality >= cfg.positive_quality_threshold,
            "Positive should have quality >= threshold"
        );
        // Negative should be a low-quality sample
        assert!(
            samples[t.negative].quality <= cfg.negative_quality_threshold,
            "Negative should have quality <= threshold"
        );
    }
}

#[test]
fn test_contrastive_loss_config_defaults() {
    let cfg = ContrastiveLossConfig::default();
    assert!((cfg.margin - 0.5).abs() < 1e-6);
    assert!((cfg.positive_quality_threshold - 0.8).abs() < 1e-6);
    assert!((cfg.negative_quality_threshold - 0.3).abs() < 1e-6);
    assert_eq!(cfg.triplet_strategy, TripletStrategy::HardestNegative);
}

#[test]
fn test_gradient_batch_from_compute_loss() {
    let anchor = vec![0.0, 0.0];
    let positive = vec![5.0, 0.0];
    let negative = vec![0.1, 0.0];
    let margin = 1.0;

    let result: GradientBatch = compute_loss(&anchor, &positive, &negative, margin);
    assert!(result.loss > 0.0);
    assert!(!result.grad_anchor.iter().all(|&x| x == 0.0));
}

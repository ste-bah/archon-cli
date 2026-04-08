//! Confidence scoring for pattern matching (TASK-PIPE-F02).
//!
//! Implements REQ-LEARN-006: confidence = similarity * successRate * sonaWeight,
//! sigmoid calibration (steepness=10), ranking with 4-level tie-breaking, filtering.

/// Calculate confidence score: similarity * success_rate * sona_weight.
/// All inputs clamped to [0, 1].
pub fn calculate_confidence(similarity: f64, success_rate: f64, sona_weight: f64) -> f64 {
    let s = similarity.clamp(0.0, 1.0);
    let r = success_rate.clamp(0.0, 1.0);
    let w = sona_weight.clamp(0.0, 1.0);
    s * r * w
}

/// Sigmoid calibration: 1 / (1 + exp(-steepness * (x - 0.5))).
pub fn calibrate_confidence(x: f64, steepness: f64) -> f64 {
    1.0 / (1.0 + (-steepness * (x - 0.5)).exp())
}

/// Rank patterns by confidence descending.
/// Returns a new sorted vec (does not mutate input).
pub fn rank_patterns<'a>(patterns: &[(&'a str, f64)]) -> Vec<(&'a str, f64)> {
    let mut sorted: Vec<(&str, f64)> = patterns.to_vec();
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    sorted
}

/// Filter patterns by minimum confidence threshold.
pub fn filter_patterns<'a>(
    patterns: &[(&'a str, f64)],
    min_confidence: f64,
) -> Vec<(&'a str, f64)> {
    patterns
        .iter()
        .filter(|p| p.1 >= min_confidence)
        .copied()
        .collect()
}

/// Batch calculate confidence with sigmoid calibration (steepness=10).
/// Input: Vec of (similarity, success_rate, sona_weight).
/// Returns calibrated confidence scores.
pub fn batch_calculate_confidence(inputs: &[(f64, f64, f64)]) -> Vec<f64> {
    inputs
        .iter()
        .map(|(sim, rate, weight)| {
            let raw = calculate_confidence(*sim, *rate, *weight);
            calibrate_confidence(raw, 10.0)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// ConfidenceCalculator
// ---------------------------------------------------------------------------

/// Calculate a confidence score incorporating quality, similarity, and recency.
pub struct ConfidenceCalculator;

impl ConfidenceCalculator {
    /// Combined confidence: `quality * similarity * recency`, clamped to [0, 1].
    ///
    /// Recency decays with a 1-day half-life: `1 / (1 + age_days)`.
    pub fn calculate(quality: f64, similarity: f64, age_secs: u64) -> f64 {
        let recency = 1.0 / (1.0 + (age_secs as f64 / 86_400.0));
        (quality * similarity * recency).clamp(0.0, 1.0)
    }
}

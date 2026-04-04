/// Known model context limits (in tokens).
pub fn model_context_limit(model: &str) -> u64 {
    let lower = model.to_lowercase();
    if lower.contains("opus") && lower.contains("1m") {
        1_000_000
    } else {
        // sonnet, opus (non-1M), haiku, and unknown models all default to 200k
        200_000
    }
}

/// Estimate token count from text using chars/4 heuristic.
pub fn estimate_tokens(text: &str) -> u64 {
    (text.len() as f64 / 4.0).ceil() as u64
}

/// Token counter that calibrates against API-reported usage.
#[derive(Debug, Clone)]
pub struct TokenCounter {
    multiplier: f64,
    samples: u32,
}

impl Default for TokenCounter {
    fn default() -> Self {
        Self {
            multiplier: 4.0, // chars per token (start with 4.0)
            samples: 0,
        }
    }
}

impl TokenCounter {
    /// Estimate tokens for a piece of text.
    pub fn estimate(&self, text: &str) -> u64 {
        (text.len() as f64 / self.multiplier).ceil() as u64
    }

    /// Calibrate the counter against an API-reported token count.
    pub fn calibrate(&mut self, text_len: usize, actual_tokens: u64) {
        if actual_tokens == 0 {
            return;
        }

        let actual_ratio = text_len as f64 / actual_tokens as f64;
        self.samples += 1;

        // Exponential moving average -- converge toward actual
        let weight = if self.samples < 5 { 0.5 } else { 0.2 };
        self.multiplier = self.multiplier * (1.0 - weight) + actual_ratio * weight;
    }

    /// Check if the current usage exceeds the compaction threshold.
    pub fn should_compact(
        &self,
        current_tokens: u64,
        model_limit: u64,
        threshold: f32,
    ) -> bool {
        let limit = (model_limit as f64 * threshold as f64) as u64;
        current_tokens >= limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_limits_correct() {
        assert_eq!(model_context_limit("claude-sonnet-4-6"), 200_000);
        assert_eq!(model_context_limit("claude-opus-4-6"), 200_000);
        assert_eq!(model_context_limit("claude-haiku-4-5"), 200_000);
    }

    #[test]
    fn estimate_tokens_basic() {
        // 400 chars / 4 = 100 tokens
        let text = "a".repeat(400);
        assert_eq!(estimate_tokens(&text), 100);
    }

    #[test]
    fn counter_calibration() {
        let mut counter = TokenCounter::default();
        assert_eq!(counter.multiplier, 4.0);

        // Actual: 1000 chars = 250 tokens -> ratio = 4.0 (same as default)
        counter.calibrate(1000, 250);
        assert!((counter.multiplier - 4.0).abs() < 0.1);

        // Actual: 1000 chars = 333 tokens -> ratio = 3.0
        counter.calibrate(1000, 333);
        // Should move toward 3.0
        assert!(counter.multiplier < 4.0);
    }

    #[test]
    fn should_compact_threshold() {
        let counter = TokenCounter::default();
        // 80% of 200k = 160k
        assert!(!counter.should_compact(100_000, 200_000, 0.80));
        assert!(counter.should_compact(160_000, 200_000, 0.80));
        assert!(counter.should_compact(200_000, 200_000, 0.80));
    }
}

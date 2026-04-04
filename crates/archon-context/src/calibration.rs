use std::collections::VecDeque;

/// Maximum number of calibration samples retained for running average.
const MAX_SAMPLES: usize = 10;

/// Default chars-per-token multiplier (chars / 4 heuristic).
const DEFAULT_MULTIPLIER: f64 = 0.25;

/// Tracks API cache token metrics.
#[derive(Debug, Clone, Default)]
#[must_use]
pub struct CacheStats {
    /// Tokens spent creating new cache entries.
    pub creation_tokens: u64,
    /// Tokens read from cache (free / discounted).
    pub read_tokens: u64,
}

impl CacheStats {
    /// Record a cache creation event.
    pub fn record_creation(&mut self, tokens: u64) {
        self.creation_tokens = self.creation_tokens.saturating_add(tokens);
    }

    /// Record a cache read event.
    pub fn record_read(&mut self, tokens: u64) {
        self.read_tokens = self.read_tokens.saturating_add(tokens);
    }

    /// Total cache-related tokens observed.
    pub fn total(&self) -> u64 {
        self.creation_tokens.saturating_add(self.read_tokens)
    }
}

/// A single calibration sample: estimated character count paired with
/// the actual token count reported by the API.
#[derive(Debug, Clone, Copy)]
struct Sample {
    estimated_chars: usize,
    actual_tokens: u32,
}

/// Calibrates token estimates against real API-reported usage.
///
/// Starts with the standard `chars / 4` heuristic and refines the
/// multiplier via a running average over the last [`MAX_SAMPLES`]
/// request/response pairs.
#[derive(Debug, Clone)]
#[must_use]
pub struct CalibrationState {
    /// Current chars-to-tokens multiplier (tokens = chars * multiplier).
    multiplier: f64,
    /// Ring buffer of recent samples.
    samples: VecDeque<Sample>,
    /// Whether at least one real sample has been ingested.
    calibrated: bool,
    /// Cumulative cache statistics.
    pub cache: CacheStats,
}

impl Default for CalibrationState {
    fn default() -> Self {
        Self {
            multiplier: DEFAULT_MULTIPLIER,
            samples: VecDeque::with_capacity(MAX_SAMPLES),
            calibrated: false,
            cache: CacheStats::default(),
        }
    }
}

impl CalibrationState {
    /// Create a new state with the default `chars / 4` multiplier.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the estimator has been calibrated with at least one real sample.
    pub fn is_calibrated(&self) -> bool {
        self.calibrated
    }

    /// Current multiplier value (tokens = chars * multiplier).
    pub fn multiplier(&self) -> f64 {
        self.multiplier
    }

    /// Number of samples currently retained.
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    /// Record a calibration sample from an API response.
    ///
    /// `estimated_chars` is the character length of the text that was sent.
    /// `actual_tokens` is the token count the API reported for that text.
    ///
    /// Zero-token responses are silently ignored (prevents division by zero).
    pub fn record_sample(&mut self, estimated_chars: usize, actual_tokens: u32) {
        if actual_tokens == 0 {
            return;
        }

        let sample = Sample {
            estimated_chars,
            actual_tokens,
        };

        if self.samples.len() >= MAX_SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
        self.calibrated = true;

        // Recompute multiplier as the average ratio across all retained samples.
        self.recompute_multiplier();
    }

    /// Estimate the token count for `text` using the calibrated multiplier.
    ///
    /// On the first turn (no samples yet), falls back to `chars / 4`.
    pub fn calibrated_estimate(&self, text: &str) -> u32 {
        let len = text.len();
        if len == 0 {
            return 0;
        }
        let tokens = len as f64 * self.multiplier;
        // Ceiling to avoid undercount; clamp to u32 range.
        (tokens.ceil() as u64).min(u32::MAX as u64) as u32
    }

    /// Recompute the multiplier from the running sample window.
    fn recompute_multiplier(&mut self) {
        if self.samples.is_empty() {
            self.multiplier = DEFAULT_MULTIPLIER;
            return;
        }

        let sum: f64 = self
            .samples
            .iter()
            .map(|s| {
                if s.estimated_chars == 0 {
                    DEFAULT_MULTIPLIER
                } else {
                    s.actual_tokens as f64 / s.estimated_chars as f64
                }
            })
            .sum();

        self.multiplier = sum / self.samples.len() as f64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_turn_fallback_uses_chars_div_4() {
        let state = CalibrationState::new();
        assert!(!state.is_calibrated());
        // 400 chars * 0.25 = 100 tokens
        assert_eq!(state.calibrated_estimate(&"a".repeat(400)), 100);
    }

    #[test]
    fn zero_length_input_returns_zero() {
        let state = CalibrationState::new();
        assert_eq!(state.calibrated_estimate(""), 0);

        // Also after calibration
        let mut calibrated = CalibrationState::new();
        calibrated.record_sample(100, 30);
        assert_eq!(calibrated.calibrated_estimate(""), 0);
    }

    #[test]
    fn calibration_converges_to_actual_ratio() {
        let mut state = CalibrationState::new();

        // Suppose the real ratio is 0.30 tokens per char (3.33 chars/token).
        for _ in 0..10 {
            state.record_sample(1000, 300);
        }

        assert!(state.is_calibrated());
        let est = state.calibrated_estimate(&"x".repeat(1000));
        // Should be very close to 300
        assert!(
            (est as i64 - 300).unsigned_abs() <= 1,
            "expected ~300, got {est}"
        );
    }

    #[test]
    fn running_average_stabilises_over_window() {
        let mut state = CalibrationState::new();

        // Feed 9 samples at ratio 0.25, then one outlier at 0.50.
        for _ in 0..9 {
            state.record_sample(1000, 250);
        }
        state.record_sample(1000, 500);

        // Average ratio = (9 * 0.25 + 0.50) / 10 = 0.275
        let expected_multiplier = 0.275;
        assert!(
            (state.multiplier() - expected_multiplier).abs() < 1e-6,
            "expected ~{expected_multiplier}, got {}",
            state.multiplier()
        );
    }

    #[test]
    fn old_samples_evicted_after_max() {
        let mut state = CalibrationState::new();

        // Fill with ratio 0.25
        for _ in 0..MAX_SAMPLES {
            state.record_sample(1000, 250);
        }
        assert_eq!(state.sample_count(), MAX_SAMPLES);

        // Now push 10 more at ratio 0.50 -- should fully replace old window.
        for _ in 0..MAX_SAMPLES {
            state.record_sample(1000, 500);
        }
        assert_eq!(state.sample_count(), MAX_SAMPLES);

        let est = state.calibrated_estimate(&"x".repeat(1000));
        // Should be ~500
        assert!(
            (est as i64 - 500).unsigned_abs() <= 1,
            "expected ~500, got {est}"
        );
    }

    #[test]
    fn cache_stats_tracking() {
        let mut state = CalibrationState::new();

        state.cache.record_creation(1500);
        state.cache.record_read(3000);
        state.cache.record_creation(500);

        assert_eq!(state.cache.creation_tokens, 2000);
        assert_eq!(state.cache.read_tokens, 3000);
        assert_eq!(state.cache.total(), 5000);
    }

    #[test]
    fn zero_actual_tokens_ignored() {
        let mut state = CalibrationState::new();
        state.record_sample(1000, 0);
        assert!(!state.is_calibrated());
        assert_eq!(state.sample_count(), 0);
        // Multiplier unchanged
        assert!((state.multiplier() - DEFAULT_MULTIPLIER).abs() < f64::EPSILON);
    }

    #[test]
    fn cache_stats_saturating() {
        let mut stats = CacheStats::default();
        stats.record_creation(u64::MAX);
        stats.record_creation(1);
        assert_eq!(stats.creation_tokens, u64::MAX);
    }
}

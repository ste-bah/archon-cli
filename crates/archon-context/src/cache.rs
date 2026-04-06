/// Cache hit-rate tracker.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    pub total_input_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub requests: u32,
}

impl CacheStats {
    /// Record a new API response's cache metrics.
    pub fn update(&mut self, creation: u64, read: u64, total_input: u64) {
        self.cache_creation_tokens += creation;
        self.cache_read_tokens += read;
        self.total_input_tokens += total_input;
        self.requests += 1;
    }

    /// Cache hit rate as a percentage (0.0 – 100.0).
    /// Returns 0.0 when no input tokens have been recorded.
    pub fn hit_rate(&self) -> f64 {
        if self.total_input_tokens == 0 {
            return 0.0;
        }
        self.cache_read_tokens as f64 / self.total_input_tokens as f64 * 100.0
    }

    /// Estimated token savings from cache reads.
    /// Cache reads cost 90 % less than regular input, so each cached token
    /// saves 0.9 tokens worth of cost.
    pub fn estimated_savings(&self) -> f64 {
        self.cache_read_tokens as f64 * 0.9
    }

    /// Formatted string suitable for the `/cost` slash command.
    pub fn format_for_cost(&self) -> String {
        format!(
            "Cache hit rate: {:.1}% ({} reads / {} total)\n\
             Cache creation: {} tokens\n\
             Estimated savings: {:.0} token-equivalents",
            self.hit_rate(),
            self.cache_read_tokens,
            self.total_input_tokens,
            self.cache_creation_tokens,
            self.estimated_savings(),
        )
    }
}

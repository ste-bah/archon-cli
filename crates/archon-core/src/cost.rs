use std::collections::HashMap;

/// Per-model pricing (per million tokens).
#[derive(Debug, Clone)]
pub struct ModelPricing {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// Cost tracker for a session.
#[derive(Debug)]
pub struct CostTracker {
    pricing: HashMap<String, ModelPricing>,
    turns: Vec<TurnCost>,
    total_cost: f64,
}

#[derive(Debug, Clone)]
pub struct TurnCost {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost: f64,
}

impl CostTracker {
    pub fn new() -> Self {
        let mut pricing = HashMap::new();

        // Default Anthropic pricing (per million tokens)
        pricing.insert(
            "claude-sonnet-4-6".into(),
            ModelPricing {
                input: 3.0,
                output: 15.0,
                cache_read: 0.30,
                cache_write: 3.75,
            },
        );
        pricing.insert(
            "claude-opus-4-6".into(),
            ModelPricing {
                input: 15.0,
                output: 75.0,
                cache_read: 1.50,
                cache_write: 18.75,
            },
        );
        pricing.insert(
            "claude-haiku-4-5".into(),
            ModelPricing {
                input: 0.80,
                output: 4.0,
                cache_read: 0.08,
                cache_write: 1.0,
            },
        );

        Self {
            pricing,
            turns: Vec::new(),
            total_cost: 0.0,
        }
    }

    /// Add custom pricing for a model.
    pub fn set_pricing(&mut self, model: &str, pricing: ModelPricing) {
        self.pricing.insert(model.to_string(), pricing);
    }

    /// Record a turn's token usage and calculate cost.
    pub fn record_turn(
        &mut self,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
    ) -> f64 {
        let cost = self.calculate_cost(
            model,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        );

        self.turns.push(TurnCost {
            model: model.to_string(),
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
            cost,
        });

        self.total_cost += cost;
        cost
    }

    /// Calculate cost for given token counts (does not record).
    pub fn calculate_cost(
        &self,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
    ) -> f64 {
        let pricing = match self.pricing.get(model) {
            Some(p) => p,
            None => {
                tracing::warn!("no pricing for model '{model}', using zero cost");
                return 0.0;
            }
        };

        let input_cost = (input_tokens as f64 / 1_000_000.0) * pricing.input;
        let output_cost = (output_tokens as f64 / 1_000_000.0) * pricing.output;
        let cache_read_cost = (cache_read_tokens as f64 / 1_000_000.0) * pricing.cache_read;
        let cache_write_cost = (cache_write_tokens as f64 / 1_000_000.0) * pricing.cache_write;

        input_cost + output_cost + cache_read_cost + cache_write_cost
    }

    /// Get total session cost.
    pub fn total_cost(&self) -> f64 {
        self.total_cost
    }

    /// Get all turn costs for detailed breakdown.
    pub fn turns(&self) -> &[TurnCost] {
        &self.turns
    }

    /// Format cost breakdown for display.
    pub fn format_breakdown(&self) -> String {
        let mut output = format!("Total: ${:.4}\n\n", self.total_cost);

        for (i, turn) in self.turns.iter().enumerate() {
            output.push_str(&format!(
                "Turn {}: {} — in:{} out:{} cache_r:{} cache_w:{} — ${:.4}\n",
                i + 1,
                turn.model,
                turn.input_tokens,
                turn.output_tokens,
                turn.cache_read_tokens,
                turn.cache_write_tokens,
                turn.cost,
            ));
        }

        output
    }
}

impl Default for CostTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_calculation_sonnet() {
        let tracker = CostTracker::new();
        // 1000 input, 500 output, no cache
        let cost = tracker.calculate_cost("claude-sonnet-4-6", 1000, 500, 0, 0);
        // (1000/1M)*3.0 + (500/1M)*15.0 = 0.003 + 0.0075 = 0.0105
        assert!((cost - 0.0105).abs() < 0.0001);
    }

    #[test]
    fn cost_calculation_with_cache() {
        let tracker = CostTracker::new();
        let cost = tracker.calculate_cost("claude-sonnet-4-6", 1000, 500, 2000, 1000);
        // input: 0.003, output: 0.0075, cache_read: 0.0006, cache_write: 0.00375
        let expected = 0.003 + 0.0075 + 0.0006 + 0.00375;
        assert!((cost - expected).abs() < 0.0001);
    }

    #[test]
    fn record_turn_accumulates() {
        let mut tracker = CostTracker::new();
        tracker.record_turn("claude-sonnet-4-6", 1000, 500, 0, 0);
        tracker.record_turn("claude-sonnet-4-6", 2000, 1000, 0, 0);

        assert_eq!(tracker.turns().len(), 2);
        assert!(tracker.total_cost() > 0.0);
    }

    #[test]
    fn unknown_model_zero_cost() {
        let tracker = CostTracker::new();
        let cost = tracker.calculate_cost("unknown-model", 1000, 500, 0, 0);
        assert!((cost - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn format_breakdown() {
        let mut tracker = CostTracker::new();
        tracker.record_turn("claude-sonnet-4-6", 1000, 500, 0, 0);
        let breakdown = tracker.format_breakdown();
        assert!(breakdown.contains("Total:"));
        assert!(breakdown.contains("Turn 1:"));
        assert!(breakdown.contains("claude-sonnet-4-6"));
    }
}

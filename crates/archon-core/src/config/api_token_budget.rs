//! Token-budget resolution for [`ApiConfig`].
//!
//! Split from `sections.rs` for the 500-line ceiling; this is one unit with the
//! struct it implements.

use super::ApiConfig;

impl ApiConfig {
    /// The `max_tokens` to put on a request.
    ///
    /// Falls back to `thinking_budget` when unset, reproducing pre-split
    /// behaviour exactly for a config that never mentions `max_tokens`.
    pub fn resolved_max_tokens(&self) -> u32 {
        self.max_tokens.unwrap_or(self.thinking_budget)
    }

    /// Reject a pair that would be refused by the API, at load time rather than
    /// on the first request of a run.
    ///
    /// Anthropic requires `thinking.budget_tokens < max_tokens` — the answer
    /// needs room after the reasoning stops. Sharing one field made that
    /// impossible to violate and equally impossible to tune; splitting them
    /// makes it expressible, so it has to be checked.
    ///
    /// Compares the values that will actually travel: `select_thinking_mode`
    /// clamps the budget into its supported range, so a raw-value check would
    /// pass pairs the clamped ones fail. The clamp is applied by the caller
    /// that knows the model, so the clamped budget is passed in.
    ///
    /// The fallback case can never fail this: it is the pre-split behaviour,
    /// and rejecting it would break every config that predates the field.
    pub fn validate_token_budgets(&self, clamped_budget: u32) -> Result<(), String> {
        let Some(max_tokens) = self.max_tokens else {
            return Ok(());
        };
        if clamped_budget == 0 {
            return Ok(());
        }
        if clamped_budget >= max_tokens {
            return Err(format!(
                "[api] thinking_budget ({clamped_budget} after clamping) must be \
                 less than [api] max_tokens ({max_tokens}): the model needs room \
                 to answer after it stops reasoning, and the request is rejected \
                 otherwise. Raise max_tokens or lower thinking_budget."
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(thinking_budget: u32, max_tokens: Option<u32>) -> ApiConfig {
        ApiConfig {
            thinking_budget,
            max_tokens,
            ..ApiConfig::default()
        }
    }

    /// A config that predates the field behaves exactly as it did: max_tokens
    /// IS the thinking budget. Every existing deployment depends on this.
    #[test]
    fn an_unset_max_tokens_falls_back_to_the_thinking_budget() {
        assert_eq!(config(65_536, None).resolved_max_tokens(), 65_536);
    }

    #[test]
    fn an_explicit_max_tokens_wins() {
        assert_eq!(config(65_536, Some(16_384)).resolved_max_tokens(), 16_384);
    }

    /// The fallback pair is thinking_budget == max_tokens, which the ordering
    /// rule would otherwise reject — it must stay permitted or every config
    /// written before the split fails to load.
    #[test]
    fn the_fallback_pair_is_never_rejected() {
        assert!(config(65_536, None).validate_token_budgets(65_536).is_ok());
    }

    #[test]
    fn a_budget_below_max_tokens_is_accepted() {
        assert!(config(32_768, Some(65_536)).validate_token_budgets(32_768).is_ok());
    }

    /// Equality is a failure, not a boundary: the answer needs room after the
    /// reasoning stops.
    #[test]
    fn a_budget_equal_to_max_tokens_is_rejected() {
        let err = config(16_384, Some(16_384))
            .validate_token_budgets(16_384)
            .expect_err("equal budgets must be rejected");
        assert!(err.contains("must be"), "{err}");
    }

    #[test]
    fn a_budget_above_max_tokens_is_rejected() {
        assert!(
            config(65_536, Some(16_384))
                .validate_token_budgets(65_536)
                .is_err()
        );
    }

    /// The CLAMPED budget is what travels, so it is what gets compared. A raw
    /// value under the ceiling can clamp up past it.
    #[test]
    fn the_clamped_budget_is_what_is_checked() {
        let cfg = config(512, Some(2_048));
        assert!(cfg.validate_token_budgets(512).is_ok());
        assert!(
            cfg.validate_token_budgets(4_096).is_err(),
            "a budget clamped above max_tokens must still be caught"
        );
    }

    /// Thinking disabled is not a budget conflict.
    #[test]
    fn a_zero_budget_is_not_a_conflict() {
        assert!(config(0, Some(4_096)).validate_token_budgets(0).is_ok());
    }
}

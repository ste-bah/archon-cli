const TOKENS_PER_MILLION: f64 = 1_000_000.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Pricing {
    input_cache_miss_per_mtok: f64,
    input_cache_hit_per_mtok: f64,
    output_per_mtok: f64,
}

const LEGACY_SONNET_ESTIMATE: Pricing = Pricing {
    input_cache_miss_per_mtok: 3.0,
    input_cache_hit_per_mtok: 3.0,
    output_per_mtok: 15.0,
};

const DEEPSEEK_V4_PRO: Pricing = Pricing {
    input_cache_miss_per_mtok: 0.435,
    input_cache_hit_per_mtok: 0.003625,
    output_per_mtok: 0.87,
};

const DEEPSEEK_V4_FLASH: Pricing = Pricing {
    input_cache_miss_per_mtok: 0.14,
    input_cache_hit_per_mtok: 0.0028,
    output_per_mtok: 0.28,
};

/// Estimate the current turn cost from provider-reported usage.
///
/// `input_tokens` is the non-cache input reported by the provider for this
/// turn. `cache_creation_tokens` are treated as cache-miss input for DeepSeek.
/// Legacy providers keep the old Sonnet display estimate to avoid surprise
/// churn outside the DeepSeek path.
pub fn estimate_turn_cost_usd(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
) -> f64 {
    let pricing = pricing_for_model(model);
    let cache_miss_input = input_tokens.saturating_add(cache_creation_tokens);
    estimate_with_pricing(
        pricing,
        cache_miss_input,
        output_tokens,
        cache_read_tokens,
        is_deepseek_model(model),
    )
}

/// Estimate cumulative session cost from cumulative context/input counters.
///
/// For DeepSeek, `context_input_tokens` is split into cache-miss and cache-hit
/// buckets because cache hits are priced separately. For legacy providers this
/// preserves the existing rough Sonnet estimate that charged all cumulative
/// input at the same rate.
pub fn estimate_session_cost_usd(
    model: &str,
    context_input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
) -> f64 {
    if is_deepseek_model(model) {
        let cache_miss_input =
            context_input_tokens.saturating_sub(cache_creation_tokens + cache_read_tokens);
        return estimate_turn_cost_usd(
            model,
            cache_miss_input,
            output_tokens,
            cache_creation_tokens,
            cache_read_tokens,
        );
    }

    estimate_with_pricing(
        LEGACY_SONNET_ESTIMATE,
        context_input_tokens,
        output_tokens,
        0,
        false,
    )
}

fn estimate_with_pricing(
    pricing: Pricing,
    input_cache_miss_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    price_cache_hits: bool,
) -> f64 {
    let input_cost =
        input_cache_miss_tokens as f64 * pricing.input_cache_miss_per_mtok / TOKENS_PER_MILLION;
    let cache_cost = if price_cache_hits {
        cache_read_tokens as f64 * pricing.input_cache_hit_per_mtok / TOKENS_PER_MILLION
    } else {
        0.0
    };
    let output_cost = output_tokens as f64 * pricing.output_per_mtok / TOKENS_PER_MILLION;
    input_cost + cache_cost + output_cost
}

fn pricing_for_model(model: &str) -> Pricing {
    let normalized = model.trim().to_ascii_lowercase();
    if normalized.starts_with("deepseek-v4-pro") {
        DEEPSEEK_V4_PRO
    } else if normalized.starts_with("deepseek-v4-flash")
        || normalized == "deepseek-chat"
        || normalized == "deepseek-reasoner"
    {
        DEEPSEEK_V4_FLASH
    } else {
        LEGACY_SONNET_ESTIMATE
    }
}

fn is_deepseek_model(model: &str) -> bool {
    model.trim().to_ascii_lowercase().starts_with("deepseek-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_pro_turn_uses_deepseek_pricing() {
        let cost = estimate_turn_cost_usd("deepseek-v4-pro[1m]", 1_000_000, 1_000_000, 0, 0);

        assert!((cost - 1.305).abs() < f64::EPSILON);
    }

    #[test]
    fn deepseek_pro_cache_hits_are_cheap() {
        let cost = estimate_turn_cost_usd("deepseek-v4-pro[1m]", 0, 0, 0, 1_000_000);

        assert!((cost - 0.003625).abs() < f64::EPSILON);
    }

    #[test]
    fn legacy_estimate_keeps_sonnet_rates() {
        let cost = estimate_turn_cost_usd("claude-sonnet-4-6", 1_000_000, 1_000_000, 0, 0);

        assert!((cost - 18.0).abs() < f64::EPSILON);
    }
}

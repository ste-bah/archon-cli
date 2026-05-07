use super::super::registry::GAMETHEORY_AGENTS;

pub(super) fn agent_tier(agent_key: &str) -> Option<u8> {
    GAMETHEORY_AGENTS
        .iter()
        .find(|agent| agent.key == agent_key)
        .map(|agent| agent.tier)
}

/// Estimate API cost from token usage.
///
/// Rates are documented here to keep Group 5 deterministic in tests:
/// Claude Sonnet family is estimated at $3 / 1M input tokens and $15 / 1M
/// output tokens. Unknown models fall back to the same conservative rate.
pub(super) fn estimate_llm_cost_usd(model: &str, tokens_in: u64, tokens_out: u64) -> f64 {
    let (input_per_million, output_per_million) = if model.contains("sonnet") {
        (3.0, 15.0)
    } else if model.contains("opus") {
        (15.0, 75.0)
    } else {
        (3.0, 15.0)
    };

    (tokens_in as f64 / 1_000_000.0 * input_per_million)
        + (tokens_out as f64 / 1_000_000.0 * output_per_million)
}

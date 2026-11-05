use super::super::registry::GAMETHEORY_AGENTS;

pub(super) fn agent_tier(agent_key: &str) -> Option<u8> {
    GAMETHEORY_AGENTS
        .iter()
        .find(|agent| agent.key == agent_key)
        .map(|agent| agent.tier)
}

/// Pricing comes from the serving client, never from a model-name heuristic.
pub(super) fn estimate_llm_cost_usd(
    client: &dyn crate::runner::LlmClient,
    model: &str,
    input: Option<u64>,
    output: Option<u64>,
) -> Option<f64> {
    client.estimate_cost_usd(&client.resolve_model_alias(model), input?, output?)
}

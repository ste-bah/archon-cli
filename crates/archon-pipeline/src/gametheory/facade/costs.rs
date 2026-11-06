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

/// Refuse a budgeted run before dispatch if any selected model lacks valid pricing.
pub(super) fn require_budget_prices(
    client: &dyn crate::runner::LlmClient,
    agents: &[String],
    budget: f64,
) -> Result<(), super::super::errors::GameTheoryError> {
    if !budget.is_finite() {
        return Ok(());
    }
    for key in agents {
        if let Some(agent) = GAMETHEORY_AGENTS.iter().find(|agent| agent.key == key) {
            let model = client.resolve_model_alias(agent.model);
            if client.estimate_cost_usd(&model, 1, 1).is_none() {
                return Err(super::super::errors::GameTheoryError::SpecialistFailed {
                    agent_key: key.clone(),
                    message: format!(
                        "Cannot enforce game-theory budget: model '{model}' has no valid price; set llm.model_prices.<model> for '{model}' or set the budget to unbounded"
                    ),
                });
            }
        }
    }
    Ok(())
}

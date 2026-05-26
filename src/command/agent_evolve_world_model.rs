use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct WorldModelAgentEvolutionSummary {
    pub signal_count: usize,
    pub evidence_count: usize,
    pub requires_shadow_evaluation: usize,
    pub requires_approval: usize,
}

pub(crate) fn world_model_proposals(
    agent_type: &str,
) -> Vec<archon_core::agents::evolution::AgentEvolutionProposal> {
    world_model_signals(agent_type)
        .into_iter()
        .map(|signal| {
            signal.evidence_ids.iter().fold(
                archon_core::agents::evolution::AgentEvolutionProposal::new(
                    agent_type,
                    "world-model-observed",
                    "world-model-proposed",
                    archon_core::agents::evolution::AgentEvolutionProposalKind::QualityGateProfile,
                    format!(
                        "+ add guardrail for repeated world-model `{}` signal",
                        signal.signal_kind
                    ),
                    format!(
                        "World model observed {} `{}` events; run shadow evaluation before approval.",
                        signal.observation_count, signal.signal_kind
                    ),
                )
                .with_risk_level(archon_core::agents::evolution::AgentEvolutionRiskLevel::High),
                |proposal, evidence_id| {
                    proposal.add_evidence(format!("world_model:{evidence_id}"))
                },
            )
        })
        .collect()
}

pub(crate) fn world_model_summary(agent_type: &str) -> WorldModelAgentEvolutionSummary {
    let signals = world_model_signals(agent_type);
    WorldModelAgentEvolutionSummary {
        signal_count: signals.len(),
        evidence_count: signals.iter().map(|signal| signal.evidence_ids.len()).sum(),
        requires_shadow_evaluation: signals
            .iter()
            .filter(|signal| signal.requires_shadow_evaluation)
            .count(),
        requires_approval: signals
            .iter()
            .filter(|signal| signal.requires_approval)
            .count(),
    }
}

pub(crate) fn world_model_shadow_evidence(agent_type: &str) -> serde_json::Value {
    serde_json::json!({
        "signals": world_model_signals(agent_type),
        "summary": world_model_summary(agent_type),
    })
}

fn world_model_signals(
    agent_type: &str,
) -> Vec<archon_world_model::evolution::WorldModelEvolutionSignal> {
    let rows = load_world_rows_fail_open();
    archon_world_model::evolution::repeated_risk_signals(&rows, 3)
        .into_iter()
        .filter(|signal| signal.agent_type == agent_type)
        .collect()
}

fn load_world_rows_fail_open() -> Vec<archon_world_model::WorldTraceRow> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(load_world_rows))
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default()
}

fn load_world_rows() -> Result<Vec<archon_world_model::WorldTraceRow>> {
    let root = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("home directory unavailable"))?
        .join(".archon")
        .join("world-model");
    archon_world_model::storage::WorldModelStore::open(root)?.load_rows()
}

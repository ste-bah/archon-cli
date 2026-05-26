use std::path::{Path, PathBuf};

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
    summary_from_signals(&signals)
}

pub(crate) fn world_model_shadow_evidence(agent_type: &str) -> serde_json::Value {
    let signals = world_model_signals(agent_type);
    serde_json::json!({
        "signals": signals,
        "summary": summary_from_signals(&signals),
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
    let root = match world_model_root() {
        Ok(root) => root,
        Err(error) => {
            tracing::warn!(error = %error, "world-model shadow evidence unavailable");
            return Vec::new();
        }
    };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| load_world_rows_at(&root))) {
        Ok(Ok(rows)) => {
            let _ = archon_world_model::storage::deferred_retry::clear_shadow_evidence_retry(&root);
            rows
        }
        Ok(Err(error)) => {
            record_deferred_shadow_retry(&root, error.to_string());
            Vec::new()
        }
        Err(payload) => {
            record_deferred_shadow_retry(&root, panic_payload_message(payload));
            Vec::new()
        }
    }
}

fn record_deferred_shadow_retry(root: &Path, reason: String) {
    if let Err(error) =
        archon_world_model::storage::deferred_retry::record_shadow_evidence_retry(root, reason)
    {
        tracing::warn!(
            error = %error,
            "failed to record deferred world-model shadow-evidence retry"
        );
    }
}

fn panic_payload_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else {
        "world-model shadow evidence panicked".to_owned()
    }
}

fn load_world_rows_at(root: &Path) -> Result<Vec<archon_world_model::WorldTraceRow>> {
    archon_world_model::storage::WorldModelStore::open(root)?.load_rows()
}

fn world_model_root() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("home directory unavailable"))?
        .join(".archon")
        .join("world-model"))
}

fn summary_from_signals(
    signals: &[archon_world_model::evolution::WorldModelEvolutionSignal],
) -> WorldModelAgentEvolutionSummary {
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

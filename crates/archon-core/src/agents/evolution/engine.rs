use std::collections::HashMap;

use super::ledger::AgentPerformanceEvent;
use super::proposal::{AgentEvolutionProposal, AgentEvolutionProposalKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentEvolutionRuntimeConfig {
    pub min_corrections: usize,
    pub min_gate_failures: usize,
    pub min_provider_incidents: usize,
    pub min_positive_model_signals: usize,
}

impl Default for AgentEvolutionRuntimeConfig {
    fn default() -> Self {
        Self {
            min_corrections: 3,
            min_gate_failures: 3,
            min_provider_incidents: 3,
            min_positive_model_signals: 5,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentEvolutionRuntime {
    config: AgentEvolutionRuntimeConfig,
}

impl AgentEvolutionRuntime {
    pub fn new(config: AgentEvolutionRuntimeConfig) -> Self {
        Self { config }
    }

    pub fn propose(&self, events: &[AgentPerformanceEvent]) -> Vec<AgentEvolutionProposal> {
        let mut proposals = Vec::new();
        proposals.extend(self.prompt_profile_proposals(events));
        proposals.extend(self.quality_gate_proposals(events));
        proposals.extend(self.provider_incident_proposals(events));
        proposals.extend(self.model_profile_proposals(events));
        proposals
    }

    fn prompt_profile_proposals(
        &self,
        events: &[AgentPerformanceEvent],
    ) -> Vec<AgentEvolutionProposal> {
        let mut corrections: HashMap<AgentVersionKey, Vec<&AgentPerformanceEvent>> = HashMap::new();

        for event in events {
            if event.user_corrected == Some(true) {
                corrections
                    .entry(agent_version_key(event))
                    .or_default()
                    .push(event);
            }
        }

        corrections
            .into_iter()
            .filter(|(_, events)| events.len() >= self.config.min_corrections)
            .map(|(key, events)| {
                let owned_events = clone_events(events);
                AgentEvolutionProposal::from_ledger_pattern(
                    key.agent_type,
                    key.agent_version.clone(),
                    proposed_version(&key.agent_version),
                    AgentEvolutionProposalKind::PromptProfile,
                    "+ review repeated user corrections before changing prompt text",
                    "Reduce repeated user corrections for the same agent profile",
                    &owned_events,
                )
            })
            .collect()
    }

    fn quality_gate_proposals(
        &self,
        events: &[AgentPerformanceEvent],
    ) -> Vec<AgentEvolutionProposal> {
        let mut failures: HashMap<GateKey, Vec<&AgentPerformanceEvent>> = HashMap::new();

        for event in events {
            if let Some(gate) = &event.gate_failed {
                failures
                    .entry(GateKey {
                        agent: agent_version_key(event),
                        gate: gate.clone(),
                    })
                    .or_default()
                    .push(event);
            }
        }

        failures
            .into_iter()
            .filter(|(_, events)| events.len() >= self.config.min_gate_failures)
            .map(|(key, events)| {
                let owned_events = clone_events(events);
                AgentEvolutionProposal::from_ledger_pattern(
                    key.agent.agent_type,
                    key.agent.agent_version.clone(),
                    proposed_version(&key.agent.agent_version),
                    AgentEvolutionProposalKind::QualityGateProfile,
                    format!("+ tighten or add remediation for gate `{}`", key.gate),
                    "Reduce repeated gate failures before agent output is trusted",
                    &owned_events,
                )
            })
            .collect()
    }

    fn provider_incident_proposals(
        &self,
        events: &[AgentPerformanceEvent],
    ) -> Vec<AgentEvolutionProposal> {
        let mut incidents: HashMap<ProviderKey, Vec<&AgentPerformanceEvent>> = HashMap::new();

        for event in events {
            if event.provider_incident_id.is_some() {
                incidents
                    .entry(provider_key(event))
                    .or_default()
                    .push(event);
            }
        }

        incidents
            .into_iter()
            .filter(|(_, events)| events.len() >= self.config.min_provider_incidents)
            .map(|(key, events)| {
                let owned_events = clone_events(events);
                AgentEvolutionProposal::from_ledger_pattern(
                    key.agent.agent_type,
                    key.agent.agent_version.clone(),
                    proposed_version(&key.agent.agent_version),
                    AgentEvolutionProposalKind::ModelProfile,
                    format!(
                        "+ review model/provider profile `{}` / `{}` after repeated incidents",
                        key.model_id, key.provider_id
                    ),
                    "Reduce provider incident impact on agent runs",
                    &owned_events,
                )
                .with_provider_identity_impact()
            })
            .collect()
    }

    fn model_profile_proposals(
        &self,
        events: &[AgentPerformanceEvent],
    ) -> Vec<AgentEvolutionProposal> {
        let mut positives: HashMap<ProviderKey, Vec<&AgentPerformanceEvent>> = HashMap::new();

        for event in events {
            if event.is_positive_signal() && event.model_id.is_some() {
                positives
                    .entry(provider_key(event))
                    .or_default()
                    .push(event);
            }
        }

        positives
            .into_iter()
            .filter(|(_, events)| events.len() >= self.config.min_positive_model_signals)
            .map(|(key, events)| {
                let owned_events = clone_events(events);
                AgentEvolutionProposal::from_ledger_pattern(
                    key.agent.agent_type,
                    key.agent.agent_version.clone(),
                    proposed_version(&key.agent.agent_version),
                    AgentEvolutionProposalKind::ModelProfile,
                    format!(
                        "+ consider model/provider profile `{}` / `{}` based on positive outcomes",
                        key.model_id, key.provider_id
                    ),
                    "Promote a repeatedly successful model/profile only through governed review",
                    &owned_events,
                )
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AgentVersionKey {
    agent_type: String,
    agent_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GateKey {
    agent: AgentVersionKey,
    gate: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProviderKey {
    agent: AgentVersionKey,
    model_id: String,
    provider_id: String,
}

fn agent_version_key(event: &AgentPerformanceEvent) -> AgentVersionKey {
    AgentVersionKey {
        agent_type: event.agent_type.clone(),
        agent_version: event
            .agent_version
            .clone()
            .unwrap_or_else(|| "unversioned".to_string()),
    }
}

fn provider_key(event: &AgentPerformanceEvent) -> ProviderKey {
    ProviderKey {
        agent: agent_version_key(event),
        model_id: event
            .model_id
            .clone()
            .unwrap_or_else(|| "unspecified-model".to_string()),
        provider_id: event
            .provider_id
            .clone()
            .unwrap_or_else(|| "unspecified-provider".to_string()),
    }
}

fn proposed_version(current_version: &str) -> String {
    format!("{current_version}+evo")
}

fn clone_events(events: Vec<&AgentPerformanceEvent>) -> Vec<AgentPerformanceEvent> {
    events.into_iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::evolution::ledger::AgentCompletionStatus;
    use crate::agents::evolution::proposal::AgentEvolutionRiskLevel;

    fn runtime() -> AgentEvolutionRuntime {
        AgentEvolutionRuntime::new(AgentEvolutionRuntimeConfig::default())
    }

    fn base_event() -> AgentPerformanceEvent {
        AgentPerformanceEvent::new("researcher").with_agent_version("agentv-1")
    }

    #[test]
    fn repeated_user_corrections_create_prompt_proposal() {
        let events = vec![
            base_event().with_user_feedback(Some(false), Some(true)),
            base_event().with_user_feedback(Some(false), Some(true)),
            base_event().with_user_feedback(Some(false), Some(true)),
        ];

        let proposals = runtime().propose(&events);

        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].kind, AgentEvolutionProposalKind::PromptProfile);
        assert_eq!(proposals[0].risk_level, AgentEvolutionRiskLevel::High);
        assert_eq!(proposals[0].evidence_ids.len(), 3);
    }

    #[test]
    fn repeated_gate_failures_create_quality_gate_proposal() {
        let events = vec![
            base_event().with_gate_failed("source_check"),
            base_event().with_gate_failed("source_check"),
            base_event().with_gate_failed("source_check"),
        ];

        let proposals = runtime().propose(&events);

        assert_eq!(proposals.len(), 1);
        assert_eq!(
            proposals[0].kind,
            AgentEvolutionProposalKind::QualityGateProfile
        );
        assert!(proposals[0].diff.contains("source_check"));
    }

    #[test]
    fn provider_incidents_are_provider_identity_sensitive() {
        let events = vec![
            base_event()
                .with_model_provider("claude-sonnet-4-6", "anthropic")
                .with_provider_incident("prov-1"),
            base_event()
                .with_model_provider("claude-sonnet-4-6", "anthropic")
                .with_provider_incident("prov-2"),
            base_event()
                .with_model_provider("claude-sonnet-4-6", "anthropic")
                .with_provider_incident("prov-3"),
        ];

        let proposals = runtime().propose(&events);

        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].kind, AgentEvolutionProposalKind::ModelProfile);
        assert!(proposals[0].affects_provider_identity);
        assert!(proposals[0].requires_approval());
    }

    #[test]
    fn positive_model_patterns_create_low_risk_model_proposal() {
        let events: Vec<AgentPerformanceEvent> = (0..5)
            .map(|_| {
                base_event()
                    .with_model_provider("gpt-5.5", "openai")
                    .with_completion_status(AgentCompletionStatus::Succeeded)
                    .with_scores(None, None, Some(0.92), None)
            })
            .collect();

        let proposals = runtime().propose(&events);

        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].kind, AgentEvolutionProposalKind::ModelProfile);
        assert_eq!(proposals[0].risk_level, AgentEvolutionRiskLevel::Low);
        assert!(!proposals[0].requires_approval());
    }

    #[test]
    fn below_threshold_patterns_do_not_emit_proposals() {
        let events = vec![
            base_event().with_user_feedback(Some(false), Some(true)),
            base_event().with_user_feedback(Some(false), Some(true)),
        ];

        assert!(runtime().propose(&events).is_empty());
    }
}

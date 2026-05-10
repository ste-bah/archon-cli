use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::ledger::AgentPerformanceEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEvolutionProposalKind {
    PromptProfile,
    AgentRoutingProfile,
    ToolAccessProfile,
    MemoryProfile,
    ModelProfile,
    QualityGateProfile,
    SkillProfile,
}

impl AgentEvolutionProposalKind {
    pub fn default_risk_level(self) -> AgentEvolutionRiskLevel {
        match self {
            Self::ModelProfile => AgentEvolutionRiskLevel::Low,
            Self::AgentRoutingProfile | Self::MemoryProfile => AgentEvolutionRiskLevel::Medium,
            Self::PromptProfile
            | Self::ToolAccessProfile
            | Self::QualityGateProfile
            | Self::SkillProfile => AgentEvolutionRiskLevel::High,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEvolutionRiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl AgentEvolutionRiskLevel {
    pub fn requires_approval(self) -> bool {
        matches!(self, Self::High | Self::Critical)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEvolutionPolicyDecision {
    EligibleForAutoApply,
    PendingApproval,
    Denied,
}

impl AgentEvolutionPolicyDecision {
    pub fn initial_for_risk(risk_level: AgentEvolutionRiskLevel) -> Self {
        if risk_level.requires_approval() {
            Self::PendingApproval
        } else {
            Self::EligibleForAutoApply
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentEvolutionStatus {
    Pending,
    Approved,
    Rejected,
    Applied,
    RolledBack,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentEvolutionProposal {
    pub proposal_id: String,
    pub agent_type: String,
    pub current_version: String,
    pub proposed_version: String,
    pub kind: AgentEvolutionProposalKind,
    pub diff: String,
    pub evidence_ids: Vec<String>,
    pub risk_level: AgentEvolutionRiskLevel,
    pub policy_decision: AgentEvolutionPolicyDecision,
    pub status: AgentEvolutionStatus,
    pub expected_impact: String,
    pub rollback_target_version: String,
    pub affects_provider_identity: bool,
    pub affects_permissions: bool,
    pub created_at: DateTime<Utc>,
}

impl AgentEvolutionProposal {
    pub fn new(
        agent_type: impl Into<String>,
        current_version: impl Into<String>,
        proposed_version: impl Into<String>,
        kind: AgentEvolutionProposalKind,
        diff: impl Into<String>,
        expected_impact: impl Into<String>,
    ) -> Self {
        let risk_level = kind.default_risk_level();
        let current_version = current_version.into();

        Self {
            proposal_id: agent_evolution_proposal_id(),
            agent_type: agent_type.into(),
            current_version: current_version.clone(),
            proposed_version: proposed_version.into(),
            kind,
            diff: diff.into(),
            evidence_ids: Vec::new(),
            risk_level,
            policy_decision: AgentEvolutionPolicyDecision::initial_for_risk(risk_level),
            status: AgentEvolutionStatus::Pending,
            expected_impact: expected_impact.into(),
            rollback_target_version: current_version,
            affects_provider_identity: false,
            affects_permissions: kind == AgentEvolutionProposalKind::ToolAccessProfile,
            created_at: Utc::now(),
        }
    }

    pub fn from_ledger_pattern(
        agent_type: impl Into<String>,
        current_version: impl Into<String>,
        proposed_version: impl Into<String>,
        kind: AgentEvolutionProposalKind,
        diff: impl Into<String>,
        expected_impact: impl Into<String>,
        events: &[AgentPerformanceEvent],
    ) -> Self {
        events.iter().fold(
            Self::new(
                agent_type,
                current_version,
                proposed_version,
                kind,
                diff,
                expected_impact,
            ),
            |proposal, event| proposal.add_evidence(&event.event_id),
        )
    }

    pub fn add_evidence(mut self, evidence_id: impl Into<String>) -> Self {
        let evidence_id = evidence_id.into();
        if !self
            .evidence_ids
            .iter()
            .any(|existing| existing == &evidence_id)
        {
            self.evidence_ids.push(evidence_id);
        }
        self
    }

    pub fn with_permission_impact(mut self, critical: bool) -> Self {
        self.affects_permissions = true;
        self.escalate_risk(if critical {
            AgentEvolutionRiskLevel::Critical
        } else {
            AgentEvolutionRiskLevel::High
        })
    }

    pub fn with_provider_identity_impact(mut self) -> Self {
        self.affects_provider_identity = true;
        self.escalate_risk(AgentEvolutionRiskLevel::High)
    }

    pub fn with_risk_level(mut self, risk_level: AgentEvolutionRiskLevel) -> Self {
        self.risk_level = risk_level;
        self.policy_decision = AgentEvolutionPolicyDecision::initial_for_risk(risk_level);
        self
    }

    pub fn requires_approval(&self) -> bool {
        self.risk_level.requires_approval()
            || self.policy_decision == AgentEvolutionPolicyDecision::PendingApproval
    }

    fn escalate_risk(mut self, minimum: AgentEvolutionRiskLevel) -> Self {
        if self.risk_level < minimum {
            self.risk_level = minimum;
            self.policy_decision = AgentEvolutionPolicyDecision::initial_for_risk(minimum);
        }
        self
    }
}

pub fn agent_evolution_proposal_id() -> String {
    format!("agent-evo-prop-{}", uuid::Uuid::new_v4())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_profile_proposals_are_high_risk_and_pending() {
        let proposal = AgentEvolutionProposal::new(
            "researcher",
            "agentv-12",
            "agentv-13",
            AgentEvolutionProposalKind::PromptProfile,
            "+ require provenance before claim synthesis",
            "Reduce unsupported claims",
        );

        assert!(proposal.proposal_id.starts_with("agent-evo-prop-"));
        assert_eq!(proposal.rollback_target_version, "agentv-12");
        assert_eq!(proposal.risk_level, AgentEvolutionRiskLevel::High);
        assert_eq!(
            proposal.policy_decision,
            AgentEvolutionPolicyDecision::PendingApproval
        );
        assert!(proposal.requires_approval());
    }

    #[test]
    fn low_risk_model_profile_still_escalates_for_provider_identity() {
        let proposal = AgentEvolutionProposal::new(
            "planner",
            "agentv-1",
            "agentv-2",
            AgentEvolutionProposalKind::ModelProfile,
            "+ effort: high",
            "Prefer the successful effort profile",
        )
        .with_provider_identity_impact();

        assert_eq!(proposal.risk_level, AgentEvolutionRiskLevel::High);
        assert!(proposal.affects_provider_identity);
        assert!(proposal.requires_approval());
    }

    #[test]
    fn permission_impact_can_escalate_to_critical() {
        let proposal = AgentEvolutionProposal::new(
            "verifier",
            "agentv-4",
            "agentv-5",
            AgentEvolutionProposalKind::MemoryProfile,
            "+ recall query",
            "Improve recall",
        )
        .with_permission_impact(true);

        assert_eq!(proposal.risk_level, AgentEvolutionRiskLevel::Critical);
        assert!(proposal.affects_permissions);
        assert_eq!(
            proposal.policy_decision,
            AgentEvolutionPolicyDecision::PendingApproval
        );
    }

    #[test]
    fn explicit_risk_level_recomputes_policy_decision() {
        let proposal = AgentEvolutionProposal::new(
            "verifier",
            "agentv-4",
            "agentv-5",
            AgentEvolutionProposalKind::ToolAccessProfile,
            "- disallow risky tool",
            "Reduce blast radius",
        )
        .with_risk_level(AgentEvolutionRiskLevel::Medium);

        assert_eq!(proposal.risk_level, AgentEvolutionRiskLevel::Medium);
        assert_eq!(
            proposal.policy_decision,
            AgentEvolutionPolicyDecision::EligibleForAutoApply
        );
    }

    #[test]
    fn ledger_pattern_deduplicates_evidence() {
        let event = AgentPerformanceEvent::new("coder");
        let duplicate = event.clone();
        let proposal = AgentEvolutionProposal::from_ledger_pattern(
            "coder",
            "agentv-1",
            "agentv-2",
            AgentEvolutionProposalKind::QualityGateProfile,
            "+ tighten test gate",
            "Reduce repeated test failures",
            &[event, duplicate],
        );

        assert_eq!(proposal.evidence_ids.len(), 1);
        assert_eq!(
            proposal.kind,
            AgentEvolutionProposalKind::QualityGateProfile
        );
        assert!(proposal.requires_approval());
    }
}

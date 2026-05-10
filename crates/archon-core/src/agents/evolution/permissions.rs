use serde::{Deserialize, Serialize};

use crate::agents::definition::PermissionMode;

use super::proposal::{
    AgentEvolutionProposal, AgentEvolutionProposalKind, AgentEvolutionRiskLevel,
};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolAccessProfileDiff {
    pub added_tools: Vec<String>,
    pub removed_tools: Vec<String>,
    pub added_mcp_servers: Vec<String>,
    pub removed_mcp_servers: Vec<String>,
    pub added_skills: Vec<String>,
    pub removed_skills: Vec<String>,
    pub hooks_added: bool,
    pub permission_mode_from: Option<PermissionMode>,
    pub permission_mode_to: Option<PermissionMode>,
    pub sandbox_backend_from: Option<String>,
    pub sandbox_backend_to: Option<String>,
}

impl ToolAccessProfileDiff {
    pub fn risk_level(&self) -> AgentEvolutionRiskLevel {
        if self.requests_bypass_permissions() {
            return AgentEvolutionRiskLevel::Critical;
        }

        if self.expands_permissions() {
            return AgentEvolutionRiskLevel::High;
        }

        if self.reduces_permissions() {
            return AgentEvolutionRiskLevel::Medium;
        }

        AgentEvolutionRiskLevel::Low
    }

    pub fn expands_permissions(&self) -> bool {
        !self.added_tools.is_empty()
            || !self.added_mcp_servers.is_empty()
            || !self.added_skills.is_empty()
            || self.hooks_added
            || self.permission_mode_expands()
            || self.sandbox_backend_loosens()
    }

    pub fn reduces_permissions(&self) -> bool {
        !self.removed_tools.is_empty()
            || !self.removed_mcp_servers.is_empty()
            || !self.removed_skills.is_empty()
            || self.permission_mode_tightens()
    }

    pub fn requests_bypass_permissions(&self) -> bool {
        self.permission_mode_to == Some(PermissionMode::BypassPermissions)
    }

    pub fn to_proposal(
        &self,
        agent_type: impl Into<String>,
        current_version: impl Into<String>,
        proposed_version: impl Into<String>,
        evidence_ids: impl IntoIterator<Item = String>,
    ) -> AgentEvolutionProposal {
        let risk_level = self.risk_level();
        let proposal = AgentEvolutionProposal::new(
            agent_type,
            current_version,
            proposed_version,
            AgentEvolutionProposalKind::ToolAccessProfile,
            self.diff_summary(),
            self.expected_impact(),
        )
        .with_risk_level(risk_level);

        evidence_ids
            .into_iter()
            .fold(proposal, |proposal, evidence_id| {
                proposal.add_evidence(evidence_id)
            })
    }

    fn permission_mode_expands(&self) -> bool {
        match (&self.permission_mode_from, &self.permission_mode_to) {
            (Some(from), Some(to)) => permission_mode_rank(to) > permission_mode_rank(from),
            (None, Some(to)) => {
                permission_mode_rank(to) > permission_mode_rank(&PermissionMode::Default)
            }
            _ => false,
        }
    }

    fn permission_mode_tightens(&self) -> bool {
        match (&self.permission_mode_from, &self.permission_mode_to) {
            (Some(from), Some(to)) => permission_mode_rank(to) < permission_mode_rank(from),
            _ => false,
        }
    }

    fn sandbox_backend_loosens(&self) -> bool {
        match (&self.sandbox_backend_from, &self.sandbox_backend_to) {
            (Some(from), Some(to)) => sandbox_strictness(to) < sandbox_strictness(from),
            _ => false,
        }
    }

    fn diff_summary(&self) -> String {
        serde_json::json!({
            "added_tools": self.added_tools,
            "removed_tools": self.removed_tools,
            "added_mcp_servers": self.added_mcp_servers,
            "removed_mcp_servers": self.removed_mcp_servers,
            "added_skills": self.added_skills,
            "removed_skills": self.removed_skills,
            "hooks_added": self.hooks_added,
            "permission_mode_from": self.permission_mode_from.as_ref().map(PermissionMode::as_str),
            "permission_mode_to": self.permission_mode_to.as_ref().map(PermissionMode::as_str),
            "sandbox_backend_from": self.sandbox_backend_from,
            "sandbox_backend_to": self.sandbox_backend_to,
        })
        .to_string()
    }

    fn expected_impact(&self) -> String {
        match self.risk_level() {
            AgentEvolutionRiskLevel::Critical => {
                "Requires explicit dangerous-bypass approval before any durable change".to_string()
            }
            AgentEvolutionRiskLevel::High => {
                "Requires review because the profile expands executable, MCP, skill, hook, mode, or sandbox access".to_string()
            }
            AgentEvolutionRiskLevel::Medium => {
                "Reduces or tightens access while keeping an inspectable proposal trail".to_string()
            }
            AgentEvolutionRiskLevel::Low => "No effective permission change detected".to_string(),
        }
    }
}

fn permission_mode_rank(mode: &PermissionMode) -> u8 {
    match mode {
        PermissionMode::Plan => 0,
        PermissionMode::Default => 1,
        PermissionMode::Auto => 2,
        PermissionMode::AcceptEdits => 3,
        PermissionMode::Bubble => 4,
        PermissionMode::DontAsk => 5,
        PermissionMode::BypassPermissions => 6,
    }
}

fn sandbox_strictness(kind: &str) -> u8 {
    match kind {
        "disabled" => 0,
        "logical" => 1,
        "ssh" => 2,
        "openshell" => 3,
        "docker" => 4,
        _ => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::definition::PermissionMode;
    use crate::agents::evolution::AgentEvolutionPolicyDecision;

    #[test]
    fn bypass_permission_request_is_critical() {
        let diff = ToolAccessProfileDiff {
            permission_mode_from: Some(PermissionMode::Default),
            permission_mode_to: Some(PermissionMode::BypassPermissions),
            ..Default::default()
        };

        let proposal = diff.to_proposal(
            "verifier",
            "agentv-1",
            "agentv-2",
            vec!["denial-1".to_string()],
        );

        assert_eq!(diff.risk_level(), AgentEvolutionRiskLevel::Critical);
        assert_eq!(proposal.risk_level, AgentEvolutionRiskLevel::Critical);
        assert_eq!(
            proposal.policy_decision,
            AgentEvolutionPolicyDecision::PendingApproval
        );
        assert!(proposal.affects_permissions);
    }

    #[test]
    fn added_tools_are_high_risk_not_grants() {
        let diff = ToolAccessProfileDiff {
            added_tools: vec!["Bash".to_string()],
            ..Default::default()
        };

        let proposal = diff.to_proposal(
            "tester",
            "agentv-1",
            "agentv-2",
            vec!["manual-approval-1".to_string()],
        );

        assert!(diff.expands_permissions());
        assert_eq!(proposal.risk_level, AgentEvolutionRiskLevel::High);
        assert_eq!(
            proposal.policy_decision,
            AgentEvolutionPolicyDecision::PendingApproval
        );
        assert_eq!(proposal.evidence_ids, vec!["manual-approval-1"]);
    }

    #[test]
    fn plan_to_default_is_permission_expansion() {
        let diff = ToolAccessProfileDiff {
            permission_mode_from: Some(PermissionMode::Plan),
            permission_mode_to: Some(PermissionMode::Default),
            ..Default::default()
        };

        assert!(diff.expands_permissions());
        assert_eq!(diff.risk_level(), AgentEvolutionRiskLevel::High);
    }

    #[test]
    fn default_to_bubble_is_permission_expansion() {
        let diff = ToolAccessProfileDiff {
            permission_mode_from: Some(PermissionMode::Default),
            permission_mode_to: Some(PermissionMode::Bubble),
            ..Default::default()
        };

        assert!(diff.expands_permissions());
        assert_eq!(diff.risk_level(), AgentEvolutionRiskLevel::High);
    }

    #[test]
    fn removing_tools_only_is_medium_risk() {
        let diff = ToolAccessProfileDiff {
            removed_tools: vec!["Bash".to_string()],
            ..Default::default()
        };

        let proposal = diff.to_proposal("coder", "agentv-1", "agentv-2", Vec::<String>::new());

        assert!(!diff.expands_permissions());
        assert!(diff.reduces_permissions());
        assert_eq!(proposal.risk_level, AgentEvolutionRiskLevel::Medium);
        assert_eq!(
            proposal.policy_decision,
            AgentEvolutionPolicyDecision::EligibleForAutoApply
        );
    }

    #[test]
    fn docker_to_logical_sandbox_is_high_risk_loosening() {
        let diff = ToolAccessProfileDiff {
            sandbox_backend_from: Some("docker".to_string()),
            sandbox_backend_to: Some("logical".to_string()),
            ..Default::default()
        };

        assert!(diff.expands_permissions());
        assert_eq!(diff.risk_level(), AgentEvolutionRiskLevel::High);
    }
}

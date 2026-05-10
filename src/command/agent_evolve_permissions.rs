//! Read-only permission governance review for agent evolution proposals.

use anyhow::Result;
use archon_core::agents::evolution::ToolAccessProfileDiff;
use cozo::DbInstance;
use serde::Serialize;

pub(crate) fn cmd_show_permission_diff(
    db: &DbInstance,
    proposal_id: &str,
    json: bool,
) -> Result<()> {
    let review = PermissionProposalReview::load(db, proposal_id)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&review)?);
    } else {
        print!("{}", review.render_human());
    }
    Ok(())
}

#[derive(Debug, Serialize, PartialEq)]
struct PermissionProposalReview {
    proposal_id: String,
    agent_type: String,
    kind: String,
    status: String,
    risk_level: String,
    policy_decision: String,
    provider_identity_affected: bool,
    permissions_affected: bool,
    rollback_target_version: String,
    permission_related: bool,
    guardrails: Vec<&'static str>,
    structured_diff: Option<PermissionDiffReview>,
    raw_diff: Option<String>,
    evidence_ids: Vec<String>,
}

impl PermissionProposalReview {
    fn load(db: &DbInstance, proposal_id: &str) -> Result<Self> {
        let proposal = archon_learning::agent_evolution_proposals::get_agent_evolution_proposal(
            db,
            proposal_id,
        )?
        .ok_or_else(|| anyhow::anyhow!("agent evolution proposal not found: {proposal_id}"))?;
        Ok(Self::from_proposal(proposal))
    }

    fn from_proposal(
        proposal: archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord,
    ) -> Self {
        let structured_diff = parse_tool_access_diff(&proposal.diff);
        let raw_diff = structured_diff
            .is_none()
            .then(|| non_empty_string(&proposal.diff))
            .flatten();
        let permission_related =
            proposal.affects_permissions || looks_permission_related(&proposal.kind);

        Self {
            proposal_id: proposal.proposal_id,
            agent_type: proposal.agent_type,
            kind: proposal.kind,
            status: proposal.status,
            risk_level: proposal.risk_level,
            policy_decision: proposal.policy_decision,
            provider_identity_affected: proposal.affects_provider_identity,
            permissions_affected: proposal.affects_permissions,
            rollback_target_version: proposal.rollback_target_version,
            permission_related,
            guardrails: permission_guardrails(),
            structured_diff,
            raw_diff,
            evidence_ids: proposal.evidence_ids,
        }
    }

    fn render_human(&self) -> String {
        let mut output = String::new();
        output.push_str(&format!("Proposal:    {}\n", self.proposal_id));
        output.push_str(&format!("Agent:       {}\n", self.agent_type));
        output.push_str(&format!("Kind:        {}\n", self.kind));
        output.push_str(&format!("Status:      {}\n", self.status));
        output.push_str(&format!("Risk:        {}\n", self.risk_level));
        output.push_str(&format!("Decision:    {}\n", self.policy_decision));
        output.push_str(&format!(
            "Impacts:     provider={} permissions={}\n",
            yes_no(self.provider_identity_affected),
            yes_no(self.permissions_affected)
        ));
        output.push_str(&format!("Rollback:    {}\n", self.rollback_target_version));

        if !self.permission_related {
            output.push_str("\nThis proposal is not marked as permission-impacting.\n");
        }

        output.push_str("\nPermission guardrails:\n");
        for guardrail in &self.guardrails {
            output.push_str(&format!("- {guardrail}\n"));
        }

        output.push_str("\nDiff:\n");
        if let Some(diff) = &self.structured_diff {
            output.push_str(&diff.render_human());
        } else if let Some(diff) = &self.raw_diff {
            output.push_str(diff);
            output.push('\n');
        } else {
            output.push_str("(no diff recorded)\n");
        }

        if !self.evidence_ids.is_empty() {
            output.push_str(&format!("\nEvidence: {}\n", self.evidence_ids.join(", ")));
        }
        output
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct PermissionDiffReview {
    added_tools: Vec<String>,
    removed_tools: Vec<String>,
    added_mcp_servers: Vec<String>,
    removed_mcp_servers: Vec<String>,
    added_skills: Vec<String>,
    removed_skills: Vec<String>,
    hooks_added: bool,
    permission_mode_from: Option<String>,
    permission_mode_to: Option<String>,
    sandbox_backend_from: Option<String>,
    sandbox_backend_to: Option<String>,
    expands_permissions: bool,
    requests_bypass_permissions: bool,
    derived_risk_level: String,
}

impl PermissionDiffReview {
    fn from_diff(diff: ToolAccessProfileDiff) -> Self {
        let expands_permissions = diff.expands_permissions();
        let requests_bypass_permissions = diff.requests_bypass_permissions();
        let derived_risk_level = format!("{:?}", diff.risk_level()).to_ascii_lowercase();
        Self {
            added_tools: diff.added_tools,
            removed_tools: diff.removed_tools,
            added_mcp_servers: diff.added_mcp_servers,
            removed_mcp_servers: diff.removed_mcp_servers,
            added_skills: diff.added_skills,
            removed_skills: diff.removed_skills,
            hooks_added: diff.hooks_added,
            permission_mode_from: diff
                .permission_mode_from
                .as_ref()
                .map(|mode| mode.as_str().to_string()),
            permission_mode_to: diff
                .permission_mode_to
                .as_ref()
                .map(|mode| mode.as_str().to_string()),
            sandbox_backend_from: diff.sandbox_backend_from,
            sandbox_backend_to: diff.sandbox_backend_to,
            expands_permissions,
            requests_bypass_permissions,
            derived_risk_level,
        }
    }

    fn render_human(&self) -> String {
        let mut output = String::new();
        output.push_str(&format!(
            "Derived risk: {}\nExpands permissions: {}\nRequests bypass: {}\n",
            self.derived_risk_level,
            yes_no(self.expands_permissions),
            yes_no(self.requests_bypass_permissions)
        ));
        output.push_str(&format!(
            "Added tools: {}\n",
            list_or_none(&self.added_tools)
        ));
        output.push_str(&format!(
            "Removed tools: {}\n",
            list_or_none(&self.removed_tools)
        ));
        output.push_str(&format!(
            "Added MCP servers: {}\n",
            list_or_none(&self.added_mcp_servers)
        ));
        output.push_str(&format!(
            "Removed MCP servers: {}\n",
            list_or_none(&self.removed_mcp_servers)
        ));
        output.push_str(&format!(
            "Added skills: {}\n",
            list_or_none(&self.added_skills)
        ));
        output.push_str(&format!(
            "Removed skills: {}\n",
            list_or_none(&self.removed_skills)
        ));
        output.push_str(&format!("Hooks added: {}\n", yes_no(self.hooks_added)));
        output.push_str(&format!(
            "Permission mode: {} -> {}\n",
            self.permission_mode_from.as_deref().unwrap_or("-"),
            self.permission_mode_to.as_deref().unwrap_or("-")
        ));
        output.push_str(&format!(
            "Sandbox backend: {} -> {}\n",
            self.sandbox_backend_from.as_deref().unwrap_or("-"),
            self.sandbox_backend_to.as_deref().unwrap_or("-")
        ));
        output
    }
}

fn parse_tool_access_diff(diff: &str) -> Option<PermissionDiffReview> {
    serde_json::from_str::<ToolAccessProfileDiff>(diff)
        .ok()
        .map(PermissionDiffReview::from_diff)
}

fn permission_guardrails() -> Vec<&'static str> {
    vec![
        "parent session mode, sandbox state, and CLI bypass guards remain authoritative",
        "subagent deny lists remain authoritative",
        "evolved profiles can only narrow access or propose reviewed changes",
        "this command is read-only and never applies permission grants",
    ]
}

fn looks_permission_related(kind: &str) -> bool {
    let kind = kind.to_ascii_lowercase();
    kind.contains("permission")
        || kind.contains("tool_access")
        || kind.contains("toolaccess")
        || kind.contains("sandbox")
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn list_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values.join(", ")
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_kind_detection_covers_tool_access_and_sandbox() {
        assert!(looks_permission_related("ToolAccessProfile"));
        assert!(looks_permission_related("sandbox_backend_profile"));
        assert!(looks_permission_related("permission_overlay"));
        assert!(!looks_permission_related("prompt_profile"));
    }

    #[test]
    fn structured_permission_diff_renders_expansion_without_applying() {
        let proposal =
            archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord::new(
                "prop-1",
                "reviewer",
                "agentv-1",
                "agentv-2",
                "tool_access_profile",
                "2026-05-08T12:00:00Z",
            )
            .with_diff(
                archon_core::agents::evolution::ToolAccessProfileDiff {
                    added_tools: vec!["Bash".into()],
                    permission_mode_from: Some(
                        archon_core::agents::definition::PermissionMode::Plan,
                    ),
                    permission_mode_to: Some(
                        archon_core::agents::definition::PermissionMode::Default,
                    ),
                    sandbox_backend_from: Some("docker".into()),
                    sandbox_backend_to: Some("logical".into()),
                    ..Default::default()
                }
                .to_proposal("reviewer", "agentv-1", "agentv-2", Vec::<String>::new())
                .diff,
            )
            .with_permission_impact();

        let review = PermissionProposalReview::from_proposal(proposal);
        let diff = review.structured_diff.as_ref().expect("structured diff");
        let body = review.render_human();

        assert!(diff.expands_permissions);
        assert_eq!(diff.derived_risk_level, "high");
        assert!(body.contains("Added tools: Bash"));
        assert!(body.contains("Permission mode: plan -> default"));
        assert!(body.contains("never applies permission grants"));
    }

    #[test]
    fn raw_diff_is_preserved_for_legacy_permission_proposals() {
        let proposal =
            archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord::new(
                "prop-2",
                "reviewer",
                "agentv-1",
                "agentv-2",
                "tool_access_profile",
                "2026-05-08T12:00:00Z",
            )
            .with_diff("+ review repeated denied tool `Bash`; do not grant automatically")
            .with_permission_impact();

        let review = PermissionProposalReview::from_proposal(proposal);

        assert!(review.structured_diff.is_none());
        assert!(
            review
                .raw_diff
                .as_deref()
                .unwrap()
                .contains("do not grant automatically")
        );
    }
}

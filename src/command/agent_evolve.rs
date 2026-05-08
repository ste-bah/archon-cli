//! `archon agent evolve` read-only CLI surfaces.

use anyhow::Result;
use cozo::DbInstance;

use crate::cli_args::{AgentAction, AgentEvolveAction};

pub(crate) async fn handle_agent_command(
    action: &AgentAction,
    _config: &archon_core::config::ArchonConfig,
) -> Result<()> {
    let db_path = learning_db_path()?;
    let db = open_learning_db(&db_path)?;
    archon_learning::schema::ensure_learning_schema(&db)?;

    match action {
        AgentAction::Evolve { action } => handle_evolve_action(&db, action),
    }
}

fn handle_evolve_action(db: &DbInstance, action: &AgentEvolveAction) -> Result<()> {
    match action {
        AgentEvolveAction::List { status, agent } => {
            cmd_list_agent_evolution(db, status.as_deref(), agent.as_deref())
        }
        AgentEvolveAction::Permissions { proposal_id } => cmd_show_permission_diff(db, proposal_id),
    }
}

fn learning_db_path() -> Result<std::path::PathBuf> {
    let base = archon_session::storage::default_db_path();
    let parent = base
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cannot determine data directory"))?;
    Ok(parent.join("learning.db"))
}

fn open_learning_db(path: &std::path::Path) -> Result<DbInstance> {
    let path_str = path.to_string_lossy().to_string();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    DbInstance::new("sqlite", &path_str, "").map_err(|e| anyhow::anyhow!("open learning db: {e}"))
}

fn cmd_list_agent_evolution(
    db: &DbInstance,
    status: Option<&str>,
    agent_filter: Option<&str>,
) -> Result<()> {
    let mut proposals =
        archon_learning::agent_evolution_proposals::list_agent_evolution_proposals(db, status)?;
    if let Some(agent) = agent_filter {
        proposals.retain(|proposal| proposal.agent_type == agent);
    }

    if proposals.is_empty() {
        println!("No agent evolution proposals found.");
        return Ok(());
    }

    println!(
        "{:<24} {:<18} {:<20} {:<10} {:<12} {:<10} permissions",
        "proposal_id", "agent", "kind", "status", "risk", "provider"
    );
    for proposal in &proposals {
        println!(
            "{:<24} {:<18} {:<20} {:<10} {:<12} {:<10} {}",
            proposal.proposal_id,
            proposal.agent_type,
            proposal.kind,
            proposal.status,
            proposal.risk_level,
            yes_no(proposal.affects_provider_identity),
            yes_no(proposal.affects_permissions)
        );
    }
    println!("\n{} proposal(s)", proposals.len());
    Ok(())
}

fn cmd_show_permission_diff(db: &DbInstance, proposal_id: &str) -> Result<()> {
    let proposal =
        archon_learning::agent_evolution_proposals::get_agent_evolution_proposal(db, proposal_id)?
            .ok_or_else(|| anyhow::anyhow!("agent evolution proposal not found: {proposal_id}"))?;

    println!("Proposal:    {}", proposal.proposal_id);
    println!("Agent:       {}", proposal.agent_type);
    println!("Kind:        {}", proposal.kind);
    println!("Status:      {}", proposal.status);
    println!("Risk:        {}", proposal.risk_level);
    println!("Decision:    {}", proposal.policy_decision);
    println!(
        "Impacts:     provider={} permissions={}",
        yes_no(proposal.affects_provider_identity),
        yes_no(proposal.affects_permissions)
    );
    println!("Rollback:    {}", proposal.rollback_target_version);

    if !proposal.affects_permissions && !looks_permission_related(&proposal.kind) {
        println!("\nThis proposal is not marked as permission-impacting.");
    }

    println!("\nPermission guardrails:");
    println!("- parent session mode, sandbox state, and CLI bypass guards remain authoritative");
    println!("- subagent deny lists remain authoritative");
    println!("- evolved profiles can only narrow or propose reviewed changes");

    println!("\nDiff:");
    if proposal.diff.trim().is_empty() {
        println!("(no diff recorded)");
    } else {
        println!("{}", proposal.diff);
    }

    if !proposal.evidence_ids.is_empty() {
        println!("\nEvidence: {}", proposal.evidence_ids.join(", "));
    }
    Ok(())
}

fn looks_permission_related(kind: &str) -> bool {
    let kind = kind.to_ascii_lowercase();
    kind.contains("permission")
        || kind.contains("tool_access")
        || kind.contains("toolaccess")
        || kind.contains("sandbox")
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
}

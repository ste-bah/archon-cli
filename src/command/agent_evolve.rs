//! `archon agent evolve` CLI surfaces.

use anyhow::Result;
use chrono::{DateTime, Utc};
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
        AgentEvolveAction::Approve { proposal_id } => {
            cmd_update_proposal_status(db, proposal_id, "approved")
        }
        AgentEvolveAction::Generate { agent } => cmd_generate_agent_evolution(db, agent),
        AgentEvolveAction::List { status, agent } => {
            cmd_list_agent_evolution(db, status.as_deref(), agent.as_deref())
        }
        AgentEvolveAction::Permissions { proposal_id } => cmd_show_permission_diff(db, proposal_id),
        AgentEvolveAction::Reject { proposal_id } => {
            cmd_update_proposal_status(db, proposal_id, "rejected")
        }
    }
}

fn cmd_generate_agent_evolution(db: &DbInstance, agent_type: &str) -> Result<()> {
    let ledger = archon_learning::agent_evolution_ledger::list_agent_performance_ledger_by_agent(
        db, agent_type,
    )?;
    if ledger.is_empty() {
        println!("No performance ledger rows found for agent: {agent_type}");
        return Ok(());
    }

    let events: Vec<_> = ledger.iter().map(ledger_record_to_event).collect();
    let runtime = archon_core::agents::evolution::AgentEvolutionRuntime::new(
        archon_core::agents::evolution::AgentEvolutionRuntimeConfig::default(),
    );
    let proposals = runtime.propose(&events);

    if proposals.is_empty() {
        println!(
            "No agent evolution proposals generated for {agent_type}. Scanned {} ledger row(s).",
            ledger.len()
        );
        return Ok(());
    }

    for proposal in &proposals {
        archon_learning::agent_evolution_proposals::insert_agent_evolution_proposal(
            db,
            &proposal_to_record(proposal),
        )?;
    }

    println!(
        "Generated {} agent evolution proposal(s) for {agent_type} from {} ledger row(s).",
        proposals.len(),
        ledger.len()
    );
    for proposal in proposals {
        println!(
            "{:<24} {:<20} {:<10} {}",
            proposal.proposal_id,
            proposal_kind_str(proposal.kind),
            risk_level_str(proposal.risk_level),
            proposal.expected_impact
        );
    }
    Ok(())
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

fn cmd_update_proposal_status(db: &DbInstance, proposal_id: &str, status: &str) -> Result<()> {
    let proposal =
        archon_learning::agent_evolution_proposals::update_agent_evolution_proposal_status(
            db,
            proposal_id,
            status,
        )?;
    println!(
        "Proposal {} marked {}. Agent={} kind={} risk={}",
        proposal.proposal_id,
        proposal.status,
        proposal.agent_type,
        proposal.kind,
        proposal.risk_level
    );
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

fn ledger_record_to_event(
    record: &archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord,
) -> archon_core::agents::evolution::AgentPerformanceEvent {
    archon_core::agents::evolution::AgentPerformanceEvent {
        event_id: record.event_id.clone(),
        agent_type: record.agent_type.clone(),
        agent_version: record.agent_version.clone(),
        run_id: record.run_id.clone(),
        pipeline_id: record.pipeline_id.clone(),
        phase: record.phase.clone(),
        task_hash: record.task_hash.clone(),
        model_id: record.model_id.clone(),
        provider_id: record.provider_id.clone(),
        profile_id: record.profile_id.clone(),
        permission_mode: record.permission_mode.clone(),
        completion_status: completion_status(&record.completion_status),
        applied_rate: record.applied_rate,
        completion_rate: record.completion_rate,
        quality_score: record.quality_score,
        l_score: record.l_score,
        user_accepted: record.user_accepted,
        user_corrected: record.user_corrected,
        gate_failed: record.gate_failed.clone(),
        test_failed: record.test_failed,
        provider_incident_id: record.provider_incident_id.clone(),
        evidence_ids: record.evidence_ids.clone(),
        created_at: parse_rfc3339_utc(&record.created_at),
    }
}

fn proposal_to_record(
    proposal: &archon_core::agents::evolution::AgentEvolutionProposal,
) -> archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord {
    let mut record = archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord::new(
        proposal.proposal_id.clone(),
        proposal.agent_type.clone(),
        proposal.current_version.clone(),
        proposal.proposed_version.clone(),
        proposal_kind_str(proposal.kind),
        proposal.created_at.to_rfc3339(),
    )
    .with_diff(proposal.diff.clone())
    .with_risk(
        risk_level_str(proposal.risk_level),
        policy_decision_str(proposal.policy_decision),
    )
    .with_status(status_str(proposal.status))
    .with_expected_impact(proposal.expected_impact.clone());

    for evidence_id in &proposal.evidence_ids {
        record = record.with_evidence(evidence_id.clone());
    }
    if proposal.affects_provider_identity {
        record = record.with_provider_identity_impact();
    }
    if proposal.affects_permissions {
        record = record.with_permission_impact();
    }
    record
}

fn completion_status(value: &str) -> archon_core::agents::evolution::AgentCompletionStatus {
    match value {
        "succeeded" | "success" => archon_core::agents::evolution::AgentCompletionStatus::Succeeded,
        "failed" | "failure" => archon_core::agents::evolution::AgentCompletionStatus::Failed,
        "cancelled" | "canceled" => {
            archon_core::agents::evolution::AgentCompletionStatus::Cancelled
        }
        "timed_out" | "timeout" => archon_core::agents::evolution::AgentCompletionStatus::TimedOut,
        _ => archon_core::agents::evolution::AgentCompletionStatus::Inconclusive,
    }
}

fn proposal_kind_str(
    kind: archon_core::agents::evolution::AgentEvolutionProposalKind,
) -> &'static str {
    match kind {
        archon_core::agents::evolution::AgentEvolutionProposalKind::PromptProfile => {
            "prompt_profile"
        }
        archon_core::agents::evolution::AgentEvolutionProposalKind::AgentRoutingProfile => {
            "agent_routing_profile"
        }
        archon_core::agents::evolution::AgentEvolutionProposalKind::ToolAccessProfile => {
            "tool_access_profile"
        }
        archon_core::agents::evolution::AgentEvolutionProposalKind::MemoryProfile => {
            "memory_profile"
        }
        archon_core::agents::evolution::AgentEvolutionProposalKind::ModelProfile => "model_profile",
        archon_core::agents::evolution::AgentEvolutionProposalKind::QualityGateProfile => {
            "quality_gate_profile"
        }
        archon_core::agents::evolution::AgentEvolutionProposalKind::SkillProfile => "skill_profile",
    }
}

fn risk_level_str(level: archon_core::agents::evolution::AgentEvolutionRiskLevel) -> &'static str {
    match level {
        archon_core::agents::evolution::AgentEvolutionRiskLevel::Low => "low",
        archon_core::agents::evolution::AgentEvolutionRiskLevel::Medium => "medium",
        archon_core::agents::evolution::AgentEvolutionRiskLevel::High => "high",
        archon_core::agents::evolution::AgentEvolutionRiskLevel::Critical => "critical",
    }
}

fn policy_decision_str(
    decision: archon_core::agents::evolution::AgentEvolutionPolicyDecision,
) -> &'static str {
    match decision {
        archon_core::agents::evolution::AgentEvolutionPolicyDecision::EligibleForAutoApply => {
            "eligible_for_auto_apply"
        }
        archon_core::agents::evolution::AgentEvolutionPolicyDecision::PendingApproval => {
            "pending_approval"
        }
        archon_core::agents::evolution::AgentEvolutionPolicyDecision::Denied => "denied",
    }
}

fn status_str(status: archon_core::agents::evolution::AgentEvolutionStatus) -> &'static str {
    match status {
        archon_core::agents::evolution::AgentEvolutionStatus::Pending => "pending",
        archon_core::agents::evolution::AgentEvolutionStatus::Approved => "approved",
        archon_core::agents::evolution::AgentEvolutionStatus::Rejected => "rejected",
        archon_core::agents::evolution::AgentEvolutionStatus::Applied => "applied",
        archon_core::agents::evolution::AgentEvolutionStatus::RolledBack => "rolled_back",
    }
}

fn parse_rfc3339_utc(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
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
    fn proposal_conversion_preserves_permission_flags() {
        let proposal = archon_core::agents::evolution::AgentEvolutionProposal::new(
            "reviewer",
            "agentv-1",
            "agentv-2",
            archon_core::agents::evolution::AgentEvolutionProposalKind::ToolAccessProfile,
            "+ Bash",
            "Review before expanding tools",
        )
        .add_evidence("ledger-1");

        let record = proposal_to_record(&proposal);

        assert_eq!(record.kind, "tool_access_profile");
        assert_eq!(record.risk_level, "high");
        assert_eq!(record.policy_decision, "pending_approval");
        assert!(record.affects_permissions);
        assert_eq!(record.evidence_ids, vec!["ledger-1"]);
    }

    #[test]
    fn ledger_record_conversion_maps_success_status() {
        let record = archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord::new(
            "ledger-1",
            "planner",
            "succeeded",
            "2026-05-08T12:00:00Z",
        )
        .with_agent_version("agentv-1")
        .with_scores(Some(0.9), Some(1.0));

        let event = ledger_record_to_event(&record);

        assert_eq!(
            event.completion_status,
            archon_core::agents::evolution::AgentCompletionStatus::Succeeded
        );
        assert_eq!(event.agent_version.as_deref(), Some("agentv-1"));
        assert_eq!(event.quality_score, Some(0.9));
    }
}

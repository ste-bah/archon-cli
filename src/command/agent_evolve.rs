//! `archon agent evolve` CLI surfaces.

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
        AgentEvolveAction::Active { agent, json } => cmd_show_active_profile(db, agent, *json),
        AgentEvolveAction::Apply {
            proposal_id,
            activate,
        } => cmd_apply_proposal(db, proposal_id, *activate),
        AgentEvolveAction::Approve { proposal_id } => {
            cmd_update_proposal_status(db, proposal_id, "approved")
        }
        AgentEvolveAction::Generate { agent } => {
            crate::command::agent_evolve_generate::cmd_generate_agent_evolution(db, agent)
        }
        AgentEvolveAction::List { status, agent } => {
            cmd_list_agent_evolution(db, status.as_deref(), agent.as_deref())
        }
        AgentEvolveAction::MemoryCandidates { agent } => cmd_list_memory_candidates(db, agent),
        AgentEvolveAction::MemoryPromote {
            candidate_id,
            min_score,
            dry_run,
        } => cmd_promote_memory_candidate(db, candidate_id, *min_score, *dry_run),
        AgentEvolveAction::Permissions { proposal_id } => cmd_show_permission_diff(db, proposal_id),
        AgentEvolveAction::Reject { proposal_id } => {
            cmd_update_proposal_status(db, proposal_id, "rejected")
        }
        AgentEvolveAction::Shadow {
            proposal_id,
            task_set,
            json,
        } => crate::command::agent_evolve_shadow::cmd_run_shadow_evaluation(
            db,
            proposal_id,
            task_set.as_deref(),
            *json,
        ),
        AgentEvolveAction::Rollback {
            agent,
            version_id,
            activate,
        } => cmd_rollback_profile(db, agent, version_id, *activate),
    }
}

fn cmd_list_memory_candidates(db: &DbInstance, agent_type: &str) -> Result<()> {
    let candidates =
        archon_learning::memory_promotion_candidates::list_memory_promotion_candidates_by_agent(
            db, agent_type,
        )?;
    if candidates.is_empty() {
        println!("No memory promotion candidates found for agent: {agent_type}");
        return Ok(());
    }

    println!(
        "{:<28} {:<18} {:<24} {:<8} claim",
        "candidate_id", "source", "target", "score"
    );
    for candidate in &candidates {
        let score = promotion_score(candidate);
        println!(
            "{:<28} {:<18} {:<24} {:<8.3} {}",
            candidate.candidate_id,
            candidate.signal_source,
            candidate.target,
            score,
            candidate.claim
        );
    }
    println!("\n{} candidate(s)", candidates.len());
    Ok(())
}

fn cmd_promote_memory_candidate(
    db: &DbInstance,
    candidate_id: &str,
    min_score: f64,
    dry_run: bool,
) -> Result<()> {
    let candidate = archon_learning::memory_promotion_candidates::get_memory_promotion_candidate(
        db,
        candidate_id,
    )?
    .ok_or_else(|| anyhow::anyhow!("memory promotion candidate not found: {candidate_id}"))?;
    let score = promotion_score(&candidate);
    if score < min_score {
        anyhow::bail!(
            "candidate {} score {:.3} is below required threshold {:.3}",
            candidate.candidate_id,
            score,
            min_score
        );
    }

    if dry_run {
        println!("Memory promotion dry run");
        println!("Candidate: {}", candidate.candidate_id);
        println!("Agent:     {}", candidate.agent_type);
        println!("Score:     {:.3}", score);
        println!("Claim:     {}", candidate.claim);
        println!("Target:    Archon memory graph (Cozo)");
        return Ok(());
    }

    let graph = archon_memory::MemoryGraph::open_default()
        .map_err(|error| anyhow::anyhow!("open memory graph: {error}"))?;
    let project_path = std::env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let memory_id =
        promote_memory_candidate_to_graph(&graph, &candidate, min_score, &project_path)?;
    println!(
        "Promoted memory candidate {} into Archon memory graph as {}.",
        candidate.candidate_id, memory_id
    );
    Ok(())
}

fn promote_memory_candidate_to_graph(
    memory: &dyn archon_memory::MemoryTrait,
    candidate: &archon_learning::memory_promotion_candidates::MemoryPromotionCandidateRecord,
    min_score: f64,
    project_path: &str,
) -> Result<String> {
    let score = promotion_score(candidate);
    if score < min_score {
        anyhow::bail!(
            "candidate {} score {:.3} is below required threshold {:.3}",
            candidate.candidate_id,
            score,
            min_score
        );
    }
    let mut tags = vec![
        "agent-evolution".to_string(),
        "memory-promotion".to_string(),
        format!("agent:{}", candidate.agent_type),
        format!("candidate:{}", candidate.candidate_id),
    ];
    tags.extend(
        candidate
            .evidence_ids
            .iter()
            .map(|evidence_id| format!("evidence:{evidence_id}")),
    );
    let title = format!("Agent {} correction", candidate.agent_type);
    memory
        .store_memory(
            &candidate.claim,
            &title,
            archon_memory::types::MemoryType::Correction,
            score,
            &tags,
            "agent_evolution_memory_promotion",
            project_path,
        )
        .map_err(|error| anyhow::anyhow!("store promoted memory: {error}"))
}

fn cmd_show_active_profile(db: &DbInstance, agent_type: &str, json: bool) -> Result<()> {
    let active =
        archon_learning::agent_profile_versions::get_active_agent_profile_version(db, agent_type)?;
    let Some(active) = active else {
        println!("No active governed profile version found for agent: {agent_type}");
        return Ok(());
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&active)?);
    } else {
        println!("Agent:       {}", active.agent_type);
        println!("Version:     {}", active.version_id);
        println!("Version no:  {}", active.version_number);
        println!("Source:      {}", active.source);
        println!(
            "Proposal:    {}",
            active.created_by_proposal_id.as_deref().unwrap_or("-")
        );
        println!(
            "Parent:      {}",
            active.parent_version_id.as_deref().unwrap_or("-")
        );
        println!(
            "Rollback:    {}",
            if active.is_rollback_target {
                "yes"
            } else {
                "no"
            }
        );
        println!("Created:     {}", active.created_at);
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

fn cmd_apply_proposal(db: &DbInstance, proposal_id: &str, activate: bool) -> Result<()> {
    let applied = crate::command::agent_evolve_apply::apply_proposal(db, proposal_id, activate)?;
    println!(
        "Applied proposal {} into profile version {} (active={}).",
        proposal_id,
        applied.version_id,
        yes_no(applied.active)
    );
    Ok(())
}

fn cmd_rollback_profile(
    db: &DbInstance,
    agent_type: &str,
    version_id: &str,
    activate: bool,
) -> Result<()> {
    let applied =
        crate::command::agent_evolve_apply::rollback_profile(db, agent_type, version_id, activate)?;
    println!(
        "Created rollback profile version {} for agent {} (active={}).",
        applied.version_id,
        agent_type,
        yes_no(applied.active)
    );
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

fn promotion_score(
    candidate: &archon_learning::memory_promotion_candidates::MemoryPromotionCandidateRecord,
) -> f64 {
    (candidate.confidence * 0.35)
        + (candidate.frequency_score * 0.20)
        + (candidate.recency_score * 0.15)
        + (candidate.diversity_score * 0.10)
        + (candidate.evidence_quality * 0.20)
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
    fn promotion_score_matches_core_weighting() {
        let candidate =
            archon_learning::memory_promotion_candidates::MemoryPromotionCandidateRecord::new(
                "mem-1",
                "planner",
                "user_correction",
                "governed_learning_event",
                "Remember to review repeated corrections.",
                "2026-05-08T12:00:00Z",
            )
            .with_scores(1.0, 0.5, 0.5, 0.0, 1.0);

        assert!((promotion_score(&candidate) - 0.725).abs() < f64::EPSILON);
    }

    #[test]
    fn memory_candidate_promotes_to_memory_graph() {
        let graph = archon_memory::MemoryGraph::in_memory().unwrap();
        let candidate =
            archon_learning::memory_promotion_candidates::MemoryPromotionCandidateRecord::new(
                "mem-1",
                "planner",
                "user_correction",
                "memory_graph_fact",
                "Always cite source evidence before changing plans.",
                "2026-05-08T12:00:00Z",
            )
            .with_scores(1.0, 1.0, 1.0, 1.0, 1.0)
            .with_evidence("ledger-1");

        let memory_id = promote_memory_candidate_to_graph(&graph, &candidate, 0.85, "/repo")
            .expect("candidate should promote");
        let memory = graph.get_memory(&memory_id).unwrap();

        assert_eq!(memory.content, candidate.claim);
        assert!(memory.tags.contains(&"agent:planner".to_string()));
        assert!(memory.tags.contains(&"evidence:ledger-1".to_string()));
        assert_eq!(
            memory.memory_type,
            archon_memory::types::MemoryType::Correction
        );
    }
}

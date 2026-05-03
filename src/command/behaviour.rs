//! `archon behaviour` CLI subcommand — governed learning management.
//!
//! Subcommands: list-proposals, list-events, show, apply, history,
//! generate-proposals, status, approve, deny, rollback.

use anyhow::Result;
use cozo::DbInstance;

use crate::cli_args::BehaviourAction;

/// Handle `archon behaviour` subcommands.
pub async fn handle_behaviour_command(
    action: &BehaviourAction,
    _config: &archon_core::config::ArchonConfig,
) -> Result<()> {
    let db_path = learning_db_path()?;
    let db = open_learning_db(&db_path)?;
    archon_learning::schema::ensure_learning_schema(&db)?;

    match action {
        BehaviourAction::ListProposals => cmd_list_proposals(&db),
        BehaviourAction::ListEvents { event_type } => {
            cmd_list_events(&db, event_type.as_deref())
        }
        BehaviourAction::Show { id } => cmd_show(&db, id),
        BehaviourAction::Apply { proposal_id } => cmd_apply(&db, proposal_id),
        BehaviourAction::History { kind } => cmd_history(&db, kind),
        BehaviourAction::GenerateProposals => cmd_generate_proposals(&db),
        BehaviourAction::Status => cmd_status(&db),
        BehaviourAction::Approve { proposal_id } => cmd_approve(&db, proposal_id),
        BehaviourAction::Deny { proposal_id } => cmd_deny(&db, proposal_id),
        BehaviourAction::Rollback {
            version_id,
            reason,
        } => cmd_rollback(&db, version_id, reason.as_deref()),
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

// ── Subcommand handlers ──────────────────────────────────────────────────────

fn cmd_list_proposals(db: &DbInstance) -> Result<()> {
    let proposals = archon_learning::store::list_behaviour_proposals(db, None)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if proposals.is_empty() {
        println!("No behaviour proposals found.");
        return Ok(());
    }

    for p in &proposals {
        println!(
            "{id}  {kind:30}  {status:10}  {risk:8}  {decision:16}",
            id = p.proposal_id,
            kind = p.manifest_kind.as_str(),
            status = p.status.as_str(),
            risk = p.risk_level.as_str(),
            decision = p.policy_decision.as_str(),
        );
    }
    println!("\n{} proposal(s)", proposals.len());
    Ok(())
}

fn cmd_list_events(db: &DbInstance, event_type: Option<&str>) -> Result<()> {
    let events = if let Some(et) = event_type {
        archon_learning::store::list_learning_events_by_type(db, et)
            .map_err(|e| anyhow::anyhow!("{e}"))?
    } else {
        archon_learning::store::list_all_learning_events(db)
            .map_err(|e| anyhow::anyhow!("{e}"))?
    };

    if events.is_empty() {
        println!("No learning events found.");
        return Ok(());
    }

    for ev in &events {
        println!(
            "{id}  {etype:35}  src={src:20}  cf={cf:.2}  {ts}",
            id = ev.event_id,
            etype = ev.event_type.as_str(),
            src = &ev.source_artifact_id[..std::cmp::min(20, ev.source_artifact_id.len())],
            cf = ev.confidence,
            ts = ev.created_at,
        );
    }
    println!("\n{} event(s)", events.len());
    Ok(())
}

fn cmd_show(db: &DbInstance, id: &str) -> Result<()> {
    // Try as proposal first
    if let Ok(Some(p)) = archon_learning::store::get_behaviour_proposal(db, id) {
        println!("Type:       BehaviourProposal");
        println!("ID:         {}", p.proposal_id);
        println!("Workspace:  {}", p.workspace_id);
        println!("Kind:       {}", p.manifest_kind.as_str());
        println!("Status:     {}", p.status.as_str());
        println!("Risk:       {}", p.risk_level.as_str());
        println!("Decision:   {}", p.policy_decision.as_str());
        println!("Diff:");
        println!("{}", p.diff);
        println!("Evidence:   {:?}", p.evidence_ids);
        return Ok(());
    }

    // Try as manifest version
    if let Ok(Some(v)) = archon_learning::store::get_manifest_version(db, id) {
        println!("Type:       BehaviourManifestVersion");
        println!("ID:         {}", v.version_id);
        println!("Kind:       {}", v.manifest_kind.as_str());
        println!("Version:    {}", v.version_number);
        println!("Content:    {}", v.content);
        println!("Diff:       {}", v.diff);
        println!("Rollback:   {}", v.is_rollback_target);
        if let Some(ref pid) = v.parent_version_id {
            println!("Parent:     {pid}");
        }
        if let Some(ref cid) = v.created_by_proposal_id {
            println!("Created by: {cid}");
        }
        return Ok(());
    }

    // Try as learning event
    if let Ok(Some(ev)) = archon_learning::store::get_learning_event(db, id) {
        println!("Type:       LearningEvent");
        println!("ID:         {}", ev.event_id);
        println!("EventType:  {}", ev.event_type.as_str());
        println!("Workspace:  {}", ev.workspace_id);
        println!("Source:     {}", ev.source_artifact_id);
        if let Some(ref oid) = ev.outcome_artifact_id {
            println!("Outcome:    {oid}");
        }
        println!("Signal:     {}", ev.signal);
        println!("Confidence: {}", ev.confidence);
        println!("Created:    {}", ev.created_at);
        return Ok(());
    }

    println!("No proposal, version, or event found with ID: {id}");
    Ok(())
}

fn cmd_apply(db: &DbInstance, proposal_id: &str) -> Result<()> {
    let result = archon_learning::apply::apply_decision(
        db,
        proposal_id,
        archon_learning::models::PolicyDecision::AutoApplied,
        None,
        Some("cli"),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    println!(
        "Proposal {id} auto-applied. New version: {ver}",
        id = proposal_id,
        ver = result
            .new_version
            .as_ref()
            .map(|v| v.version_id.as_str())
            .unwrap_or("N/A"),
    );
    Ok(())
}

fn cmd_history(db: &DbInstance, kind: &str) -> Result<()> {
    let versions = archon_learning::store::list_manifest_version_history(db, kind)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    if versions.is_empty() {
        println!("No version history found for manifest kind: {kind}");
        return Ok(());
    }

    println!("Version history for {kind}:");
    println!("{:<25} {:<8} {:<20} {:<8} {:<10} {}",
        "version_id", "v#", "created_by", "parent", "rollback", "created_at");
    for v in &versions {
        println!(
            "{vid:<25} {vn:<8} {cbid:<20} {pvid:<8} {rt:<10} {ca}",
            vid = v.version_id,
            vn = v.version_number,
            cbid = v.created_by_proposal_id.as_deref().unwrap_or("-"),
            pvid = v.parent_version_id.as_deref().unwrap_or("-"),
            rt = if v.is_rollback_target { "YES" } else { "no" },
            ca = v.created_at,
        );
    }
    println!("\n{} version(s)", versions.len());
    Ok(())
}

fn cmd_generate_proposals(db: &DbInstance) -> Result<()> {
    let events = archon_learning::store::list_all_learning_events(db)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let proposals = archon_learning::proposal::generate_proposals(&events);

    if proposals.is_empty() {
        println!("No proposals generated (thresholds not met).");
        println!("Scanned {} learning event(s).", events.len());
        return Ok(());
    }

    println!("Generated {} proposal(s) from {} event(s):", proposals.len(), events.len());
    for p in &proposals {
        println!(
            "  {id}  {kind:30}  risk={risk}  evidence={n}",
            id = p.proposal_id,
            kind = p.manifest_kind.as_str(),
            risk = p.risk_level.as_str(),
            n = p.evidence_ids.len(),
        );
        // Persist the proposal
        if let Err(e) = archon_learning::store::insert_behaviour_proposal(db, p) {
            eprintln!("  WARNING: failed to persist proposal {}: {e}", p.proposal_id);
        }
    }
    Ok(())
}

fn cmd_status(db: &DbInstance) -> Result<()> {
    let proposals = archon_learning::store::list_behaviour_proposals(db, None)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let pending = proposals.iter().filter(|p| p.status == archon_learning::models::ProposalStatus::Pending).count();
    let applied = proposals.iter().filter(|p| p.status == archon_learning::models::ProposalStatus::Applied).count();
    let denied = proposals.iter().filter(|p| p.status == archon_learning::models::ProposalStatus::Denied).count();
    let rolled_back = proposals.iter().filter(|p| p.status == archon_learning::models::ProposalStatus::RolledBack).count();

    let events = archon_learning::store::list_learning_events_by_type(db, "FalseCompletionDetected")
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let all_events = archon_learning::store::list_all_learning_events(db)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    println!("Learning System Status");
    println!("======================");
    println!("Learning events: {} total ({} false completions)", all_events.len(), events.len());
    println!("Proposals:  {} total ({} pending, {} applied, {} denied, {} rolled back)",
        proposals.len(), pending, applied, denied, rolled_back);

    // Show latest manifest versions
    for kind in &[
        "RetrievalProfile",
        "SourceQualityProfile",
        "AgentRoutingProfile",
        "ConstellationThresholds",
        "PipelineGates",
        "PromptProfile",
    ] {
        if let Ok(Some(v)) = archon_learning::store::get_latest_manifest_version(db, kind) {
            println!(
                "  {kind}: v{ver} ({id})",
                kind = kind,
                ver = v.version_number,
                id = v.version_id,
            );
        }
    }

    Ok(())
}

fn cmd_approve(db: &DbInstance, proposal_id: &str) -> Result<()> {
    let result = archon_learning::apply::apply_decision(
        db,
        proposal_id,
        archon_learning::models::PolicyDecision::Approved,
        None,
        Some("cli"),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    println!(
        "Proposal {id} approved. New version: {ver}",
        id = proposal_id,
        ver = result
            .new_version
            .as_ref()
            .map(|v| v.version_id.as_str())
            .unwrap_or("N/A"),
    );
    Ok(())
}

fn cmd_deny(db: &DbInstance, proposal_id: &str) -> Result<()> {
    archon_learning::apply::apply_decision(
        db,
        proposal_id,
        archon_learning::models::PolicyDecision::Denied,
        None,
        Some("cli"),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    println!("Proposal {id} denied.", id = proposal_id);
    Ok(())
}

fn cmd_rollback(db: &DbInstance, version_id: &str, reason: Option<&str>) -> Result<()> {
    let result = archon_learning::rollback::rollback_to_version(
        db,
        version_id,
        "cli",
        reason.unwrap_or("manual rollback via CLI"),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    println!(
        "Rolled back {kind} from {from} to {to} (v{ver})",
        kind = result.new_version.manifest_kind.as_str(),
        from = result.rolled_back_from_version_id,
        to = result.new_version.version_id,
        ver = result.new_version.version_number,
    );
    Ok(())
}

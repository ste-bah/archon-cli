//! Generate governed agent evolution proposals from the Cozo ledger.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use chrono::{DateTime, Utc};
use cozo::DbInstance;
use sha2::{Digest, Sha256};

const MIN_PERMISSION_DENIALS: usize = 3;

pub(crate) fn cmd_generate_agent_evolution(db: &DbInstance, agent_type: &str) -> Result<()> {
    let ledger = archon_learning::agent_evolution_ledger::list_agent_performance_ledger_by_agent(
        db, agent_type,
    )?;
    let permission_proposals = permission_denial_proposals(db, agent_type)?;
    if ledger.is_empty() && permission_proposals.is_empty() {
        println!("No performance ledger rows found for agent: {agent_type}");
        return Ok(());
    }

    let events: Vec<_> = ledger.iter().map(ledger_record_to_event).collect();
    let runtime = archon_core::agents::evolution::AgentEvolutionRuntime::new(
        archon_core::agents::evolution::AgentEvolutionRuntimeConfig::default(),
    );
    let mut proposals = runtime.propose(&events);
    proposals.extend(permission_proposals);
    let memory_candidates = memory_candidates_from_events(agent_type, &events);

    if proposals.is_empty() && memory_candidates.is_empty() {
        println!(
            "No agent evolution proposals or memory candidates generated for {agent_type}. Scanned {} ledger row(s).",
            ledger.len()
        );
        return Ok(());
    }

    let mut existing_fingerprints = existing_proposal_fingerprints(db, agent_type)?;
    let mut inserted_proposals = Vec::new();
    let mut skipped_duplicates = 0usize;
    for proposal in proposals {
        let record = proposal_to_record(&proposal);
        if !existing_fingerprints.insert(proposal_fingerprint(&record)) {
            skipped_duplicates += 1;
            continue;
        }
        archon_learning::agent_evolution_proposals::insert_agent_evolution_proposal(db, &record)?;
        inserted_proposals.push(proposal);
    }
    for candidate in &memory_candidates {
        archon_learning::memory_promotion_candidates::insert_memory_promotion_candidate(
            db, candidate,
        )?;
    }

    if inserted_proposals.is_empty() && memory_candidates.is_empty() {
        println!(
            "No new agent evolution proposals or memory candidates generated for {agent_type}. Skipped {skipped_duplicates} duplicate proposal(s)."
        );
        return Ok(());
    }

    println!(
        "Generated {} new agent evolution proposal(s) and {} memory candidate(s) for {agent_type} from {} ledger row(s). Skipped {} duplicate proposal(s).",
        inserted_proposals.len(),
        memory_candidates.len(),
        ledger.len(),
        skipped_duplicates
    );
    for proposal in inserted_proposals {
        println!(
            "{:<24} {:<20} {:<10} {}",
            proposal.proposal_id,
            proposal_kind_str(proposal.kind),
            risk_level_str(proposal.risk_level),
            proposal.expected_impact
        );
    }
    for candidate in memory_candidates {
        println!(
            "{:<24} {:<20} {:<10} {}",
            candidate.candidate_id, candidate.target, "memory", candidate.claim
        );
    }
    Ok(())
}

fn existing_proposal_fingerprints(db: &DbInstance, agent_type: &str) -> Result<HashSet<String>> {
    let proposals =
        archon_learning::agent_evolution_proposals::list_agent_evolution_proposals(db, None)?;
    Ok(proposals
        .iter()
        .filter(|proposal| proposal.agent_type == agent_type)
        .map(proposal_fingerprint)
        .collect())
}

fn permission_denial_proposals(
    db: &DbInstance,
    agent_type: &str,
) -> Result<Vec<archon_core::agents::evolution::AgentEvolutionProposal>> {
    let events =
        archon_learning::permission_runtime_events::list_permission_runtime_events_by_decision(
            db, "denied",
        )?;
    Ok(permission_denial_proposals_from_events(agent_type, &events))
}

fn permission_denial_proposals_from_events(
    agent_type: &str,
    events: &[archon_learning::permission_runtime_events::PermissionRuntimeEventRecord],
) -> Vec<archon_core::agents::evolution::AgentEvolutionProposal> {
    let mut by_tool: HashMap<
        String,
        Vec<&archon_learning::permission_runtime_events::PermissionRuntimeEventRecord>,
    > = HashMap::new();
    for event in events {
        if event.agent_type.as_deref() == Some(agent_type) {
            by_tool
                .entry(event.tool_name.clone())
                .or_default()
                .push(event);
        }
    }

    by_tool
        .into_iter()
        .filter(|(_, events)| events.len() >= MIN_PERMISSION_DENIALS)
        .map(|(tool, events)| {
            events.iter().fold(
                archon_core::agents::evolution::AgentEvolutionProposal::new(
                    agent_type,
                    "unversioned",
                    "unversioned+evo",
                    archon_core::agents::evolution::AgentEvolutionProposalKind::ToolAccessProfile,
                    format!("+ review repeated denied tool `{tool}`; do not grant automatically"),
                    "Repeated permission denials should be reviewed as a tool-access proposal",
                )
                .with_permission_impact(false),
                |proposal, event| proposal.add_evidence(&event.event_id),
            )
        })
        .collect()
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

fn proposal_fingerprint(
    proposal: &archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord,
) -> String {
    let mut evidence = proposal.evidence_ids.clone();
    evidence.sort();
    let mut digest = Sha256::new();
    digest.update(proposal.agent_type.as_bytes());
    digest.update(b"\0");
    digest.update(proposal.kind.as_bytes());
    digest.update(b"\0");
    digest.update(proposal.diff.as_bytes());
    digest.update(b"\0");
    for evidence_id in evidence {
        digest.update(evidence_id.as_bytes());
        digest.update(b"\0");
    }
    format!("proposal-fingerprint-{}", hex::encode(digest.finalize()))
}

fn memory_candidates_from_events(
    agent_type: &str,
    events: &[archon_core::agents::evolution::AgentPerformanceEvent],
) -> Vec<archon_learning::memory_promotion_candidates::MemoryPromotionCandidateRecord> {
    let correction_events: Vec<_> = events
        .iter()
        .filter(|event| event.user_corrected == Some(true))
        .collect();
    if correction_events.len() < 3 {
        return Vec::new();
    }

    let evidence_ids: Vec<String> = correction_events
        .iter()
        .map(|event| event.event_id.clone())
        .collect();
    let mut candidate =
        archon_learning::memory_promotion_candidates::MemoryPromotionCandidateRecord::new(
            stable_memory_candidate_id(agent_type, &evidence_ids),
            agent_type.to_string(),
            "user_correction",
            "governed_learning_event",
            format!(
                "Repeated user corrections observed for agent `{agent_type}`; review before durable memory promotion."
            ),
            Utc::now().to_rfc3339(),
        )
        .with_scores(0.8, 0.8, 0.7, 0.6, 0.75);
    for evidence_id in evidence_ids {
        candidate = candidate.with_evidence(evidence_id);
    }
    vec![candidate]
}

fn stable_memory_candidate_id(agent_type: &str, evidence_ids: &[String]) -> String {
    let mut digest = Sha256::new();
    digest.update(agent_type.as_bytes());
    for evidence_id in evidence_ids {
        digest.update(b"\0");
        digest.update(evidence_id.as_bytes());
    }
    format!("memory-promotion-{}", hex::encode(digest.finalize()))
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

    #[test]
    fn repeated_corrections_create_governed_memory_candidate() {
        let events = vec![
            archon_core::agents::evolution::AgentPerformanceEvent::new("planner")
                .with_user_feedback(Some(false), Some(true)),
            archon_core::agents::evolution::AgentPerformanceEvent::new("planner")
                .with_user_feedback(Some(false), Some(true)),
            archon_core::agents::evolution::AgentPerformanceEvent::new("planner")
                .with_user_feedback(Some(false), Some(true)),
        ];

        let candidates = memory_candidates_from_events("planner", &events);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].signal_source, "user_correction");
        assert_eq!(candidates[0].target, "governed_learning_event");
        assert!(candidates[0].proposal_required);
        assert_eq!(candidates[0].evidence_ids.len(), 3);
    }

    #[test]
    fn memory_candidate_ids_are_stable_for_same_evidence() {
        let evidence = vec!["ledger-1".to_string(), "ledger-2".to_string()];

        assert_eq!(
            stable_memory_candidate_id("planner", &evidence),
            stable_memory_candidate_id("planner", &evidence)
        );
    }

    #[test]
    fn proposal_fingerprint_is_stable_for_evidence_order() {
        let left = archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord::new(
            "proposal-1",
            "reviewer",
            "agentv-1",
            "agentv-2",
            "tool_access_profile",
            "2026-05-08T12:00:00Z",
        )
        .with_diff("+ review Bash")
        .with_evidence("permission-2")
        .with_evidence("permission-1");
        let right = archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord::new(
            "proposal-2",
            "reviewer",
            "agentv-1",
            "agentv-2",
            "tool_access_profile",
            "2026-05-08T12:00:00Z",
        )
        .with_diff("+ review Bash")
        .with_evidence("permission-1")
        .with_evidence("permission-2");

        assert_eq!(proposal_fingerprint(&left), proposal_fingerprint(&right));
    }

    #[test]
    fn repeated_permission_denials_create_tool_access_proposal() {
        let events: Vec<_> = (0..3)
            .map(|index| {
                archon_learning::permission_runtime_events::PermissionRuntimeEventRecord::new(
                    format!("permission-{index}"),
                    "Bash",
                    "default",
                    "denied",
                    "2026-05-08T12:00:00Z",
                )
                .with_run_context(None, Some("reviewer".into()))
            })
            .collect();

        let proposals = permission_denial_proposals_from_events("reviewer", &events);

        assert_eq!(proposals.len(), 1);
        assert_eq!(
            proposals[0].kind,
            archon_core::agents::evolution::AgentEvolutionProposalKind::ToolAccessProfile
        );
        assert!(proposals[0].affects_permissions);
        assert!(proposals[0].diff.contains("Bash"));
        assert_eq!(proposals[0].evidence_ids.len(), 3);
    }
}

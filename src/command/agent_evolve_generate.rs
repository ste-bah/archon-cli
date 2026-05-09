//! Generate governed agent evolution proposals from the Cozo ledger.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use chrono::Utc;
use cozo::DbInstance;
use sha2::{Digest, Sha256};

use super::agent_evolve_generate_support::{
    ledger_record_to_event, proposal_fingerprint, proposal_kind_str, proposal_to_record,
    risk_level_str,
};

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
    let (inserted_memory_candidates, skipped_duplicate_memory_candidates) =
        insert_new_memory_candidates(db, agent_type, memory_candidates)?;

    if inserted_proposals.is_empty() && inserted_memory_candidates.is_empty() {
        println!(
            "No new agent evolution proposals or memory candidates generated for {agent_type}. Skipped {skipped_duplicates} duplicate proposal(s) and {skipped_duplicate_memory_candidates} duplicate memory candidate(s)."
        );
        return Ok(());
    }

    println!(
        "Generated {} new agent evolution proposal(s) and {} new memory candidate(s) for {agent_type} from {} ledger row(s). Skipped {} duplicate proposal(s) and {} duplicate memory candidate(s).",
        inserted_proposals.len(),
        inserted_memory_candidates.len(),
        ledger.len(),
        skipped_duplicates,
        skipped_duplicate_memory_candidates
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
    for candidate in inserted_memory_candidates {
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

fn insert_new_memory_candidates(
    db: &DbInstance,
    agent_type: &str,
    memory_candidates: Vec<
        archon_learning::memory_promotion_candidates::MemoryPromotionCandidateRecord,
    >,
) -> Result<(
    Vec<archon_learning::memory_promotion_candidates::MemoryPromotionCandidateRecord>,
    usize,
)> {
    let mut existing_ids = existing_memory_candidate_ids(db, agent_type)?;
    let mut inserted_candidates = Vec::new();
    let mut skipped_duplicates = 0usize;
    for candidate in memory_candidates {
        if !existing_ids.insert(candidate.candidate_id.clone()) {
            skipped_duplicates += 1;
            continue;
        }
        archon_learning::memory_promotion_candidates::insert_memory_promotion_candidate(
            db, &candidate,
        )?;
        inserted_candidates.push(candidate);
    }
    Ok((inserted_candidates, skipped_duplicates))
}

fn existing_memory_candidate_ids(db: &DbInstance, agent_type: &str) -> Result<HashSet<String>> {
    let candidates =
        archon_learning::memory_promotion_candidates::list_memory_promotion_candidates_by_agent(
            db, agent_type,
        )?;
    Ok(candidates
        .into_iter()
        .map(|candidate| candidate.candidate_id)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!(
            "/tmp/test-agent-evolve-generate-{}.db",
            uuid::Uuid::new_v4()
        );
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        archon_learning::schema::ensure_learning_schema(&db).unwrap();
        db
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
    fn duplicate_memory_candidates_are_skipped_by_candidate_id() {
        let db = test_db();
        let candidate =
            archon_learning::memory_promotion_candidates::MemoryPromotionCandidateRecord::new(
                "memory-candidate-1",
                "planner",
                "user_correction",
                "governed_learning_event",
                "Review repeated corrections before durable memory promotion.",
                "2026-05-08T12:00:00Z",
            )
            .with_evidence("ledger-1");
        archon_learning::memory_promotion_candidates::insert_memory_promotion_candidate(
            &db, &candidate,
        )
        .unwrap();

        let (inserted, skipped) =
            insert_new_memory_candidates(&db, "planner", vec![candidate]).unwrap();

        assert!(inserted.is_empty());
        assert_eq!(skipped, 1);
        let persisted =
            archon_learning::memory_promotion_candidates::list_memory_promotion_candidates_by_agent(
                &db, "planner",
            )
            .unwrap();
        assert_eq!(persisted.len(), 1);
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

//! Agent knowledge digest generation from governed Cozo evidence.

use anyhow::Result;
use cozo::DbInstance;
use serde::Serialize;
use sha2::{Digest, Sha256};

pub(crate) fn cmd_generate_agent_digest(
    db: &DbInstance,
    agent_type: &str,
    persist: bool,
    json: bool,
) -> Result<()> {
    let mut digest = AgentKnowledgeDigestReport::load(db, agent_type)?;
    if persist {
        digest.persisted_event_ids = persist_claims(db, &digest.claims)?;
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&digest)?);
    } else {
        print_digest(&digest);
    }
    Ok(())
}

#[derive(Debug, Serialize, PartialEq)]
struct AgentKnowledgeDigestReport {
    agent_type: String,
    generated_at: String,
    claims: Vec<archon_learning::agent_knowledge_digest::AgentKnowledgeClaimRecord>,
    contradictions: Vec<String>,
    open_questions: Vec<String>,
    persisted_event_ids: Vec<String>,
}

impl AgentKnowledgeDigestReport {
    fn load(db: &DbInstance, agent_type: &str) -> Result<Self> {
        let generated_at = chrono::Utc::now().to_rfc3339();
        let ledger =
            archon_learning::agent_evolution_ledger::list_agent_performance_ledger_by_agent(
                db, agent_type,
            )?;
        let proposals =
            archon_learning::agent_evolution_proposals::list_agent_evolution_proposals(db, None)?
                .into_iter()
                .filter(|proposal| proposal.agent_type == agent_type)
                .collect::<Vec<_>>();
        let memory =
            archon_learning::memory_promotion_candidates::list_memory_promotion_candidates_by_agent(
                db, agent_type,
            )?;

        let mut claims = Vec::new();
        let mut contradictions = Vec::new();
        let mut open_questions = Vec::new();
        add_ledger_claims(
            agent_type,
            &generated_at,
            &ledger,
            &mut claims,
            &mut contradictions,
        );
        add_proposal_claims(
            agent_type,
            &generated_at,
            &proposals,
            &mut claims,
            &mut open_questions,
        );
        add_memory_claims(
            agent_type,
            &generated_at,
            &memory,
            &mut claims,
            &mut open_questions,
        );

        Ok(Self {
            agent_type: agent_type.to_string(),
            generated_at,
            claims,
            contradictions,
            open_questions,
            persisted_event_ids: Vec::new(),
        })
    }
}

fn add_ledger_claims(
    agent_type: &str,
    created_at: &str,
    ledger: &[archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord],
    claims: &mut Vec<archon_learning::agent_knowledge_digest::AgentKnowledgeClaimRecord>,
    contradictions: &mut Vec<String>,
) {
    if ledger.is_empty() {
        return;
    }
    let failures = ledger.iter().filter(|record| is_failure(record)).count();
    let successes = ledger.iter().filter(|record| is_success(record)).count();
    let corrections = ledger
        .iter()
        .filter(|record| record.user_corrected == Some(true))
        .count();
    let provider_incidents = ledger
        .iter()
        .filter_map(|record| record.provider_incident_id.clone())
        .collect::<Vec<_>>();
    if failures > 0 {
        let mut claim = claim_record(
            agent_type,
            "agent_reliability",
            format!(
                "{failures} failed or interrupted ledger event(s) are recorded for `{agent_type}`."
            ),
            confidence_from_ratio(failures, ledger.len()),
            created_at,
        );
        for evidence_id in ledger
            .iter()
            .filter(|record| is_failure(record))
            .map(|r| &r.event_id)
        {
            claim = claim.with_evidence(evidence_id.clone());
        }
        claims.push(claim);
    }
    if corrections >= 3 {
        let mut claim = claim_record(
            agent_type,
            "repeated_correction",
            format!(
                "{corrections} user correction signal(s) suggest a reviewable agent memory or prompt update."
            ),
            confidence_from_ratio(corrections, ledger.len()),
            created_at,
        );
        for evidence_id in ledger
            .iter()
            .filter(|record| record.user_corrected == Some(true))
            .map(|record| &record.event_id)
        {
            claim = claim.with_evidence(evidence_id.clone());
        }
        claims.push(claim.with_open_question("Should this become a governed memory promotion?"));
    }
    if !provider_incidents.is_empty() {
        let mut claim = claim_record(
            agent_type,
            "provider_reliability",
            format!("Provider incident evidence exists for `{agent_type}` runtime attempts."),
            0.8,
            created_at,
        );
        for evidence_id in provider_incidents {
            claim = claim.with_evidence(evidence_id);
        }
        claims.push(claim);
    }
    if successes > 0 && failures > 0 {
        contradictions.push(format!(
            "`{agent_type}` has mixed success and failure signals; shadow evaluation should decide whether failures are provider, permission, or prompt related."
        ));
    }
}

fn add_proposal_claims(
    agent_type: &str,
    created_at: &str,
    proposals: &[archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord],
    claims: &mut Vec<archon_learning::agent_knowledge_digest::AgentKnowledgeClaimRecord>,
    open_questions: &mut Vec<String>,
) {
    let pending_high_risk = proposals
        .iter()
        .filter(|proposal| proposal.status == "pending" && high_risk(&proposal.risk_level))
        .collect::<Vec<_>>();
    if pending_high_risk.is_empty() {
        return;
    }
    let mut claim = claim_record(
        agent_type,
        "high_risk_pending_evolution",
        format!(
            "{} high-risk pending evolution proposal(s) require review before apply.",
            pending_high_risk.len()
        ),
        0.9,
        created_at,
    );
    for proposal in &pending_high_risk {
        claim = claim.with_evidence(proposal.proposal_id.clone());
        if proposal.affects_provider_identity {
            open_questions
                .push("Does this proposal affect Anthropic spoof compatibility?".to_string());
        }
        if proposal.affects_permissions {
            open_questions.push("Does this proposal change effective tool access?".to_string());
        }
    }
    claims.push(
        claim.with_open_question("Run shadow evaluation before approving high-risk changes."),
    );
}

fn add_memory_claims(
    agent_type: &str,
    created_at: &str,
    memory: &[archon_learning::memory_promotion_candidates::MemoryPromotionCandidateRecord],
    claims: &mut Vec<archon_learning::agent_knowledge_digest::AgentKnowledgeClaimRecord>,
    open_questions: &mut Vec<String>,
) {
    if memory.is_empty() {
        return;
    }
    let mut claim = claim_record(
        agent_type,
        "memory_promotion_pending",
        format!(
            "{} governed memory promotion candidate(s) are waiting for review.",
            memory.len()
        ),
        0.85,
        created_at,
    );
    for candidate in memory {
        claim = claim.with_evidence(candidate.candidate_id.clone());
    }
    open_questions
        .push("Which candidates should be promoted into Archon's memory graph?".to_string());
    claims.push(claim);
}

fn persist_claims(
    db: &DbInstance,
    claims: &[archon_learning::agent_knowledge_digest::AgentKnowledgeClaimRecord],
) -> Result<Vec<String>> {
    let mut ids = Vec::new();
    for claim in claims {
        let event = archon_learning::agent_knowledge_digest::insert_agent_knowledge_claim(
            db, "archon", claim,
        )?;
        ids.push(event.event_id);
    }
    Ok(ids)
}

fn claim_record(
    agent_type: &str,
    claim_type: &str,
    claim: String,
    confidence: f32,
    created_at: &str,
) -> archon_learning::agent_knowledge_digest::AgentKnowledgeClaimRecord {
    archon_learning::agent_knowledge_digest::AgentKnowledgeClaimRecord::new(
        stable_claim_id(agent_type, claim_type, &claim),
        agent_type.to_string(),
        claim_type.to_string(),
        claim,
        confidence,
        created_at.to_string(),
    )
}

fn stable_claim_id(agent_type: &str, claim_type: &str, claim: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(agent_type.as_bytes());
    digest.update(b"\0");
    digest.update(claim_type.as_bytes());
    digest.update(b"\0");
    digest.update(claim.as_bytes());
    format!("agent-knowledge-{}", hex::encode(digest.finalize()))
}

fn print_digest(report: &AgentKnowledgeDigestReport) {
    println!("Agent knowledge digest: {}", report.agent_type);
    if report.claims.is_empty() {
        println!("No digest claims generated from current Cozo evidence.");
        return;
    }
    println!("Claims: {}", report.claims.len());
    for claim in &report.claims {
        println!(
            "- [{}] {:.2} {}",
            claim.claim_type, claim.confidence, claim.claim
        );
        if !claim.evidence_ids.is_empty() {
            println!("  evidence: {}", claim.evidence_ids.join(", "));
        }
    }
    if !report.contradictions.is_empty() {
        println!("\nContradictions:");
        for contradiction in &report.contradictions {
            println!("- {contradiction}");
        }
    }
    if !report.open_questions.is_empty() {
        println!("\nOpen questions:");
        for question in &report.open_questions {
            println!("- {question}");
        }
    }
    if !report.persisted_event_ids.is_empty() {
        println!("\nPersisted: {}", report.persisted_event_ids.len());
    }
}

fn confidence_from_ratio(count: usize, total: usize) -> f32 {
    if total == 0 {
        return 0.0;
    }
    ((count as f32 / total as f32) * 0.8 + 0.2).clamp(0.0, 1.0)
}

fn is_failure(
    record: &archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord,
) -> bool {
    matches!(
        record.completion_status.as_str(),
        "failed" | "failure" | "timed_out" | "timeout"
    ) || record.test_failed
        || record.provider_incident_id.is_some()
}

fn is_success(
    record: &archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord,
) -> bool {
    matches!(record.completion_status.as_str(), "succeeded" | "success")
}

fn high_risk(risk: &str) -> bool {
    matches!(risk, "high" | "critical")
}

#[cfg(test)]
#[path = "agent_evolve_digest_tests.rs"]
mod tests;

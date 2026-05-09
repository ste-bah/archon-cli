//! Governed agent profile history and status surfaces.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::DbInstance;
use serde::Serialize;

pub(crate) fn cmd_show_agent_history(db: &DbInstance, agent_type: &str, json: bool) -> Result<()> {
    let history = AgentHistory::load(db, agent_type)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&history)?);
    } else {
        print_history(&history);
    }
    Ok(())
}

pub(crate) fn cmd_show_agent_status(db: &DbInstance, agent_type: &str, json: bool) -> Result<()> {
    let status = AgentStatus::load(db, agent_type)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        print_status(&status);
    }
    Ok(())
}

#[derive(Debug, Serialize, PartialEq)]
struct AgentHistory {
    agent_type: String,
    versions: Vec<ProfileVersionItem>,
}

impl AgentHistory {
    fn load(db: &DbInstance, agent_type: &str) -> Result<Self> {
        let versions =
            archon_learning::agent_profile_versions::list_agent_profile_versions(db, agent_type)?
                .iter()
                .map(ProfileVersionItem::from)
                .collect();
        Ok(Self {
            agent_type: agent_type.to_string(),
            versions,
        })
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct AgentStatus {
    agent_type: String,
    active_profile: Option<ProfileVersionItem>,
    version_count: usize,
    proposals_by_status: BTreeMap<String, usize>,
    pending_high_risk: usize,
    pending_permission_impacts: usize,
    pending_provider_identity_impacts: usize,
    ledger_total: usize,
    ledger_failures: usize,
    latest_ledger_event: Option<LedgerStatusItem>,
    latest_shadow_evaluation: Option<ShadowStatusItem>,
    memory_candidate_count: usize,
}

impl AgentStatus {
    fn load(db: &DbInstance, agent_type: &str) -> Result<Self> {
        let history = AgentHistory::load(db, agent_type)?;
        let active_profile =
            archon_learning::agent_profile_versions::get_active_agent_profile_version(
                db, agent_type,
            )?
            .as_ref()
            .map(ProfileVersionItem::from);
        let proposals =
            archon_learning::agent_evolution_proposals::list_agent_evolution_proposals(db, None)?
                .into_iter()
                .filter(|proposal| proposal.agent_type == agent_type)
                .collect::<Vec<_>>();
        let ledger =
            archon_learning::agent_evolution_ledger::list_agent_performance_ledger_by_agent(
                db, agent_type,
            )?;
        let shadows =
            archon_learning::agent_shadow_evaluations::list_agent_shadow_evaluations_by_agent(
                db, agent_type,
            )?;
        let memory_candidates =
            archon_learning::memory_promotion_candidates::list_memory_promotion_candidates_by_agent(
                db, agent_type,
            )?;

        Ok(Self {
            agent_type: agent_type.to_string(),
            active_profile,
            version_count: history.versions.len(),
            proposals_by_status: proposal_status_counts(&proposals),
            pending_high_risk: proposals
                .iter()
                .filter(|proposal| proposal.status == "pending" && high_risk(&proposal.risk_level))
                .count(),
            pending_permission_impacts: proposals
                .iter()
                .filter(|proposal| proposal.status == "pending" && proposal.affects_permissions)
                .count(),
            pending_provider_identity_impacts: proposals
                .iter()
                .filter(|proposal| {
                    proposal.status == "pending" && proposal.affects_provider_identity
                })
                .count(),
            ledger_total: ledger.len(),
            ledger_failures: ledger.iter().filter(|record| is_failure(record)).count(),
            latest_ledger_event: ledger.first().map(LedgerStatusItem::from),
            latest_shadow_evaluation: shadows.first().map(ShadowStatusItem::from),
            memory_candidate_count: memory_candidates.len(),
        })
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct ProfileVersionItem {
    version_id: String,
    version_number: i64,
    parent_version_id: Option<String>,
    source: String,
    proposal_id: Option<String>,
    is_active: bool,
    is_rollback_target: bool,
    prompt_hash: Option<String>,
    tools_hash: Option<String>,
    model_hash: Option<String>,
    memory_hash: Option<String>,
    created_at: String,
}

impl From<&archon_learning::agent_profile_versions::AgentProfileVersionRecord>
    for ProfileVersionItem
{
    fn from(record: &archon_learning::agent_profile_versions::AgentProfileVersionRecord) -> Self {
        Self {
            version_id: record.version_id.clone(),
            version_number: record.version_number,
            parent_version_id: record.parent_version_id.clone(),
            source: record.source.clone(),
            proposal_id: record.created_by_proposal_id.clone(),
            is_active: record.is_active,
            is_rollback_target: record.is_rollback_target,
            prompt_hash: record.prompt_hash.clone(),
            tools_hash: record.tools_hash.clone(),
            model_hash: record.model_hash.clone(),
            memory_hash: record.memory_hash.clone(),
            created_at: record.created_at.clone(),
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct LedgerStatusItem {
    event_id: String,
    completion_status: String,
    provider_id: Option<String>,
    provider_incident_id: Option<String>,
    created_at: String,
}

impl From<&archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord>
    for LedgerStatusItem
{
    fn from(
        record: &archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord,
    ) -> Self {
        Self {
            event_id: record.event_id.clone(),
            completion_status: record.completion_status.clone(),
            provider_id: record.provider_id.clone(),
            provider_incident_id: record.provider_incident_id.clone(),
            created_at: record.created_at.clone(),
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct ShadowStatusItem {
    evaluation_id: String,
    proposal_id: String,
    verdict: String,
    candidate_score: f64,
    regression_count: i64,
    improvement_count: i64,
    created_at: String,
}

impl From<&archon_learning::agent_shadow_evaluations::AgentShadowEvaluationRecord>
    for ShadowStatusItem
{
    fn from(
        record: &archon_learning::agent_shadow_evaluations::AgentShadowEvaluationRecord,
    ) -> Self {
        Self {
            evaluation_id: record.evaluation_id.clone(),
            proposal_id: record.proposal_id.clone(),
            verdict: record.verdict.clone(),
            candidate_score: record.candidate_score,
            regression_count: record.regression_count,
            improvement_count: record.improvement_count,
            created_at: record.created_at.clone(),
        }
    }
}

fn print_history(history: &AgentHistory) {
    println!("Agent profile history: {}", history.agent_type);
    if history.versions.is_empty() {
        println!("No governed profile versions found.");
        return;
    }
    println!(
        "{:<24} {:<5} {:<8} {:<14} {:<24} created",
        "version_id", "no", "active", "source", "proposal"
    );
    for version in &history.versions {
        println!(
            "{:<24} {:<5} {:<8} {:<14} {:<24} {}",
            version.version_id,
            version.version_number,
            yes_no(version.is_active),
            version.source,
            version.proposal_id.as_deref().unwrap_or("-"),
            version.created_at
        );
    }
}

fn print_status(status: &AgentStatus) {
    println!("Agent evolution status: {}", status.agent_type);
    match &status.active_profile {
        Some(active) => println!(
            "Active profile: {} v{} ({})",
            active.version_id, active.version_number, active.source
        ),
        None => println!("Active profile: none"),
    }
    println!("Profile versions: {}", status.version_count);
    println!("Proposals: {}", format_counts(&status.proposals_by_status));
    println!(
        "Pending risks: high={} permissions={} provider_identity={}",
        status.pending_high_risk,
        status.pending_permission_impacts,
        status.pending_provider_identity_impacts
    );
    println!(
        "Ledger: total={} failures={}",
        status.ledger_total, status.ledger_failures
    );
    if let Some(event) = &status.latest_ledger_event {
        println!(
            "Latest ledger: {} {} provider={}",
            event.event_id,
            event.completion_status,
            event.provider_id.as_deref().unwrap_or("-")
        );
    }
    if let Some(shadow) = &status.latest_shadow_evaluation {
        println!(
            "Latest shadow: {} verdict={} score={:.3}",
            shadow.evaluation_id, shadow.verdict, shadow.candidate_score
        );
    }
    println!("Memory candidates: {}", status.memory_candidate_count);
}

fn proposal_status_counts(
    proposals: &[archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord],
) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for proposal in proposals {
        *counts.entry(proposal.status.clone()).or_default() += 1;
    }
    counts
}

fn format_counts(counts: &BTreeMap<String, usize>) -> String {
    if counts.is_empty() {
        return "-".to_string();
    }
    counts
        .iter()
        .map(|(status, count)| format!("{status}:{count}"))
        .collect::<Vec<_>>()
        .join(", ")
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

fn high_risk(risk: &str) -> bool {
    matches!(risk, "high" | "critical")
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

#[cfg(test)]
#[path = "agent_evolve_history_tests.rs"]
mod tests;

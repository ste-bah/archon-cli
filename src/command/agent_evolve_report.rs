//! Read-only reporting for governed agent evolution.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::DbInstance;
use serde::Serialize;

const RECENT_LIMIT: usize = 5;

pub(crate) fn cmd_report_agent_evolution(
    db: &DbInstance,
    agent_type: &str,
    json: bool,
) -> Result<()> {
    let report = AgentEvolutionReport::load(db, agent_type)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_report(&report);
    }
    Ok(())
}

#[derive(Debug, Serialize, PartialEq)]
struct AgentEvolutionReport {
    agent_type: String,
    active_profile: Option<ActiveProfileSummary>,
    profile_version_count: usize,
    proposals: ProposalSummary,
    ledger: LedgerSummary,
    shadow: ShadowSummary,
    memory: MemorySummary,
}

impl AgentEvolutionReport {
    fn load(db: &DbInstance, agent_type: &str) -> Result<Self> {
        let active_profile =
            archon_learning::agent_profile_versions::get_active_agent_profile_version(
                db, agent_type,
            )?
            .map(ActiveProfileSummary::from);
        let profile_version_count =
            archon_learning::agent_profile_versions::list_agent_profile_versions(db, agent_type)?
                .len();
        let proposals =
            archon_learning::agent_evolution_proposals::list_agent_evolution_proposals(db, None)?
                .into_iter()
                .filter(|proposal| proposal.agent_type == agent_type)
                .collect::<Vec<_>>();
        let ledger =
            archon_learning::agent_evolution_ledger::list_agent_performance_ledger_by_agent(
                db, agent_type,
            )?;
        let shadow =
            archon_learning::agent_shadow_evaluations::list_agent_shadow_evaluations_by_agent(
                db, agent_type,
            )?;
        let memory =
            archon_learning::memory_promotion_candidates::list_memory_promotion_candidates_by_agent(
                db, agent_type,
            )?;

        Ok(Self {
            agent_type: agent_type.to_string(),
            active_profile,
            profile_version_count,
            proposals: ProposalSummary::from_records(&proposals),
            ledger: LedgerSummary::from_records(&ledger),
            shadow: ShadowSummary::from_records(&shadow),
            memory: MemorySummary::from_records(&memory),
        })
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct ActiveProfileSummary {
    version_id: String,
    version_number: i64,
    source: String,
    proposal_id: Option<String>,
    created_at: String,
}

impl From<archon_learning::agent_profile_versions::AgentProfileVersionRecord>
    for ActiveProfileSummary
{
    fn from(record: archon_learning::agent_profile_versions::AgentProfileVersionRecord) -> Self {
        Self {
            version_id: record.version_id,
            version_number: record.version_number,
            source: record.source,
            proposal_id: record.created_by_proposal_id,
            created_at: record.created_at,
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct ProposalSummary {
    total: usize,
    by_status: BTreeMap<String, usize>,
    by_risk: BTreeMap<String, usize>,
    by_kind: BTreeMap<String, usize>,
    provider_identity_impacts: usize,
    permission_impacts: usize,
    recent: Vec<ProposalItem>,
}

impl ProposalSummary {
    fn from_records(
        records: &[archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord],
    ) -> Self {
        let mut by_status = BTreeMap::new();
        let mut by_risk = BTreeMap::new();
        let mut by_kind = BTreeMap::new();
        for record in records {
            count(&mut by_status, &record.status);
            count(&mut by_risk, &record.risk_level);
            count(&mut by_kind, &record.kind);
        }
        Self {
            total: records.len(),
            by_status,
            by_risk,
            by_kind,
            provider_identity_impacts: records
                .iter()
                .filter(|record| record.affects_provider_identity)
                .count(),
            permission_impacts: records
                .iter()
                .filter(|record| record.affects_permissions)
                .count(),
            recent: records
                .iter()
                .take(RECENT_LIMIT)
                .map(ProposalItem::from)
                .collect(),
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct ProposalItem {
    proposal_id: String,
    kind: String,
    status: String,
    risk: String,
    created_at: String,
}

impl From<&archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord>
    for ProposalItem
{
    fn from(
        record: &archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord,
    ) -> Self {
        Self {
            proposal_id: record.proposal_id.clone(),
            kind: record.kind.clone(),
            status: record.status.clone(),
            risk: record.risk_level.clone(),
            created_at: record.created_at.clone(),
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct LedgerSummary {
    total: usize,
    by_status: BTreeMap<String, usize>,
    provider_incidents: usize,
    test_failures: usize,
    user_corrections: usize,
    average_quality: Option<f64>,
    average_l_score: Option<f64>,
    recent: Vec<LedgerItem>,
}

impl LedgerSummary {
    fn from_records(
        records: &[archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord],
    ) -> Self {
        let mut by_status = BTreeMap::new();
        for record in records {
            count(&mut by_status, &record.completion_status);
        }
        Self {
            total: records.len(),
            by_status,
            provider_incidents: records
                .iter()
                .filter(|record| record.provider_incident_id.is_some())
                .count(),
            test_failures: records.iter().filter(|record| record.test_failed).count(),
            user_corrections: records
                .iter()
                .filter(|record| record.user_corrected == Some(true))
                .count(),
            average_quality: average(records.iter().filter_map(|record| record.quality_score)),
            average_l_score: average(records.iter().filter_map(|record| record.l_score)),
            recent: records
                .iter()
                .take(RECENT_LIMIT)
                .map(LedgerItem::from)
                .collect(),
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct LedgerItem {
    event_id: String,
    completion_status: String,
    provider_id: Option<String>,
    provider_incident_id: Option<String>,
    created_at: String,
}

impl From<&archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord> for LedgerItem {
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
struct ShadowSummary {
    total: usize,
    by_verdict: BTreeMap<String, usize>,
    regressions: i64,
    improvements: i64,
    recent: Vec<ShadowItem>,
}

impl ShadowSummary {
    fn from_records(
        records: &[archon_learning::agent_shadow_evaluations::AgentShadowEvaluationRecord],
    ) -> Self {
        let mut by_verdict = BTreeMap::new();
        for record in records {
            count(&mut by_verdict, &record.verdict);
        }
        Self {
            total: records.len(),
            by_verdict,
            regressions: records.iter().map(|record| record.regression_count).sum(),
            improvements: records.iter().map(|record| record.improvement_count).sum(),
            recent: records
                .iter()
                .take(RECENT_LIMIT)
                .map(ShadowItem::from)
                .collect(),
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct ShadowItem {
    evaluation_id: String,
    proposal_id: String,
    verdict: String,
    candidate_score: f64,
    created_at: String,
}

impl From<&archon_learning::agent_shadow_evaluations::AgentShadowEvaluationRecord> for ShadowItem {
    fn from(
        record: &archon_learning::agent_shadow_evaluations::AgentShadowEvaluationRecord,
    ) -> Self {
        Self {
            evaluation_id: record.evaluation_id.clone(),
            proposal_id: record.proposal_id.clone(),
            verdict: record.verdict.clone(),
            candidate_score: record.candidate_score,
            created_at: record.created_at.clone(),
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct MemorySummary {
    total: usize,
    proposal_required: usize,
    top_candidates: Vec<MemoryItem>,
}

impl MemorySummary {
    fn from_records(
        records: &[archon_learning::memory_promotion_candidates::MemoryPromotionCandidateRecord],
    ) -> Self {
        Self {
            total: records.len(),
            proposal_required: records
                .iter()
                .filter(|record| record.proposal_required)
                .count(),
            top_candidates: records
                .iter()
                .take(RECENT_LIMIT)
                .map(MemoryItem::from)
                .collect(),
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct MemoryItem {
    candidate_id: String,
    target: String,
    score: f64,
    claim: String,
}

impl From<&archon_learning::memory_promotion_candidates::MemoryPromotionCandidateRecord>
    for MemoryItem
{
    fn from(
        record: &archon_learning::memory_promotion_candidates::MemoryPromotionCandidateRecord,
    ) -> Self {
        Self {
            candidate_id: record.candidate_id.clone(),
            target: record.target.clone(),
            score: promotion_score(record),
            claim: record.claim.clone(),
        }
    }
}

fn print_report(report: &AgentEvolutionReport) {
    println!("Agent evolution report: {}", report.agent_type);
    if let Some(active) = &report.active_profile {
        println!(
            "Active profile: {} v{} ({})",
            active.version_id, active.version_number, active.source
        );
    } else {
        println!("Active profile: none");
    }
    println!("Profile versions: {}", report.profile_version_count);
    println!(
        "Proposals: {} | status [{}] | risk [{}]",
        report.proposals.total,
        format_counts(&report.proposals.by_status),
        format_counts(&report.proposals.by_risk)
    );
    println!(
        "Impacts: provider_identity={} permissions={}",
        report.proposals.provider_identity_impacts, report.proposals.permission_impacts
    );
    println!(
        "Ledger: {} | status [{}] | provider_incidents={} test_failures={} corrections={}",
        report.ledger.total,
        format_counts(&report.ledger.by_status),
        report.ledger.provider_incidents,
        report.ledger.test_failures,
        report.ledger.user_corrections
    );
    println!(
        "Scores: quality={} l_score={}",
        format_optional_score(report.ledger.average_quality),
        format_optional_score(report.ledger.average_l_score)
    );
    println!(
        "Shadow: {} | verdict [{}] | regressions={} improvements={}",
        report.shadow.total,
        format_counts(&report.shadow.by_verdict),
        report.shadow.regressions,
        report.shadow.improvements
    );
    println!(
        "Memory candidates: {} | proposal_required={}",
        report.memory.total, report.memory.proposal_required
    );
    print_recent_proposals(&report.proposals.recent);
}

fn print_recent_proposals(recent: &[ProposalItem]) {
    if recent.is_empty() {
        return;
    }
    println!("\nRecent proposals:");
    for item in recent {
        println!(
            "- {} {} {} {}",
            item.proposal_id, item.kind, item.status, item.risk
        );
    }
}

fn count(counts: &mut BTreeMap<String, usize>, key: &str) {
    *counts.entry(key.to_string()).or_default() += 1;
}

fn average(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut total = 0.0;
    let mut count = 0;
    for value in values {
        total += value;
        count += 1;
    }
    (count > 0).then_some(total / count as f64)
}

fn format_counts(counts: &BTreeMap<String, usize>) -> String {
    if counts.is_empty() {
        return "-".to_string();
    }
    counts
        .iter()
        .map(|(key, value)| format!("{key}:{value}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_optional_score(score: Option<f64>) -> String {
    score
        .map(|value| format!("{value:.3}"))
        .unwrap_or_else(|| "-".to_string())
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
#[path = "agent_evolve_report_tests.rs"]
mod tests;

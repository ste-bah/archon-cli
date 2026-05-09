//! Detailed inspection for governed agent evolution proposals.

use anyhow::Result;
use cozo::DbInstance;
use serde::Serialize;

const EVIDENCE_LIMIT: usize = 8;
const SHADOW_LIMIT: usize = 5;

pub(crate) fn cmd_inspect_agent_evolution(
    db: &DbInstance,
    proposal_id: &str,
    json: bool,
) -> Result<()> {
    let inspection = AgentEvolutionInspection::load(db, proposal_id)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&inspection)?);
    } else {
        print_inspection(&inspection);
    }
    Ok(())
}

#[derive(Debug, Serialize, PartialEq)]
struct AgentEvolutionInspection {
    proposal: ProposalInspection,
    compatibility: CompatibilityInspection,
    evidence: Vec<EvidenceInspection>,
    shadow_evaluations: Vec<ShadowInspection>,
}

impl AgentEvolutionInspection {
    fn load(db: &DbInstance, proposal_id: &str) -> Result<Self> {
        let proposal = archon_learning::agent_evolution_proposals::get_agent_evolution_proposal(
            db,
            proposal_id,
        )?
        .ok_or_else(|| anyhow::anyhow!("agent evolution proposal not found: {proposal_id}"))?;
        let evidence = proposal
            .evidence_ids
            .iter()
            .take(EVIDENCE_LIMIT)
            .map(|evidence_id| summarize_evidence(db, evidence_id))
            .collect::<Result<Vec<_>>>()?;
        let shadows =
            archon_learning::agent_shadow_evaluations::list_agent_shadow_evaluations_by_proposal(
                db,
                proposal_id,
            )?;

        Ok(Self {
            compatibility: CompatibilityInspection::from(&proposal),
            proposal: ProposalInspection::from(proposal),
            evidence,
            shadow_evaluations: shadows
                .iter()
                .take(SHADOW_LIMIT)
                .map(ShadowInspection::from)
                .collect(),
        })
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct ProposalInspection {
    proposal_id: String,
    agent_type: String,
    current_version: String,
    proposed_version: String,
    kind: String,
    status: String,
    risk_level: String,
    policy_decision: String,
    expected_impact: String,
    rollback_target_version: String,
    evidence_count: usize,
    created_at: String,
    diff: String,
}

impl From<archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord>
    for ProposalInspection
{
    fn from(
        record: archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord,
    ) -> Self {
        Self {
            proposal_id: record.proposal_id,
            agent_type: record.agent_type,
            current_version: record.current_version,
            proposed_version: record.proposed_version,
            kind: record.kind,
            status: record.status,
            risk_level: record.risk_level,
            policy_decision: record.policy_decision,
            expected_impact: record.expected_impact,
            rollback_target_version: record.rollback_target_version,
            evidence_count: record.evidence_ids.len(),
            created_at: record.created_at,
            diff: record.diff,
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct CompatibilityInspection {
    anthropic_spoof_status: String,
    provider_identity_affected: bool,
    permissions_affected: bool,
    manual_review_required: bool,
}

impl From<&archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord>
    for CompatibilityInspection
{
    fn from(
        record: &archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord,
    ) -> Self {
        let manual_review_required = record.affects_provider_identity
            || record.affects_permissions
            || high_risk(&record.risk_level);
        let anthropic_spoof_status = if record.affects_provider_identity {
            "requires_manual_spoof_review"
        } else {
            "unaffected"
        };
        Self {
            anthropic_spoof_status: anthropic_spoof_status.to_string(),
            provider_identity_affected: record.affects_provider_identity,
            permissions_affected: record.affects_permissions,
            manual_review_required,
        }
    }
}

#[derive(Debug, Serialize, PartialEq)]
struct EvidenceInspection {
    evidence_id: String,
    source: String,
    summary: String,
    created_at: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
struct ShadowInspection {
    evaluation_id: String,
    verdict: String,
    baseline_score: f64,
    candidate_score: f64,
    regression_count: i64,
    improvement_count: i64,
    created_at: String,
}

impl From<&archon_learning::agent_shadow_evaluations::AgentShadowEvaluationRecord>
    for ShadowInspection
{
    fn from(
        record: &archon_learning::agent_shadow_evaluations::AgentShadowEvaluationRecord,
    ) -> Self {
        Self {
            evaluation_id: record.evaluation_id.clone(),
            verdict: record.verdict.clone(),
            baseline_score: record.baseline_score,
            candidate_score: record.candidate_score,
            regression_count: record.regression_count,
            improvement_count: record.improvement_count,
            created_at: record.created_at.clone(),
        }
    }
}

fn summarize_evidence(db: &DbInstance, evidence_id: &str) -> Result<EvidenceInspection> {
    if let Some(record) =
        archon_learning::agent_evolution_ledger::get_agent_performance_ledger_record(
            db,
            evidence_id,
        )?
    {
        return Ok(EvidenceInspection {
            evidence_id: evidence_id.to_string(),
            source: "agent_performance_ledger".to_string(),
            summary: ledger_summary(&record),
            created_at: Some(record.created_at),
        });
    }
    if let Some(record) =
        archon_learning::permission_runtime_events::get_permission_runtime_event(db, evidence_id)?
    {
        return Ok(EvidenceInspection {
            evidence_id: evidence_id.to_string(),
            source: "permission_runtime_events".to_string(),
            summary: permission_summary(&record),
            created_at: Some(record.created_at),
        });
    }
    Ok(EvidenceInspection {
        evidence_id: evidence_id.to_string(),
        source: "unknown".to_string(),
        summary: "evidence row not found in local learning store".to_string(),
        created_at: None,
    })
}

fn ledger_summary(
    record: &archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord,
) -> String {
    let provider = record.provider_id.as_deref().unwrap_or("-");
    let incident = record.provider_incident_id.as_deref().unwrap_or("-");
    format!(
        "status={} provider={} incident={} corrected={} test_failed={}",
        record.completion_status,
        provider,
        incident,
        yes_no(record.user_corrected == Some(true)),
        yes_no(record.test_failed)
    )
}

fn permission_summary(
    record: &archon_learning::permission_runtime_events::PermissionRuntimeEventRecord,
) -> String {
    let reason = record.reason_code.as_deref().unwrap_or("-");
    let sandbox = record.sandbox_backend.as_deref().unwrap_or("-");
    format!(
        "decision={} tool={} mode={} reason={} sandbox={}",
        record.decision, record.tool_name, record.permission_mode, reason, sandbox
    )
}

fn print_inspection(inspection: &AgentEvolutionInspection) {
    let proposal = &inspection.proposal;
    println!("Proposal: {}", proposal.proposal_id);
    println!("Agent:    {}", proposal.agent_type);
    println!("Kind:     {}", proposal.kind);
    println!("Status:   {}", proposal.status);
    println!("Risk:     {}", proposal.risk_level);
    println!("Decision: {}", proposal.policy_decision);
    println!("Current:  {}", proposal.current_version);
    println!("Proposed: {}", proposal.proposed_version);
    println!("Rollback: {}", proposal.rollback_target_version);
    println!(
        "Compatibility: Anthropic spoof {}",
        inspection.compatibility.anthropic_spoof_status
    );
    println!(
        "Impacts: provider_identity={} permissions={} manual_review={}",
        yes_no(inspection.compatibility.provider_identity_affected),
        yes_no(inspection.compatibility.permissions_affected),
        yes_no(inspection.compatibility.manual_review_required)
    );
    if !proposal.expected_impact.trim().is_empty() {
        println!("Expected impact: {}", proposal.expected_impact);
    }
    print_evidence(inspection);
    print_shadow(inspection);
    print_diff(&proposal.diff);
}

fn print_evidence(inspection: &AgentEvolutionInspection) {
    println!(
        "\nEvidence: {} total, showing {}",
        inspection.proposal.evidence_count,
        inspection.evidence.len()
    );
    if inspection.evidence.is_empty() {
        println!("(no evidence recorded)");
        return;
    }
    for evidence in &inspection.evidence {
        println!(
            "- {} [{}] {}",
            evidence.evidence_id, evidence.source, evidence.summary
        );
    }
}

fn print_shadow(inspection: &AgentEvolutionInspection) {
    if inspection.shadow_evaluations.is_empty() {
        return;
    }
    println!("\nShadow evaluations:");
    for shadow in &inspection.shadow_evaluations {
        println!(
            "- {} verdict={} baseline={:.3} candidate={:.3} regressions={} improvements={}",
            shadow.evaluation_id,
            shadow.verdict,
            shadow.baseline_score,
            shadow.candidate_score,
            shadow.regression_count,
            shadow.improvement_count
        );
    }
}

fn print_diff(diff: &str) {
    println!("\nDiff:");
    if diff.trim().is_empty() {
        println!("(no diff recorded)");
    } else {
        println!("{diff}");
    }
}

fn high_risk(risk: &str) -> bool {
    matches!(
        risk.to_ascii_lowercase().as_str(),
        "high" | "critical" | "manual_review_required"
    )
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

#[cfg(test)]
#[path = "agent_evolve_inspect_tests.rs"]
mod tests;

//! Cozo-backed shadow evaluation workflow for governed agent proposals.

use anyhow::Result;
use cozo::DbInstance;

use archon_learning::agent_evolution_ledger::AgentPerformanceLedgerRecord;
use archon_learning::agent_evolution_proposals::AgentEvolutionProposalRecord;
use archon_learning::agent_profile_versions::AgentProfileVersionRecord;
use archon_learning::agent_shadow_evaluations::AgentShadowEvaluationRecord;

pub(crate) fn cmd_run_shadow_evaluation(
    db: &DbInstance,
    proposal_id: &str,
    task_set: Option<&str>,
    json: bool,
) -> Result<()> {
    let proposal =
        archon_learning::agent_evolution_proposals::get_agent_evolution_proposal(db, proposal_id)?
            .ok_or_else(|| anyhow::anyhow!("agent evolution proposal not found: {proposal_id}"))?;

    let evaluation = build_shadow_evaluation(db, &proposal, task_set)?;
    archon_learning::agent_shadow_evaluations::insert_agent_shadow_evaluation(db, &evaluation)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&evaluation)?);
    } else {
        print_shadow_evaluation(&evaluation);
    }
    Ok(())
}

fn build_shadow_evaluation(
    db: &DbInstance,
    proposal: &AgentEvolutionProposalRecord,
    task_set: Option<&str>,
) -> Result<AgentShadowEvaluationRecord> {
    let versions = archon_learning::agent_profile_versions::list_agent_profile_versions(
        db,
        &proposal.agent_type,
    )?;
    let candidate_profile = newest_profile_for_proposal(&versions, &proposal.proposal_id);
    let baseline_version = candidate_profile
        .and_then(|version| version.parent_version_id.clone())
        .filter(|version| !version.trim().is_empty())
        .unwrap_or_else(|| proposal.current_version.clone());
    let candidate_version = candidate_profile
        .map(|version| version.version_id.clone())
        .unwrap_or_else(|| proposal.proposed_version.clone());

    let ledger = archon_learning::agent_evolution_ledger::list_agent_performance_ledger_by_agent(
        db,
        &proposal.agent_type,
    )?;
    let baseline_rows = baseline_rows(&ledger, proposal, &baseline_version);
    let candidate_rows = rows_for_version(&ledger, &candidate_version);
    let baseline_score = average_score(&baseline_rows);
    let candidate_score = average_score(&candidate_rows);
    let (regressions, improvements) = compare_task_scores(&baseline_rows, &candidate_rows);
    let verdict = verdict(
        !candidate_rows.is_empty(),
        baseline_score,
        candidate_score,
        regressions,
        improvements,
    );

    let mut evaluation = AgentShadowEvaluationRecord::new(
        archon_core::agents::evolution::agent_shadow_evaluation_id(),
        proposal.proposal_id.clone(),
        proposal.agent_type.clone(),
        verdict,
        chrono::Utc::now().to_rfc3339(),
    )
    .with_versions(candidate_version.clone(), baseline_version.clone())
    .with_scores(baseline_score, candidate_score)
    .with_counts(regressions as i64, improvements as i64)
    .with_evidence_json(serde_json::json!({
        "mode": "cozo_ledger_shadow",
        "baseline_events": event_ids(&baseline_rows),
        "candidate_events": event_ids(&candidate_rows),
        "candidate_rows": candidate_rows.len(),
        "baseline_rows": baseline_rows.len(),
        "reason": shadow_reason(&candidate_rows),
        "world_model": crate::command::agent_evolve_world_model::world_model_shadow_evidence(
            &proposal.agent_type
        ),
    }));

    if let Some(task_set) = task_set {
        evaluation = evaluation.with_task_set(task_set.to_string());
    }
    Ok(evaluation)
}

fn newest_profile_for_proposal<'a>(
    versions: &'a [AgentProfileVersionRecord],
    proposal_id: &str,
) -> Option<&'a AgentProfileVersionRecord> {
    versions
        .iter()
        .filter(|version| version.created_by_proposal_id.as_deref() == Some(proposal_id))
        .max_by_key(|version| version.version_number)
}

fn baseline_rows<'a>(
    ledger: &'a [AgentPerformanceLedgerRecord],
    proposal: &AgentEvolutionProposalRecord,
    baseline_version: &str,
) -> Vec<&'a AgentPerformanceLedgerRecord> {
    let rows = rows_for_version(ledger, baseline_version);
    if !rows.is_empty() {
        return rows;
    }
    ledger
        .iter()
        .filter(|record| proposal.evidence_ids.contains(&record.event_id))
        .collect()
}

fn rows_for_version<'a>(
    ledger: &'a [AgentPerformanceLedgerRecord],
    version: &str,
) -> Vec<&'a AgentPerformanceLedgerRecord> {
    ledger
        .iter()
        .filter(|record| record.agent_version.as_deref() == Some(version))
        .collect()
}

fn average_score(rows: &[&AgentPerformanceLedgerRecord]) -> f64 {
    if rows.is_empty() {
        return 0.0;
    }
    let total: f64 = rows.iter().map(|row| row_score(row)).sum();
    (total / rows.len() as f64).clamp(0.0, 1.0)
}

fn row_score(row: &AgentPerformanceLedgerRecord) -> f64 {
    let base =
        row.quality_score
            .or(row.completion_rate)
            .unwrap_or(match row.completion_status.as_str() {
                "succeeded" | "success" => 1.0,
                "failed" | "failure" | "cancelled" | "canceled" | "timed_out" | "timeout" => 0.0,
                _ => 0.5,
            });
    let penalty = if row.gate_failed.is_some() || row.test_failed {
        0.15
    } else if row.provider_incident_id.is_some() {
        0.1
    } else {
        0.0
    };
    (base - penalty).clamp(0.0, 1.0)
}

fn compare_task_scores(
    baseline_rows: &[&AgentPerformanceLedgerRecord],
    candidate_rows: &[&AgentPerformanceLedgerRecord],
) -> (usize, usize) {
    let mut regressions = 0usize;
    let mut improvements = 0usize;
    for candidate in candidate_rows {
        let Some(task_hash) = candidate.task_hash.as_deref() else {
            continue;
        };
        let Some(baseline) = baseline_rows
            .iter()
            .find(|row| row.task_hash.as_deref() == Some(task_hash))
        else {
            continue;
        };
        tally_scores(
            row_score(baseline),
            row_score(candidate),
            &mut regressions,
            &mut improvements,
        );
    }

    if regressions == 0
        && improvements == 0
        && !baseline_rows.is_empty()
        && !candidate_rows.is_empty()
    {
        tally_scores(
            average_score(baseline_rows),
            average_score(candidate_rows),
            &mut regressions,
            &mut improvements,
        );
    }
    (regressions, improvements)
}

fn tally_scores(baseline: f64, candidate: f64, regressions: &mut usize, improvements: &mut usize) {
    const DELTA: f64 = 0.05;
    if candidate + DELTA < baseline {
        *regressions += 1;
    } else if candidate > baseline + DELTA {
        *improvements += 1;
    }
}

fn verdict(
    has_candidate_rows: bool,
    baseline_score: f64,
    candidate_score: f64,
    regressions: usize,
    improvements: usize,
) -> &'static str {
    if !has_candidate_rows {
        return "inconclusive";
    }
    if candidate_score < baseline_score && regressions > 0 {
        return "reject";
    }
    if regressions > 0 {
        return "needs_review";
    }
    if candidate_score >= baseline_score && improvements > 0 {
        return "promote";
    }
    "inconclusive"
}

fn shadow_reason(candidate_rows: &[&AgentPerformanceLedgerRecord]) -> &'static str {
    if candidate_rows.is_empty() {
        "no_candidate_ledger_rows"
    } else {
        "candidate_ledger_rows_compared"
    }
}

fn event_ids(rows: &[&AgentPerformanceLedgerRecord]) -> Vec<String> {
    rows.iter().map(|row| row.event_id.clone()).collect()
}

fn print_shadow_evaluation(evaluation: &AgentShadowEvaluationRecord) {
    println!("Shadow evaluation recorded");
    println!("Evaluation:  {}", evaluation.evaluation_id);
    println!("Proposal:    {}", evaluation.proposal_id);
    println!("Agent:       {}", evaluation.agent_type);
    println!(
        "Baseline:    {} ({:.3})",
        evaluation.baseline_version_id.as_deref().unwrap_or("-"),
        evaluation.baseline_score
    );
    println!(
        "Candidate:   {} ({:.3})",
        evaluation.candidate_version_id.as_deref().unwrap_or("-"),
        evaluation.candidate_score
    );
    println!("Verdict:     {}", evaluation.verdict);
    println!(
        "Signals:     {} improvement(s), {} regression(s)",
        evaluation.improvement_count, evaluation.regression_count
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-agent-evolve-shadow-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        archon_learning::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn shadow_evaluation_promotes_candidate_with_better_task_scores() {
        let db = test_db();
        seed_proposal(&db);
        seed_profile_versions(&db);
        seed_ledger(&db, "ledger-base", "agentv-1", "task-a", 0.5);
        seed_ledger(&db, "ledger-cand", "agent-profile-2", "task-a", 0.8);

        let evaluation = build_shadow_evaluation(&db, &proposal_record(), Some("suite-1")).unwrap();

        assert_eq!(evaluation.verdict, "promote");
        assert_eq!(evaluation.task_set_id.as_deref(), Some("suite-1"));
        assert_eq!(
            evaluation.candidate_version_id.as_deref(),
            Some("agent-profile-2")
        );
        assert_eq!(evaluation.improvement_count, 1);
    }

    #[test]
    fn shadow_evaluation_is_inconclusive_without_candidate_rows() {
        let db = test_db();
        seed_proposal(&db);
        seed_ledger(&db, "ledger-base", "agentv-1", "task-a", 0.7);

        let evaluation = build_shadow_evaluation(&db, &proposal_record(), None).unwrap();

        assert_eq!(evaluation.verdict, "inconclusive");
        assert_eq!(
            evaluation.evidence_json["reason"],
            "no_candidate_ledger_rows"
        );
    }

    fn seed_proposal(db: &DbInstance) {
        archon_learning::agent_evolution_proposals::insert_agent_evolution_proposal(
            db,
            &proposal_record(),
        )
        .unwrap();
    }

    fn proposal_record() -> AgentEvolutionProposalRecord {
        AgentEvolutionProposalRecord::new(
            "agent-evo-prop-1",
            "reviewer",
            "agentv-1",
            "agentv-2",
            "prompt_profile",
            "2026-05-08T12:00:00Z",
        )
        .with_evidence("ledger-base")
    }

    fn seed_profile_versions(db: &DbInstance) {
        archon_learning::agent_profile_versions::insert_agent_profile_version(
            db,
            &AgentProfileVersionRecord::new(
                "agent-profile-2",
                "reviewer",
                2,
                "governed_proposal",
                "2026-05-08T12:01:00Z",
            )
            .with_parent("agentv-1")
            .with_proposal("agent-evo-prop-1"),
        )
        .unwrap();
    }

    fn seed_ledger(db: &DbInstance, event_id: &str, version: &str, task: &str, score: f64) {
        archon_learning::agent_evolution_ledger::insert_agent_performance_ledger_record(
            db,
            &AgentPerformanceLedgerRecord::new(
                event_id,
                "reviewer",
                "succeeded",
                "2026-05-08T12:02:00Z",
            )
            .with_agent_version(version)
            .with_scores(Some(score), None)
            .add_evidence(task),
        )
        .unwrap();
    }
}

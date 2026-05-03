//! CRUD operations for completion-integrity Cozo relations.
//!
//! Follows the same `:put` / read-modify-write pattern as
//! `archon-docs::store` and `archon-pipeline::gametheory::facade`.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

use super::models::*;
use super::schema::ensure_completion_schema;

/// Helper: extract a required string column from a Cozo row.
fn req_str(row: &[DataValue], col: usize) -> Result<&str> {
    row[col]
        .get_str()
        .ok_or_else(|| anyhow::anyhow!("missing required string column {col}"))
}

/// Helper: extract an optional string column.
fn opt_str(dv: &DataValue) -> Option<String> {
    dv.get_str()
        .map(|s: &str| s.to_string())
        .filter(|s: &String| !s.is_empty())
}

// ── CompletionClaim ──────────────────────────────────────────────────────────

pub fn insert_completion_claim(db: &DbInstance, claim: &CompletionClaim) -> Result<()> {
    ensure_completion_schema(db)?;
    let mut params = BTreeMap::new();
    params.insert("cid".into(), DataValue::from(claim.claim_id.as_str()));
    params.insert("rid".into(), DataValue::from(claim.run_id.as_str()));
    params.insert("ak".into(), DataValue::from(claim.agent_key.as_deref().unwrap_or("")));
    params.insert("md".into(), DataValue::from(claim.model.as_deref().unwrap_or("")));
    params.insert("tt".into(), DataValue::from(claim.task_type.as_str()));
    params.insert("ck".into(), DataValue::from(claim_kind_str(&claim.claim_kind)));
    params.insert("ct".into(), DataValue::from(claim.claim_text.as_str()));
    params.insert(
        "rej".into(),
        DataValue::from(serde_json::to_string(&claim.required_evidence).unwrap_or_default().as_str()),
    );
    params.insert(
        "lej".into(),
        DataValue::from(serde_json::to_string(&claim.linked_evidence_ids).unwrap_or_default().as_str()),
    );
    params.insert("v".into(), DataValue::from(claim.verified));
    params.insert("ca".into(), DataValue::from(claim.created_at.as_str()));

    db.run_script(
        "?[claim_id, run_id, agent_key, model, task_type, claim_kind, claim_text, \
         required_evidence_json, linked_evidence_json, verified, created_at] \
         <- [[$cid, $rid, $ak, $md, $tt, $ck, $ct, $rej, $lej, $v, $ca]] \
         :put completion_claims { claim_id => run_id, agent_key, model, task_type, \
         claim_kind, claim_text, required_evidence_json, linked_evidence_json, \
         verified, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert completion_claims failed: {e}"))?;
    Ok(())
}

pub fn get_completion_claims_by_run(db: &DbInstance, run_id: &str) -> Result<Vec<CompletionClaim>> {
    let mut params = BTreeMap::new();
    params.insert("rid".into(), DataValue::from(run_id));

    let result = db.run_script(
        "?[claim_id, agent_key, model, task_type, claim_kind, claim_text, \
         required_evidence_json, linked_evidence_json, verified, created_at] \
         := *completion_claims{claim_id, run_id: $rid, agent_key, model, \
         task_type, claim_kind, claim_text, required_evidence_json, \
         linked_evidence_json, verified, created_at}",
        params,
        ScriptMutability::Immutable,
    )
    .map_err(|e| anyhow::anyhow!("query completion_claims failed: {e}"))?;

    result
        .rows
        .iter()
        .map(|row| {
            Ok(CompletionClaim {
                claim_id: req_str(row, 0)?.to_string(),
                run_id: run_id.to_string(),
                agent_key: opt_str(&row[1]),
                model: opt_str(&row[2]),
                task_type: req_str(row, 3)?.to_string(),
                claim_kind: parse_claim_kind(req_str(row, 4)?),
                claim_text: req_str(row, 5)?.to_string(),
                required_evidence: parse_evidence_kinds(req_str(row, 6)?),
                linked_evidence_ids: parse_string_list(req_str(row, 7)?),
                verified: row[8].get_bool().unwrap_or(false),
                contradiction_ids: vec![],
                created_at: req_str(row, 9)?.to_string(),
            })
        })
        .collect()
}

// ── CompletionEvidence ───────────────────────────────────────────────────────

pub fn insert_completion_evidence(db: &DbInstance, ev: &CompletionEvidence) -> Result<()> {
    ensure_completion_schema(db)?;
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(ev.evidence_id.as_str()));
    params.insert("rid".into(), DataValue::from(ev.run_id.as_str()));
    params.insert("ek".into(), DataValue::from(evidence_kind_str(&ev.evidence_kind)));
    params.insert("pr".into(), DataValue::from(ev.producer.as_str()));
    params.insert("st".into(), DataValue::from(evidence_status_str(&ev.status)));
    params.insert("ec".into(), DataValue::from(ev.exit_code.unwrap_or(0) as i64));
    params.insert("ih".into(), DataValue::from(ev.input_hash.as_deref().unwrap_or("")));
    params.insert("oh".into(), DataValue::from(ev.output_hash.as_deref().unwrap_or("")));
    params.insert(
        "aj".into(),
        DataValue::from(serde_json::to_string(&ev.artifact_ids).unwrap_or_default().as_str()),
    );
    params.insert("prid".into(), DataValue::from(ev.provenance_record_id.as_str()));
    params.insert("sa".into(), DataValue::from(ev.started_at.as_str()));
    params.insert("coa".into(), DataValue::from(ev.completed_at.as_deref().unwrap_or("")));

    db.run_script(
        "?[evidence_id, run_id, evidence_kind, producer, status, exit_code, \
         input_hash, output_hash, artifact_ids_json, provenance_record_id, \
         started_at, completed_at] \
         <- [[$eid, $rid, $ek, $pr, $st, $ec, $ih, $oh, $aj, $prid, $sa, $coa]] \
         :put completion_evidence { evidence_id => run_id, evidence_kind, producer, \
         status, exit_code, input_hash, output_hash, artifact_ids_json, \
         provenance_record_id, started_at, completed_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert completion_evidence failed: {e}"))?;
    Ok(())
}

pub fn get_evidence_by_run(db: &DbInstance, run_id: &str) -> Result<Vec<CompletionEvidence>> {
    let mut params = BTreeMap::new();
    params.insert("rid".into(), DataValue::from(run_id));

    let result = db.run_script(
        "?[evidence_id, evidence_kind, producer, status, exit_code, \
         input_hash, output_hash, artifact_ids_json, provenance_record_id, \
         started_at, completed_at] \
         := *completion_evidence{evidence_id, run_id: $rid, evidence_kind, \
         producer, status, exit_code, input_hash, output_hash, \
         artifact_ids_json, provenance_record_id, started_at, completed_at}",
        params,
        ScriptMutability::Immutable,
    )
    .map_err(|e| anyhow::anyhow!("query completion_evidence failed: {e}"))?;

    result
        .rows
        .iter()
        .map(|row| {
            Ok(CompletionEvidence {
                evidence_id: req_str(row, 0)?.to_string(),
                run_id: run_id.to_string(),
                evidence_kind: parse_evidence_kind(req_str(row, 1)?),
                producer: req_str(row, 2)?.to_string(),
                status: parse_evidence_status(req_str(row, 3)?),
                exit_code: Some(row[4].get_int().unwrap_or(0) as i32),
                input_hash: opt_str(&row[5]),
                output_hash: opt_str(&row[6]),
                stdout_summary: None,
                stderr_summary: None,
                artifact_ids: parse_string_list(req_str(row, 7)?),
                provenance_record_id: req_str(row, 8)?.to_string(),
                started_at: req_str(row, 9)?.to_string(),
                completed_at: opt_str(&row[10]),
                command_or_operation: None,
            })
        })
        .collect()
}

// ── CompletionReport ─────────────────────────────────────────────────────────

pub fn insert_completion_report(db: &DbInstance, report: &CompletionReport) -> Result<()> {
    ensure_completion_schema(db)?;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), DataValue::from(report.report_id.as_str()));
    params.insert("runid".into(), DataValue::from(report.run_id.as_str()));
    params.insert("fs".into(), DataValue::from(completion_state_str(&report.final_state)));
    params.insert(
        "cj".into(),
        DataValue::from(serde_json::to_string(&report.claims).unwrap_or_default().as_str()),
    );
    params.insert(
        "ej".into(),
        DataValue::from(serde_json::to_string(&report.evidence).unwrap_or_default().as_str()),
    );
    params.insert(
        "fgj".into(),
        DataValue::from(serde_json::to_string(&report.failed_gates).unwrap_or_default().as_str()),
    );
    params.insert(
        "ucj".into(),
        DataValue::from(serde_json::to_string(&report.unverified_claims).unwrap_or_default().as_str()),
    );
    params.insert("cs".into(), DataValue::from(report.calibrated_summary.as_str()));
    params.insert("prid".into(), DataValue::from(report.provenance_record_id.as_str()));
    params.insert("ca".into(), DataValue::from(report.created_at.as_str()));

    db.run_script(
        "?[report_id, run_id, final_state, claims_json, evidence_json, \
         failed_gates_json, unverified_claims_json, calibrated_summary, \
         provenance_record_id, created_at] \
         <- [[$rid, $runid, $fs, $cj, $ej, $fgj, $ucj, $cs, $prid, $ca]] \
         :put completion_reports { report_id => run_id, final_state, claims_json, \
         evidence_json, failed_gates_json, unverified_claims_json, \
         calibrated_summary, provenance_record_id, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert completion_reports failed: {e}"))?;
    Ok(())
}

pub fn get_completion_report(db: &DbInstance, report_id: &str) -> Result<Option<CompletionReport>> {
    let mut params = BTreeMap::new();
    params.insert("rid".into(), DataValue::from(report_id));

    let result = db.run_script(
        "?[run_id, final_state, claims_json, evidence_json, failed_gates_json, \
         unverified_claims_json, calibrated_summary, provenance_record_id, created_at] \
         := *completion_reports{report_id: $rid, run_id, final_state, claims_json, \
         evidence_json, failed_gates_json, unverified_claims_json, \
         calibrated_summary, provenance_record_id, created_at}",
        params,
        ScriptMutability::Immutable,
    )
    .map_err(|e| anyhow::anyhow!("query completion_reports failed: {e}"))?;

    if result.rows.is_empty() {
        return Ok(None);
    }
    let row = &result.rows[0];
    Ok(Some(CompletionReport {
        report_id: report_id.to_string(),
        run_id: req_str(row, 0)?.to_string(),
        final_state: parse_completion_state(req_str(row, 1)?),
        claims: serde_json::from_str(req_str(row, 2)?).unwrap_or_default(),
        evidence: serde_json::from_str(req_str(row, 3)?).unwrap_or_default(),
        failed_gates: serde_json::from_str(req_str(row, 4)?).unwrap_or_default(),
        unverified_claims: serde_json::from_str(req_str(row, 5)?).unwrap_or_default(),
        contradictions: vec![],
        calibrated_summary: req_str(row, 6)?.to_string(),
        provenance_record_id: req_str(row, 7)?.to_string(),
        created_at: req_str(row, 8)?.to_string(),
    }))
}

// ── VerificationGateResult ───────────────────────────────────────────────────

pub fn insert_gate_result(db: &DbInstance, gr: &VerificationGateResult, run_id: &str) -> Result<()> {
    ensure_completion_schema(db)?;
    let mut params = BTreeMap::new();
    params.insert("gid".into(), DataValue::from(gr.gate_id.as_str()));
    params.insert("rid".into(), DataValue::from(run_id));
    params.insert("gn".into(), DataValue::from(gr.gate_name.as_str()));
    params.insert("p".into(), DataValue::from(gr.passed));
    params.insert("rs".into(), DataValue::from(completion_state_str(&gr.resulting_state)));
    params.insert(
        "bcj".into(),
        DataValue::from(serde_json::to_string(&gr.blocked_claims).unwrap_or_default().as_str()),
    );
    params.insert(
        "rmej".into(),
        DataValue::from(
            serde_json::to_string(&gr.required_missing_evidence)
                .unwrap_or_default()
                .as_str(),
        ),
    );
    params.insert("ex".into(), DataValue::from(gr.explanation.as_str()));
    params.insert("prid".into(), DataValue::from(gr.provenance_record_id.as_str()));
    params.insert("ca".into(), DataValue::from(chrono::Utc::now().to_rfc3339().as_str()));

    db.run_script(
        "?[gate_id, run_id, gate_name, passed, resulting_state, \
         blocked_claims_json, required_missing_evidence_json, explanation, \
         provenance_record_id, created_at] \
         <- [[$gid, $rid, $gn, $p, $rs, $bcj, $rmej, $ex, $prid, $ca]] \
         :put verification_gate_results { gate_id => run_id, gate_name, passed, \
         resulting_state, blocked_claims_json, required_missing_evidence_json, \
         explanation, provenance_record_id, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert verification_gate_results failed: {e}"))?;
    Ok(())
}

// ── FalseCompletionIncident ──────────────────────────────────────────────────

pub fn insert_false_completion_incident(
    db: &DbInstance,
    incident: &FalseCompletionIncident,
) -> Result<()> {
    ensure_completion_schema(db)?;
    let mut params = BTreeMap::new();
    params.insert("iid".into(), DataValue::from(incident.incident_id.as_str()));
    params.insert("rid".into(), DataValue::from(incident.run_id.as_str()));
    params.insert("ak".into(), DataValue::from(incident.agent_key.as_deref().unwrap_or("")));
    params.insert("md".into(), DataValue::from(incident.model.as_deref().unwrap_or("")));
    params.insert("tt".into(), DataValue::from(incident.task_type.as_str()));
    params.insert("cs".into(), DataValue::from(incident.claimed_state.as_str()));
    params.insert("as_".into(), DataValue::from(completion_state_str(&incident.actual_state)));
    params.insert(
        "mej".into(),
        DataValue::from(
            serde_json::to_string(&incident.missing_evidence)
                .unwrap_or_default()
                .as_str(),
        ),
    );
    params.insert("uc".into(), DataValue::from(incident.user_correction.as_deref().unwrap_or("")));
    params.insert("sv".into(), DataValue::from(incident_severity_str(&incident.severity)));
    params.insert("lei".into(), DataValue::from(incident.learning_event_id.as_str()));
    params.insert("ca".into(), DataValue::from(incident.created_at.as_str()));

    db.run_script(
        "?[incident_id, run_id, agent_key, model, task_type, claimed_state, \
         actual_state, missing_evidence_json, user_correction, severity, \
         learning_event_id, created_at] \
         <- [[$iid, $rid, $ak, $md, $tt, $cs, $as_, $mej, $uc, $sv, $lei, $ca]] \
         :put false_completion_incidents { incident_id => run_id, agent_key, model, \
         task_type, claimed_state, actual_state, missing_evidence_json, \
         user_correction, severity, learning_event_id, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert false_completion_incidents failed: {e}"))?;
    Ok(())
}

pub fn get_all_incidents(db: &DbInstance) -> Result<Vec<FalseCompletionIncident>> {
    let result = db.run_script(
        "?[incident_id, run_id, agent_key, model, task_type, claimed_state, \
         actual_state, missing_evidence_json, user_correction, severity, \
         learning_event_id, created_at] \
         := *false_completion_incidents{incident_id, run_id, agent_key, model, \
         task_type, claimed_state, actual_state, missing_evidence_json, \
         user_correction, severity, learning_event_id, created_at}",
        Default::default(),
        ScriptMutability::Immutable,
    )
    .map_err(|e| anyhow::anyhow!("query false_completion_incidents failed: {e}"))?;

    result
        .rows
        .iter()
        .map(|row| {
            Ok(FalseCompletionIncident {
                incident_id: req_str(row, 0)?.to_string(),
                run_id: req_str(row, 1)?.to_string(),
                agent_key: opt_str(&row[2]),
                model: opt_str(&row[3]),
                task_type: req_str(row, 4)?.to_string(),
                claimed_state: req_str(row, 5)?.to_string(),
                actual_state: parse_completion_state(req_str(row, 6)?),
                missing_evidence: parse_evidence_kinds(req_str(row, 7)?),
                contradiction_ids: vec![],
                user_correction: opt_str(&row[8]),
                severity: parse_incident_severity(req_str(row, 9)?),
                learning_event_id: req_str(row, 10)?.to_string(),
                created_at: req_str(row, 11)?.to_string(),
            })
        })
        .collect()
}

// ── AgentModelTrustScore ─────────────────────────────────────────────────────

pub fn insert_trust_score(db: &DbInstance, score: &AgentModelTrustScore) -> Result<()> {
    ensure_completion_schema(db)?;
    let mut params = BTreeMap::new();
    params.insert("sid".into(), DataValue::from(score.score_id.as_str()));
    params.insert("wid".into(), DataValue::from(score.workspace_id.as_str()));
    params.insert("ak".into(), DataValue::from(score.agent_key.as_deref().unwrap_or("")));
    params.insert("md".into(), DataValue::from(score.model.as_deref().unwrap_or("")));
    params.insert("tt".into(), DataValue::from(score.task_type.as_str()));
    params.insert("cr".into(), DataValue::from(score.completion_reliability as f64));
    params.insert("eq".into(), DataValue::from(score.evidence_quality as f64));
    params.insert("fc".into(), DataValue::from(score.false_completion_count as i64));
    params.insert("vc".into(), DataValue::from(score.verified_completion_count as i64));
    params.insert("lu".into(), DataValue::from(score.last_updated.as_str()));

    db.run_script(
        "?[score_id, workspace_id, agent_key, model, task_type, \
         completion_reliability, evidence_quality, false_completion_count, \
         verified_completion_count, last_updated] \
         <- [[$sid, $wid, $ak, $md, $tt, $cr, $eq, $fc, $vc, $lu]] \
         :put agent_model_trust_scores { score_id => workspace_id, agent_key, model, \
         task_type, completion_reliability, evidence_quality, \
         false_completion_count, verified_completion_count, last_updated }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert agent_model_trust_scores failed: {e}"))?;
    Ok(())
}

// ── String conversion helpers ────────────────────────────────────────────────

fn claim_kind_str(k: &CompletionClaimKind) -> &'static str {
    match k {
        CompletionClaimKind::Done => "Done",
        CompletionClaimKind::Implemented => "Implemented",
        CompletionClaimKind::Fixed => "Fixed",
        CompletionClaimKind::TestsPass => "TestsPass",
        CompletionClaimKind::BuildPasses => "BuildPasses",
        CompletionClaimKind::Verified => "Verified",
        CompletionClaimKind::Documented => "Documented",
        CompletionClaimKind::Ingested => "Ingested",
        CompletionClaimKind::Indexed => "Indexed",
        CompletionClaimKind::AnswerGrounded => "AnswerGrounded",
    }
}

fn parse_claim_kind(s: &str) -> CompletionClaimKind {
    match s {
        "Done" => CompletionClaimKind::Done,
        "Implemented" => CompletionClaimKind::Implemented,
        "Fixed" => CompletionClaimKind::Fixed,
        "TestsPass" => CompletionClaimKind::TestsPass,
        "BuildPasses" => CompletionClaimKind::BuildPasses,
        "Verified" => CompletionClaimKind::Verified,
        "Documented" => CompletionClaimKind::Documented,
        "Ingested" => CompletionClaimKind::Ingested,
        "Indexed" => CompletionClaimKind::Indexed,
        "AnswerGrounded" => CompletionClaimKind::AnswerGrounded,
        _ => CompletionClaimKind::Done,
    }
}

fn evidence_kind_str(k: &EvidenceKind) -> &'static str {
    match k {
        EvidenceKind::CommandRun => "CommandRun",
        EvidenceKind::TestRun => "TestRun",
        EvidenceKind::BuildResult => "BuildResult",
        EvidenceKind::FileDiff => "FileDiff",
        EvidenceKind::GeneratedArtifact => "GeneratedArtifact",
        EvidenceKind::RetrievalEvidence => "RetrievalEvidence",
        EvidenceKind::GateResult => "GateResult",
        EvidenceKind::ReviewFinding => "ReviewFinding",
        EvidenceKind::CitationTrace => "CitationTrace",
        EvidenceKind::IngestionJob => "IngestionJob",
    }
}

fn parse_evidence_kind(s: &str) -> EvidenceKind {
    match s {
        "CommandRun" => EvidenceKind::CommandRun,
        "TestRun" => EvidenceKind::TestRun,
        "BuildResult" => EvidenceKind::BuildResult,
        "FileDiff" => EvidenceKind::FileDiff,
        "GeneratedArtifact" => EvidenceKind::GeneratedArtifact,
        "RetrievalEvidence" => EvidenceKind::RetrievalEvidence,
        "GateResult" => EvidenceKind::GateResult,
        "ReviewFinding" => EvidenceKind::ReviewFinding,
        "CitationTrace" => EvidenceKind::CitationTrace,
        "IngestionJob" => EvidenceKind::IngestionJob,
        _ => EvidenceKind::CommandRun,
    }
}

fn parse_evidence_kinds(json: &str) -> Vec<EvidenceKind> {
    serde_json::from_str::<Vec<String>>(json)
        .unwrap_or_default()
        .iter()
        .map(|s| parse_evidence_kind(s))
        .collect()
}

fn evidence_status_str(s: &EvidenceStatus) -> &'static str {
    match s {
        EvidenceStatus::Passed => "Passed",
        EvidenceStatus::Failed => "Failed",
        EvidenceStatus::Missing => "Missing",
        EvidenceStatus::Skipped => "Skipped",
        EvidenceStatus::Unknown => "Unknown",
    }
}

fn parse_evidence_status(s: &str) -> EvidenceStatus {
    match s {
        "Passed" => EvidenceStatus::Passed,
        "Failed" => EvidenceStatus::Failed,
        "Missing" => EvidenceStatus::Missing,
        "Skipped" => EvidenceStatus::Skipped,
        _ => EvidenceStatus::Unknown,
    }
}

fn completion_state_str(s: &CompletionState) -> &'static str {
    match s {
        CompletionState::Verified => "Verified",
        CompletionState::Partial => "Partial",
        CompletionState::Attempted => "Attempted",
        CompletionState::Failed => "Failed",
        CompletionState::NotRun => "NotRun",
    }
}

fn parse_completion_state(s: &str) -> CompletionState {
    match s {
        "Verified" => CompletionState::Verified,
        "Partial" => CompletionState::Partial,
        "Attempted" => CompletionState::Attempted,
        "Failed" => CompletionState::Failed,
        _ => CompletionState::NotRun,
    }
}

fn incident_severity_str(s: &IncidentSeverity) -> &'static str {
    match s {
        IncidentSeverity::Low => "Low",
        IncidentSeverity::Medium => "Medium",
        IncidentSeverity::High => "High",
        IncidentSeverity::Critical => "Critical",
    }
}

fn parse_incident_severity(s: &str) -> IncidentSeverity {
    match s {
        "Low" => IncidentSeverity::Low,
        "Medium" => IncidentSeverity::Medium,
        "High" => IncidentSeverity::High,
        "Critical" => IncidentSeverity::Critical,
        _ => IncidentSeverity::Low,
    }
}

fn parse_string_list(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-completion-store-{}.db", uuid::Uuid::new_v4());
        DbInstance::new("sqlite", &path, "").unwrap()
    }

    fn make_claim(run_id: &str, kind: CompletionClaimKind, text: &str) -> CompletionClaim {
        CompletionClaim {
            claim_id: format!("cl-{}", uuid::Uuid::new_v4()),
            run_id: run_id.to_string(),
            agent_key: Some("test-agent".into()),
            model: Some("test-model".into()),
            task_type: "test".into(),
            claim_text: text.to_string(),
            claim_kind: kind,
            required_evidence: vec![EvidenceKind::TestRun],
            linked_evidence_ids: vec![],
            verified: false,
            contradiction_ids: vec![],
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    fn make_evidence(run_id: &str, kind: EvidenceKind, status: EvidenceStatus) -> CompletionEvidence {
        CompletionEvidence {
            evidence_id: format!("ev-{}", uuid::Uuid::new_v4()),
            run_id: run_id.to_string(),
            evidence_kind: kind,
            producer: "test".into(),
            command_or_operation: None,
            status,
            exit_code: Some(0),
            input_hash: None,
            output_hash: None,
            stdout_summary: None,
            stderr_summary: None,
            artifact_ids: vec![],
            provenance_record_id: "prov-1".into(),
            started_at: chrono::Utc::now().to_rfc3339(),
            completed_at: Some(chrono::Utc::now().to_rfc3339()),
        }
    }

    #[test]
    fn test_insert_and_readback_claim() {
        let db = test_db();
        let claim = make_claim("run-1", CompletionClaimKind::TestsPass, "All tests pass.");
        insert_completion_claim(&db, &claim).unwrap();

        let claims = get_completion_claims_by_run(&db, "run-1").unwrap();
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].claim_kind, CompletionClaimKind::TestsPass);
        assert_eq!(claims[0].claim_text, "All tests pass.");
    }

    #[test]
    fn test_insert_and_readback_evidence() {
        let db = test_db();
        let ev = make_evidence("run-1", EvidenceKind::TestRun, EvidenceStatus::Passed);
        insert_completion_evidence(&db, &ev).unwrap();

        let all = get_evidence_by_run(&db, "run-1").unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].evidence_kind, EvidenceKind::TestRun);
        assert_eq!(all[0].status, EvidenceStatus::Passed);
    }

    #[test]
    fn test_completion_report_roundtrip() {
        let db = test_db();
        let report = CompletionReport {
            report_id: "rep-1".into(),
            run_id: "run-1".into(),
            final_state: CompletionState::Verified,
            claims: vec![],
            evidence: vec![],
            failed_gates: vec![],
            unverified_claims: vec![],
            contradictions: vec![],
            calibrated_summary: "All claims verified.".into(),
            provenance_record_id: "prov-1".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        insert_completion_report(&db, &report).unwrap();
        let retrieved = get_completion_report(&db, "rep-1").unwrap().expect("must exist");
        assert_eq!(retrieved.final_state, CompletionState::Verified);
        assert_eq!(retrieved.calibrated_summary, "All claims verified.");
    }

    #[test]
    fn test_incident_persistence() {
        let db = test_db();
        let incident = FalseCompletionIncident {
            incident_id: "inc-1".into(),
            run_id: "run-1".into(),
            agent_key: Some("test-agent".into()),
            model: Some("test-model".into()),
            task_type: "test".into(),
            claimed_state: "All tests pass".into(),
            actual_state: CompletionState::Failed,
            missing_evidence: vec![EvidenceKind::TestRun],
            contradiction_ids: vec![],
            user_correction: Some("Tests actually failed".into()),
            severity: IncidentSeverity::High,
            learning_event_id: "le-1".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        insert_false_completion_incident(&db, &incident).unwrap();
        let all = get_all_incidents(&db).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].actual_state, CompletionState::Failed);
        assert_eq!(all[0].severity, IncidentSeverity::High);
    }
}

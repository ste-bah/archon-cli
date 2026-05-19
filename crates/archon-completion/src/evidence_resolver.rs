//! Evidence resolver — locates completion evidence from Cozo state.
//!
//! Given a list of claims, queries persisted completion evidence plus
//! domain-specific evidence relations to find matching evidence.

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

use super::models::{CompletionClaim, CompletionEvidence, EvidenceKind, EvidenceStatus};
use crate::store;

/// Resolve evidence for each claim by querying Cozo state.
pub fn resolve_evidence(
    db: &DbInstance,
    claims: &[CompletionClaim],
) -> Result<Vec<CompletionEvidence>> {
    let mut evidence = Vec::new();

    for claim in claims {
        match claim.claim_kind {
            super::models::CompletionClaimKind::TestsPass => {
                evidence.extend(find_test_run_evidence(db, &claim.run_id)?);
            }
            super::models::CompletionClaimKind::BuildPasses => {
                evidence.extend(find_build_evidence(db, &claim.run_id)?);
            }
            super::models::CompletionClaimKind::Ingested
            | super::models::CompletionClaimKind::Indexed => {
                evidence.extend(find_ingestion_evidence(db, &claim.run_id)?);
            }
            super::models::CompletionClaimKind::AnswerGrounded => {
                evidence.extend(find_citation_evidence(db, &claim.run_id)?);
            }
            super::models::CompletionClaimKind::Implemented
            | super::models::CompletionClaimKind::Fixed => {
                evidence.extend(find_diff_evidence(db, &claim.run_id)?);
                if claim.claim_kind == super::models::CompletionClaimKind::Fixed {
                    evidence.extend(find_test_run_evidence(db, &claim.run_id)?);
                }
            }
            super::models::CompletionClaimKind::Verified => {
                evidence.extend(find_persisted_evidence(
                    db,
                    &claim.run_id,
                    &[EvidenceKind::ReviewFinding],
                    "no persisted review finding evidence found",
                )?);
            }
            super::models::CompletionClaimKind::Documented => {
                evidence.extend(find_persisted_evidence(
                    db,
                    &claim.run_id,
                    &[EvidenceKind::GeneratedArtifact],
                    "no persisted generated documentation artifact evidence found",
                )?);
            }
            super::models::CompletionClaimKind::Done => {
                evidence.extend(find_gate_evidence(db, &claim.run_id)?);
            }
        }
    }

    Ok(evidence)
}

fn find_test_run_evidence(db: &DbInstance, run_id: &str) -> Result<Vec<CompletionEvidence>> {
    find_persisted_evidence(
        db,
        run_id,
        &[EvidenceKind::TestRun],
        "no persisted test run evidence found",
    )
}

/// Build a Missing-status evidence record for when no evidence is found.
fn missing_evidence_record(run_id: &str, kind: EvidenceKind, summary: &str) -> CompletionEvidence {
    let now = chrono::Utc::now().to_rfc3339();
    CompletionEvidence {
        evidence_id: format!(
            "ev-{}",
            uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
        ),
        run_id: run_id.to_string(),
        evidence_kind: kind,
        producer: "evidence_resolver".into(),
        command_or_operation: None,
        status: EvidenceStatus::Missing,
        exit_code: None,
        input_hash: None,
        output_hash: None,
        stdout_summary: Some(summary.to_string()),
        stderr_summary: None,
        artifact_ids: vec![],
        provenance_record_id: String::new(),
        started_at: now.clone(),
        completed_at: Some(now),
    }
}

fn find_build_evidence(db: &DbInstance, run_id: &str) -> Result<Vec<CompletionEvidence>> {
    find_persisted_evidence(
        db,
        run_id,
        &[EvidenceKind::BuildResult],
        "no persisted build result evidence found",
    )
}

/// Find ingestion job evidence from doc_processing_jobs.
fn find_ingestion_evidence(db: &DbInstance, run_id: &str) -> Result<Vec<CompletionEvidence>> {
    let rows = match query_ingestion_jobs(db, run_id, true)? {
        Some(rows) if !rows.rows.is_empty() => Some(rows),
        Some(empty_rows) => match query_ingestion_jobs(db, run_id, false)? {
            Some(rows) if !rows.rows.is_empty() => Some(rows),
            Some(_) => Some(empty_rows),
            None => None,
        },
        None => query_ingestion_jobs(db, run_id, false)?,
    };

    let Some(rows) = rows else {
        return Ok(vec![missing_evidence_record(
            run_id,
            EvidenceKind::IngestionJob,
            "doc_processing_jobs relation does not exist",
        )]);
    };

    if rows.rows.is_empty() {
        return Ok(vec![missing_evidence_record(
            run_id,
            EvidenceKind::IngestionJob,
            "no ingestion job evidence found",
        )]);
    }

    let now = chrono::Utc::now().to_rfc3339();
    let mut evidence = Vec::new();
    for row in &rows.rows {
        let job_id = row[0].get_str().unwrap_or("").to_string();
        let document_id = row[1].get_str().unwrap_or("").to_string();
        let job_type = row[2].get_str().unwrap_or("unknown").to_string();
        let status_str = row[3].get_str().unwrap_or("unknown");
        let started_at = row[4].get_str().unwrap_or(&now).to_string();
        let completed_at = row[5].get_str().map(|value| value.to_string());
        let status = ingestion_status(status_str);

        evidence.push(CompletionEvidence {
            evidence_id: format!(
                "ev-{}",
                uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
            ),
            run_id: run_id.to_string(),
            evidence_kind: EvidenceKind::IngestionJob,
            producer: "doc_processing_jobs".into(),
            command_or_operation: Some(job_type),
            exit_code: match status {
                EvidenceStatus::Passed => Some(0),
                EvidenceStatus::Failed => Some(1),
                _ => None,
            },
            status,
            input_hash: None,
            output_hash: None,
            stdout_summary: Some(format!("ingestion job {job_id} status: {status_str}")),
            stderr_summary: None,
            artifact_ids: [job_id, document_id]
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect(),
            provenance_record_id: String::new(),
            started_at,
            completed_at,
        });
    }

    Ok(evidence)
}

fn query_ingestion_jobs(
    db: &DbInstance,
    run_id: &str,
    by_job_id: bool,
) -> Result<Option<cozo::NamedRows>> {
    let mut params = std::collections::BTreeMap::new();
    params.insert("rid".into(), DataValue::from(run_id));
    let query = if by_job_id {
        "?[job_id, document_id, job_type, status, started_at, completed_at] \
         := *doc_processing_jobs{job_id: $rid, document_id, job_type, status, \
         started_at, completed_at}"
    } else {
        "?[job_id, document_id, job_type, status, started_at, completed_at] \
         := *doc_processing_jobs{job_id, document_id: $rid, job_type, status, \
         started_at, completed_at}"
    };

    match db.run_script(query, params, ScriptMutability::Immutable) {
        Ok(rows) => Ok(Some(rows)),
        Err(e)
            if e.to_string()
                .contains("Cannot find requested stored relation") =>
        {
            Ok(None)
        }
        Err(e) => Err(anyhow::anyhow!("query doc_processing_jobs failed: {e}")),
    }
}

fn ingestion_status(status: &str) -> EvidenceStatus {
    match status {
        "completed" | "ingested" | "indexed" | "succeeded" | "success" => EvidenceStatus::Passed,
        "failed" | "error" => EvidenceStatus::Failed,
        _ => EvidenceStatus::Unknown,
    }
}

/// Find citation evidence via the shared provenance traversal API.
fn find_citation_evidence(db: &DbInstance, run_id: &str) -> Result<Vec<CompletionEvidence>> {
    let trace = archon_provenance::traverse::trace_artifact(db, run_id)?;
    if trace.edges.is_empty() {
        return Ok(vec![missing_evidence_record(
            run_id,
            EvidenceKind::CitationTrace,
            "no provenance trace found for completion artifact",
        )]);
    }

    let now = chrono::Utc::now().to_rfc3339();
    Ok(vec![CompletionEvidence {
        evidence_id: format!(
            "ev-{}",
            uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
        ),
        run_id: run_id.to_string(),
        evidence_kind: EvidenceKind::CitationTrace,
        producer: "archon-provenance".into(),
        command_or_operation: Some("trace_artifact".into()),
        status: if trace.reaches_source() {
            EvidenceStatus::Passed
        } else {
            EvidenceStatus::Unknown
        },
        exit_code: Some(if trace.reaches_source() { 0 } else { 1 }),
        input_hash: None,
        output_hash: None,
        stdout_summary: Some(format!(
            "provenance trace: {} node(s), {} edge(s), reaches_source={}",
            trace.nodes.len(),
            trace.edges.len(),
            trace.reaches_source()
        )),
        stderr_summary: None,
        artifact_ids: trace
            .nodes
            .iter()
            .map(|node| node.artifact_id.clone())
            .collect(),
        provenance_record_id: run_id.to_string(),
        started_at: now.clone(),
        completed_at: Some(now),
    }])
}

/// Find file diff / generated artifact evidence.
fn find_diff_evidence(db: &DbInstance, run_id: &str) -> Result<Vec<CompletionEvidence>> {
    find_persisted_evidence(
        db,
        run_id,
        &[EvidenceKind::FileDiff, EvidenceKind::GeneratedArtifact],
        "no persisted file diff or generated artifact evidence found",
    )
}

fn find_persisted_evidence(
    db: &DbInstance,
    run_id: &str,
    kinds: &[EvidenceKind],
    missing_summary: &str,
) -> Result<Vec<CompletionEvidence>> {
    let records: Vec<CompletionEvidence> = match store::get_evidence_by_run(db, run_id) {
        Ok(records) => records
            .into_iter()
            .filter(|ev| kinds.contains(&ev.evidence_kind))
            .collect(),
        Err(e)
            if e.to_string()
                .contains("Cannot find requested stored relation") =>
        {
            Vec::new()
        }
        Err(e) => return Err(e),
    };

    if !records.is_empty() {
        return Ok(records);
    }

    Ok(kinds
        .iter()
        .map(|kind| missing_evidence_record(run_id, kind.clone(), missing_summary))
        .collect())
}

/// Find gate result evidence.
fn find_gate_evidence(db: &DbInstance, run_id: &str) -> Result<Vec<CompletionEvidence>> {
    let mut params = std::collections::BTreeMap::new();
    params.insert("rid".into(), DataValue::from(run_id));
    let result = db.run_script(
        "?[gate_id, gate_name, passed, explanation, provenance_record_id, created_at] \
         := *verification_gate_results{gate_id, run_id: $rid, gate_name, passed, \
         explanation, provenance_record_id, created_at}",
        params,
        ScriptMutability::Immutable,
    );

    let rows = match result {
        Ok(rows) => rows,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("Cannot find requested stored relation") {
                return Ok(vec![missing_evidence_record(
                    run_id,
                    EvidenceKind::GateResult,
                    "verification_gate_results relation does not exist",
                )]);
            }
            return Err(anyhow::anyhow!(
                "query verification_gate_results failed: {e}"
            ));
        }
    };

    let mut evidence = Vec::new();
    for row in &rows.rows {
        let gate_id = row[0].get_str().unwrap_or("").to_string();
        let gate_name = row[1].get_str().unwrap_or("unknown-gate").to_string();
        let passed = row[2].get_bool().unwrap_or(false);
        let explanation = row[3].get_str().unwrap_or("").to_string();
        let provenance_record_id = row[4].get_str().unwrap_or("").to_string();
        let created_at = row[5]
            .get_str()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "");
        let now = chrono::Utc::now().to_rfc3339();
        evidence.push(CompletionEvidence {
            evidence_id: format!(
                "ev-{}",
                uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
            ),
            run_id: run_id.to_string(),
            evidence_kind: EvidenceKind::GateResult,
            producer: "verification_gate_results".into(),
            command_or_operation: Some(gate_name),
            status: if passed {
                EvidenceStatus::Passed
            } else {
                EvidenceStatus::Failed
            },
            exit_code: Some(if passed { 0 } else { 1 }),
            input_hash: None,
            output_hash: None,
            stdout_summary: Some(explanation),
            stderr_summary: None,
            artifact_ids: vec![gate_id],
            provenance_record_id,
            started_at: if created_at.is_empty() {
                now.clone()
            } else {
                created_at.to_string()
            },
            completed_at: Some(now),
        });
    }

    if evidence.is_empty() {
        evidence.push(missing_evidence_record(
            run_id,
            EvidenceKind::GateResult,
            "no persisted verification gate results found",
        ));
    }

    Ok(evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CompletionClaimKind, CompletionState, VerificationGateResult};
    use cozo::DbInstance;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-evidence-resolver-{}.db", uuid::Uuid::new_v4());
        DbInstance::new("sqlite", &path, "").unwrap()
    }

    #[test]
    fn test_resolve_returns_missing_when_no_evidence() {
        let db = test_db();
        let claims = vec![CompletionClaim {
            claim_id: "cl-1".into(),
            run_id: "run-x".into(),
            agent_key: None,
            model: None,
            task_type: "test".into(),
            claim_text: "tests pass".into(),
            claim_kind: CompletionClaimKind::TestsPass,
            required_evidence: vec![EvidenceKind::TestRun],
            linked_evidence_ids: vec![],
            verified: false,
            contradiction_ids: vec![],
            created_at: chrono::Utc::now().to_rfc3339(),
        }];

        let evidence = resolve_evidence(&db, &claims).unwrap();
        assert!(!evidence.is_empty());
        assert_eq!(evidence[0].status, EvidenceStatus::Missing);
    }

    #[test]
    fn test_resolve_finds_test_run_evidence() {
        let db = test_db();
        crate::schema::ensure_completion_schema(&db).unwrap();
        let run_id = "run-1";
        store::insert_completion_evidence(
            &db,
            &CompletionEvidence {
                evidence_id: "ev-test-1".into(),
                run_id: run_id.into(),
                evidence_kind: EvidenceKind::TestRun,
                producer: "cargo-test".into(),
                command_or_operation: Some("cargo test".into()),
                status: EvidenceStatus::Passed,
                exit_code: Some(0),
                input_hash: None,
                output_hash: None,
                stdout_summary: Some("test result: ok".into()),
                stderr_summary: None,
                artifact_ids: vec![],
                provenance_record_id: "prov-test-1".into(),
                started_at: "2026-05-04T00:00:00Z".into(),
                completed_at: Some("2026-05-04T00:00:01Z".into()),
            },
        )
        .unwrap();

        let claims = vec![CompletionClaim {
            claim_id: "cl-1".into(),
            run_id: run_id.to_string(),
            agent_key: None,
            model: None,
            task_type: "test".into(),
            claim_text: "tests pass".into(),
            claim_kind: CompletionClaimKind::TestsPass,
            required_evidence: vec![EvidenceKind::TestRun],
            linked_evidence_ids: vec![],
            verified: false,
            contradiction_ids: vec![],
            created_at: chrono::Utc::now().to_rfc3339(),
        }];

        let evidence = resolve_evidence(&db, &claims).unwrap();
        assert!(!evidence.is_empty());
        assert_eq!(evidence[0].evidence_kind, EvidenceKind::TestRun);
        assert_eq!(evidence[0].status, EvidenceStatus::Passed);
    }

    #[test]
    fn test_resolve_ignores_gt_runs_for_test_evidence() {
        let db = test_db();
        let run_id = "run-gt-only";
        let _ = db.run_script(
            ":create gt_runs { run_id: String => situation: String, started_at: String, completed_at: String, status: String }",
            Default::default(),
            ScriptMutability::Mutable,
        );
        let _ = db.run_script(
            &format!("?[run_id, situation, started_at, completed_at, status] <- [[\"{run_id}\", \"test\", \"2026-01-01T00:00:00Z\", \"2026-01-01T00:01:00Z\", \"completed\"]] :put gt_runs {{ run_id => situation, started_at, completed_at, status }}"),
            Default::default(),
            ScriptMutability::Mutable,
        );

        let claims = vec![CompletionClaim {
            claim_id: "cl-gt-only".into(),
            run_id: run_id.to_string(),
            agent_key: None,
            model: None,
            task_type: "test".into(),
            claim_text: "tests pass".into(),
            claim_kind: CompletionClaimKind::TestsPass,
            required_evidence: vec![EvidenceKind::TestRun],
            linked_evidence_ids: vec![],
            verified: false,
            contradiction_ids: vec![],
            created_at: chrono::Utc::now().to_rfc3339(),
        }];

        let evidence = resolve_evidence(&db, &claims).unwrap();
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].evidence_kind, EvidenceKind::TestRun);
        assert_eq!(evidence[0].status, EvidenceStatus::Missing);
    }

    #[test]
    fn test_resolve_finds_persisted_file_diff_evidence() {
        let db = test_db();
        crate::schema::ensure_completion_schema(&db).unwrap();
        let run_id = "run-diff-1";
        store::insert_completion_evidence(
            &db,
            &CompletionEvidence {
                evidence_id: "ev-diff-1".into(),
                run_id: run_id.into(),
                evidence_kind: EvidenceKind::FileDiff,
                producer: "full-state-verification".into(),
                command_or_operation: Some("git diff --stat".into()),
                status: EvidenceStatus::Passed,
                exit_code: Some(0),
                input_hash: Some("before".into()),
                output_hash: Some("after".into()),
                stdout_summary: Some("src/lib.rs changed".into()),
                stderr_summary: None,
                artifact_ids: vec!["src/lib.rs".into()],
                provenance_record_id: "prov-diff-1".into(),
                started_at: "2026-05-04T00:00:00Z".into(),
                completed_at: Some("2026-05-04T00:00:01Z".into()),
            },
        )
        .unwrap();

        let claims = vec![CompletionClaim {
            claim_id: "cl-diff-1".into(),
            run_id: run_id.to_string(),
            agent_key: None,
            model: None,
            task_type: "coding".into(),
            claim_text: "implemented the feature".into(),
            claim_kind: CompletionClaimKind::Implemented,
            required_evidence: vec![EvidenceKind::FileDiff],
            linked_evidence_ids: vec![],
            verified: false,
            contradiction_ids: vec![],
            created_at: chrono::Utc::now().to_rfc3339(),
        }];

        let evidence = resolve_evidence(&db, &claims).unwrap();
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].evidence_id, "ev-diff-1");
        assert_eq!(evidence[0].evidence_kind, EvidenceKind::FileDiff);
        assert_eq!(evidence[0].status, EvidenceStatus::Passed);
    }

    #[test]
    fn test_resolve_finds_persisted_gate_evidence_for_done_claim() {
        let db = test_db();
        crate::schema::ensure_completion_schema(&db).unwrap();
        let run_id = "run-gate-1";
        store::insert_gate_result(
            &db,
            &VerificationGateResult {
                gate_id: "gate-1".into(),
                gate_name: "TestsPassGate".into(),
                passed: true,
                resulting_state: CompletionState::Verified,
                blocked_claims: vec![],
                required_missing_evidence: vec![],
                explanation: "all required evidence present".into(),
                provenance_record_id: "prov-gate-1".into(),
            },
            run_id,
        )
        .unwrap();

        let claims = vec![CompletionClaim {
            claim_id: "cl-gate-1".into(),
            run_id: run_id.to_string(),
            agent_key: None,
            model: None,
            task_type: "coding".into(),
            claim_text: "done".into(),
            claim_kind: CompletionClaimKind::Done,
            required_evidence: vec![EvidenceKind::GateResult],
            linked_evidence_ids: vec![],
            verified: false,
            contradiction_ids: vec![],
            created_at: chrono::Utc::now().to_rfc3339(),
        }];

        let evidence = resolve_evidence(&db, &claims).unwrap();
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].evidence_kind, EvidenceKind::GateResult);
        assert_eq!(evidence[0].producer, "verification_gate_results");
        assert_eq!(
            evidence[0].command_or_operation.as_deref(),
            Some("TestsPassGate")
        );
        assert_eq!(evidence[0].status, EvidenceStatus::Passed);
        assert_eq!(evidence[0].artifact_ids, vec!["gate-1"]);
    }
}

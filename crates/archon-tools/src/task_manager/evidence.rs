use std::collections::BTreeMap;

use archon_completion::{
    CompletionEvidence, EvidenceKind, EvidenceStatus, RequiredEvidence, RequiredEvidenceKind,
    RequiredEvidenceStatus,
};
use cozo::{DataValue, ScriptMutability};

use super::TaskTransitionError;

pub(crate) fn resolve_required_evidence(
    db: &cozo::DbInstance,
    run_id: &str,
    evidence_ids: &[String],
    required: &[RequiredEvidenceKind],
) -> Result<Vec<RequiredEvidence>, TaskTransitionError> {
    if evidence_ids.is_empty() {
        return Ok(Vec::new());
    }
    archon_completion::schema::ensure_completion_schema(db)
        .map_err(|error| TaskTransitionError::EvidenceResolution(error.to_string()))?;
    let evidence = archon_completion::store::get_evidence_by_run(db, run_id)
        .map_err(|error| TaskTransitionError::EvidenceResolution(error.to_string()))?;
    if evidence_ids.len() != required.len() {
        return Err(TaskTransitionError::UntrustedEvidence(
            "completion evidence does not cover every required kind".to_string(),
        ));
    }
    let mut resolved_kinds = Vec::new();
    let mut resolved = Vec::with_capacity(evidence_ids.len());
    for (sequence, evidence_id) in evidence_ids.iter().enumerate() {
        let evidence = evidence
            .iter()
            .find(|candidate| candidate.evidence_id == *evidence_id)
            .ok_or_else(|| TaskTransitionError::UntrustedEvidence(evidence_id.clone()))?;
        let kind = required
            .iter()
            .copied()
            .find(|required_kind| {
                evidence_kind_matches_requirement(*required_kind, evidence.evidence_kind)
            })
            .ok_or_else(|| TaskTransitionError::UntrustedEvidence(evidence_id.clone()))?;
        let required_evidence = if evidence.status == EvidenceStatus::Failed {
            RequiredEvidence {
                kind,
                status: RequiredEvidenceStatus::Failed,
                sequence: sequence as u64 + 1,
                evidence_id: Some(evidence.evidence_id.clone()),
                run_id: Some(evidence.run_id.clone()),
            }
        } else {
            trusted_required_evidence(db, evidence, required, sequence as u64 + 1)
                .ok_or_else(|| TaskTransitionError::UntrustedEvidence(evidence_id.clone()))?
        };
        if resolved_kinds.contains(&required_evidence.kind) {
            return Err(TaskTransitionError::UntrustedEvidence(format!(
                "duplicate completion evidence kind: {:?}",
                required_evidence.kind
            )));
        }
        resolved_kinds.push(required_evidence.kind);
        resolved.push(required_evidence);
    }
    if required.len() == resolved_kinds.len()
        && required.iter().all(|kind| resolved_kinds.contains(kind))
    {
        Ok(resolved)
    } else {
        Err(TaskTransitionError::UntrustedEvidence(
            "completion evidence kinds do not match task requirements".to_string(),
        ))
    }
}

fn trusted_required_evidence(
    db: &cozo::DbInstance,
    evidence: &CompletionEvidence,
    required: &[RequiredEvidenceKind],
    sequence: u64,
) -> Option<RequiredEvidence> {
    let kind = required.iter().copied().find(|required_kind| {
        evidence_kind_matches_requirement(*required_kind, evidence.evidence_kind)
    })?;
    if evidence.status == EvidenceStatus::Failed {
        return Some(RequiredEvidence {
            kind,
            status: RequiredEvidenceStatus::Failed,
            sequence,
            evidence_id: Some(evidence.evidence_id.clone()),
            run_id: Some(evidence.run_id.clone()),
        });
    }
    let status =
        if evidence.status == EvidenceStatus::Passed && independently_verified(db, evidence) {
            RequiredEvidenceStatus::Passed
        } else {
            RequiredEvidenceStatus::Missing
        };
    Some(RequiredEvidence {
        kind,
        status,
        sequence,
        evidence_id: Some(evidence.evidence_id.clone()),
        run_id: Some(evidence.run_id.clone()),
    })
}

fn evidence_kind_matches_requirement(
    required: RequiredEvidenceKind,
    evidence: EvidenceKind,
) -> bool {
    matches!(
        (required, evidence),
        (RequiredEvidenceKind::Tests, EvidenceKind::TestRun)
            | (RequiredEvidenceKind::Build, EvidenceKind::BuildResult)
            | (RequiredEvidenceKind::Lint, EvidenceKind::CommandRun)
            | (RequiredEvidenceKind::Typecheck, EvidenceKind::CommandRun)
            | (
                RequiredEvidenceKind::Verifier,
                EvidenceKind::GateResult | EvidenceKind::ReviewFinding
            )
            | (
                RequiredEvidenceKind::PlanReview,
                EvidenceKind::ReviewFinding
            )
            | (
                RequiredEvidenceKind::SourceEvidence,
                EvidenceKind::FileDiff | EvidenceKind::GeneratedArtifact
            )
            | (
                RequiredEvidenceKind::ManualOutcome,
                EvidenceKind::CommandRun
            )
            | (
                RequiredEvidenceKind::HumanApproval,
                EvidenceKind::GateResult
            )
    )
}

fn independently_verified(db: &cozo::DbInstance, evidence: &CompletionEvidence) -> bool {
    let persisted = persisted_evidence_matches(db, evidence);
    if !persisted {
        return false;
    }
    match evidence.evidence_kind {
        EvidenceKind::GateResult | EvidenceKind::ReviewFinding => {
            matching_gate_result_exists(db, evidence)
        }
        _ => matching_provenance_gate_exists(db, evidence),
    }
}

fn matching_provenance_gate_exists(db: &cozo::DbInstance, evidence: &CompletionEvidence) -> bool {
    let mut params = BTreeMap::new();
    params.insert("rid".into(), DataValue::from(evidence.run_id.as_str()));
    params.insert(
        "provenance".into(),
        DataValue::from(evidence.provenance_record_id.as_str()),
    );

    db.run_script(
        "?[gate_id] := *verification_gate_results{gate_id, run_id: $rid, passed: true, \
         provenance_record_id: $provenance}",
        params,
        ScriptMutability::Immutable,
    )
    .is_ok_and(|rows| !rows.rows.is_empty())
}

fn persisted_evidence_matches(db: &cozo::DbInstance, evidence: &CompletionEvidence) -> bool {
    if evidence.provenance_record_id.trim().is_empty() {
        return false;
    }
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(evidence.evidence_id.as_str()));
    params.insert("rid".into(), DataValue::from(evidence.run_id.as_str()));
    params.insert(
        "kind".into(),
        DataValue::from(evidence_kind_name(evidence.evidence_kind)),
    );
    params.insert(
        "producer".into(),
        DataValue::from(evidence.producer.as_str()),
    );
    params.insert(
        "command".into(),
        DataValue::from(evidence.command_or_operation.as_deref().unwrap_or_default()),
    );
    params.insert(
        "provenance".into(),
        DataValue::from(evidence.provenance_record_id.as_str()),
    );

    db.run_script(
        "?[evidence_id] := *completion_evidence{evidence_id, run_id: $rid, evidence_kind: $kind, \
         producer: $producer, command_or_operation: $command, status: \"Passed\", exit_code: 0, \
         provenance_record_id: $provenance, completed_at: _}, evidence_id = $eid",
        params,
        ScriptMutability::Immutable,
    )
    .is_ok_and(|rows| !rows.rows.is_empty())
}

fn matching_gate_result_exists(db: &cozo::DbInstance, evidence: &CompletionEvidence) -> bool {
    let gate_id = match evidence.artifact_ids.as_slice() {
        [gate_id] if !gate_id.trim().is_empty() => gate_id,
        _ => return false,
    };
    let mut params = BTreeMap::new();
    params.insert("gid".into(), DataValue::from(gate_id.as_str()));
    params.insert("rid".into(), DataValue::from(evidence.run_id.as_str()));
    params.insert(
        "name".into(),
        DataValue::from(evidence.command_or_operation.as_deref().unwrap_or_default()),
    );
    params.insert(
        "provenance".into(),
        DataValue::from(evidence.provenance_record_id.as_str()),
    );

    db.run_script(
        "?[gate_id] := *verification_gate_results{gate_id, run_id: $rid, gate_name: $name, \
         passed: true, provenance_record_id: $provenance}, gate_id = $gid",
        params,
        ScriptMutability::Immutable,
    )
    .is_ok_and(|rows| !rows.rows.is_empty())
}
fn evidence_kind_name(kind: EvidenceKind) -> &'static str {
    match kind {
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

#[cfg(test)]
mod tests {
    use super::*;
    use archon_completion::models::{CompletionState, VerificationGateResult};
    use archon_completion::store::{insert_completion_evidence, insert_gate_result};

    fn evidence(run_id: &str, evidence_id: &str) -> CompletionEvidence {
        CompletionEvidence {
            evidence_id: evidence_id.into(),
            run_id: run_id.into(),
            evidence_kind: EvidenceKind::TestRun,
            producer: "cargo-test".into(),
            command_or_operation: Some("cargo test -p archon-tools".into()),
            status: EvidenceStatus::Passed,
            exit_code: Some(0),
            input_hash: Some("input".into()),
            output_hash: Some("output".into()),
            stdout_summary: Some("passed".into()),
            stderr_summary: None,
            artifact_ids: vec![],
            provenance_record_id: "provenance-1".into(),
            started_at: "2026-08-14T00:00:00Z".into(),
            completed_at: Some("2026-08-14T00:00:01Z".into()),
        }
    }

    #[test]
    fn accepts_genuine_persisted_evidence_from_its_run() {
        let db = cozo::DbInstance::new("mem", "", "").unwrap();
        let evidence = evidence("run-1", "evidence-1");
        insert_gate_result(
            &db,
            &VerificationGateResult {
                gate_id: "gate-1".into(),
                gate_name: "test-evidence".into(),
                passed: true,
                resulting_state: CompletionState::Verified,
                blocked_claims: vec![],
                required_missing_evidence: vec![],
                explanation: "test evidence verified".into(),
                provenance_record_id: evidence.provenance_record_id.clone(),
            },
            "run-1",
        )
        .unwrap();
        insert_completion_evidence(&db, &evidence).unwrap();

        let resolved = resolve_required_evidence(
            &db,
            "run-1",
            &["evidence-1".into()],
            &[RequiredEvidenceKind::Tests],
        )
        .unwrap();

        assert_eq!(resolved[0].status, RequiredEvidenceStatus::Passed);
    }

    #[test]
    fn rejects_wrong_run_and_wrong_kind_for_durable_evidence() {
        let db = cozo::DbInstance::new("mem", "", "").unwrap();
        let evidence = evidence("run-1", "evidence-1");
        insert_gate_result(
            &db,
            &VerificationGateResult {
                gate_id: "gate-1".into(),
                gate_name: "test-evidence".into(),
                passed: true,
                resulting_state: CompletionState::Verified,
                blocked_claims: vec![],
                required_missing_evidence: vec![],
                explanation: "test evidence verified".into(),
                provenance_record_id: evidence.provenance_record_id.clone(),
            },
            "run-1",
        )
        .unwrap();
        insert_completion_evidence(&db, &evidence).unwrap();

        assert!(matches!(
            resolve_required_evidence(
                &db,
                "other-run",
                &["evidence-1".into()],
                &[RequiredEvidenceKind::Tests],
            ),
            Err(TaskTransitionError::UntrustedEvidence(_))
        ));
        assert!(matches!(
            resolve_required_evidence(
                &db,
                "run-1",
                &["evidence-1".into()],
                &[RequiredEvidenceKind::Build],
            ),
            Err(TaskTransitionError::UntrustedEvidence(_))
        ));
    }

    #[test]
    fn rejects_failed_durable_evidence() {
        let db = cozo::DbInstance::new("mem", "", "").unwrap();
        let mut evidence = evidence("run-1", "evidence-1");
        evidence.status = EvidenceStatus::Failed;
        evidence.exit_code = Some(1);
        insert_completion_evidence(&db, &evidence).unwrap();

        let result = resolve_required_evidence(
            &db,
            "run-1",
            &["evidence-1".into()],
            &[RequiredEvidenceKind::Tests],
        )
        .unwrap();

        assert_eq!(result[0].status, RequiredEvidenceStatus::Failed);
    }

    #[test]
    fn accepts_gate_evidence_only_when_gate_record_matches() {
        let db = cozo::DbInstance::new("mem", "", "").unwrap();
        let gate = VerificationGateResult {
            gate_id: "gate-1".into(),
            gate_name: "plan-review".into(),
            passed: true,
            resulting_state: CompletionState::Verified,
            blocked_claims: vec![],
            required_missing_evidence: vec![],
            explanation: "approved".into(),
            provenance_record_id: "gate-provenance".into(),
        };
        insert_gate_result(&db, &gate, "run-1").unwrap();
        let evidence = CompletionEvidence {
            evidence_id: "evidence-1".into(),
            run_id: "run-1".into(),
            evidence_kind: EvidenceKind::GateResult,
            producer: "verification_gate_results".into(),
            command_or_operation: Some("plan-review".into()),
            status: EvidenceStatus::Passed,
            exit_code: Some(0),
            input_hash: None,
            output_hash: None,
            stdout_summary: Some("approved".into()),
            stderr_summary: None,
            artifact_ids: vec!["gate-1".into()],
            provenance_record_id: "gate-provenance".into(),
            started_at: "2026-08-14T00:00:00Z".into(),
            completed_at: Some("2026-08-14T00:00:01Z".into()),
        };
        insert_completion_evidence(&db, &evidence).unwrap();

        let resolved = resolve_required_evidence(
            &db,
            "run-1",
            &["evidence-1".into()],
            &[RequiredEvidenceKind::Verifier],
        )
        .unwrap();

        assert_eq!(resolved[0].status, RequiredEvidenceStatus::Passed);
    }
}

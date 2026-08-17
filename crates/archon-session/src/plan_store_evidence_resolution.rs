use std::collections::BTreeMap;

use archon_completion::{
    CompletionEvidence, EvidenceKind, EvidenceStatus, RequiredEvidence, RequiredEvidenceKind,
    RequiredEvidenceStatus,
};
use cozo::{DataValue, ScriptMutability};

use super::plan_store_evidence::untrusted;
use super::{PlanApprovalAuthority, PlanStore};

impl PlanStore {
    /// Resolve evidence identities against durable records and signed test receipts.
    pub fn resolve_required_evidence(
        &self,
        authority: &PlanApprovalAuthority,
        session_id: &str,
        run_id: &str,
        evidence_ids: &[String],
        required: &[RequiredEvidenceKind],
    ) -> Result<Vec<RequiredEvidence>, std::io::Error> {
        if evidence_ids.is_empty() {
            return Ok(Vec::new());
        }
        archon_completion::schema::ensure_completion_schema(&self.db).map_err(super::db_err)?;
        let evidence = archon_completion::store::get_evidence_by_run(&self.db, run_id)
            .map_err(super::db_err)?;
        if evidence_ids.len() != required.len() {
            return Err(untrusted(
                "completion evidence does not cover every required kind",
            ));
        }
        let mut kinds = Vec::new();
        let mut resolved = Vec::with_capacity(evidence_ids.len());
        for (sequence, evidence_id) in evidence_ids.iter().enumerate() {
            let evidence = evidence
                .iter()
                .find(|candidate| candidate.evidence_id == *evidence_id)
                .ok_or_else(|| untrusted(evidence_id))?;
            let kind = required
                .iter()
                .copied()
                .find(|required| kind_matches(*required, evidence.evidence_kind))
                .ok_or_else(|| untrusted(evidence_id))?;
            let status = if evidence.status == EvidenceStatus::Failed {
                RequiredEvidenceStatus::Failed
            } else if evidence.status == EvidenceStatus::Passed
                && self.verified(authority, session_id, evidence)
            {
                RequiredEvidenceStatus::Passed
            } else {
                RequiredEvidenceStatus::Missing
            };
            if kinds.contains(&kind) {
                return Err(untrusted("duplicate completion evidence kind"));
            }
            kinds.push(kind);
            resolved.push(RequiredEvidence {
                kind,
                status,
                sequence: sequence as u64 + 1,
                evidence_id: Some(evidence.evidence_id.clone()),
                run_id: Some(evidence.run_id.clone()),
            });
        }
        if required.len() == kinds.len() && required.iter().all(|kind| kinds.contains(kind)) {
            Ok(resolved)
        } else {
            Err(untrusted(
                "completion evidence kinds do not match task requirements",
            ))
        }
    }

    fn verified(
        &self,
        authority: &PlanApprovalAuthority,
        session_id: &str,
        evidence: &CompletionEvidence,
    ) -> bool {
        if evidence.provenance_record_id.trim().is_empty() || !persisted_matches(&self.db, evidence)
        {
            return false;
        }
        if evidence.evidence_kind != EvidenceKind::TestRun {
            return false;
        }
        self.verify_test_command_evidence(
            authority,
            session_id,
            &evidence.run_id,
            &evidence.evidence_id,
        )
        .is_ok()
    }
}

fn persisted_matches(db: &cozo::DbInstance, evidence: &CompletionEvidence) -> bool {
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(evidence.evidence_id.as_str()));
    params.insert("rid".into(), DataValue::from(evidence.run_id.as_str()));
    params.insert(
        "kind".into(),
        DataValue::from(kind_name(evidence.evidence_kind)),
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
        "?[evidence_id] := *completion_evidence{evidence_id, run_id: $rid, evidence_kind: $kind, producer: $producer, command_or_operation: $command, status: 'Passed', exit_code: 0, provenance_record_id: $provenance, completed_at: _}, evidence_id = $eid",
        params,
        ScriptMutability::Immutable,
    )
    .is_ok_and(|rows| !rows.rows.is_empty())
}

fn kind_matches(required: RequiredEvidenceKind, evidence: EvidenceKind) -> bool {
    matches!(
        (required, evidence),
        (RequiredEvidenceKind::Tests, EvidenceKind::TestRun)
            | (RequiredEvidenceKind::Build, EvidenceKind::BuildResult)
            | (
                RequiredEvidenceKind::Lint | RequiredEvidenceKind::Typecheck,
                EvidenceKind::CommandRun
            )
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

fn kind_name(kind: EvidenceKind) -> &'static str {
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

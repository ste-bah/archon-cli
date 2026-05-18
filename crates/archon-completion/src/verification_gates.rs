//! Concrete verification gate implementations per TSPEC §10.5.
//!
//! Each gate evaluates whether a specific claim kind is supported by evidence.
//! Gates are small, testable, and isolated.

use crate::errors::EvidenceEngineError;
use crate::models::*;
use async_trait::async_trait;

type ClaimKindSet = &'static [CompletionClaimKind];
type EvidenceKindSet = &'static [EvidenceKind];

/// Trait for verification gates per TSPEC §10.5.
#[async_trait]
pub trait VerificationGate: Send + Sync {
    async fn evaluate(
        &self,
        request: &VerificationGateRequest,
    ) -> Result<VerificationGateResult, EvidenceEngineError>;

    fn name(&self) -> &'static str;
}

fn claim_kind_label(kind: &CompletionClaimKind) -> &'static str {
    match kind {
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

fn evidence_kind_label(kind: &EvidenceKind) -> &'static str {
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

pub fn gate_name_for_claim_kind(kind: &CompletionClaimKind) -> &'static str {
    match kind {
        CompletionClaimKind::TestsPass => "TestsPassGate",
        CompletionClaimKind::BuildPasses => "BuildPassesGate",
        CompletionClaimKind::Implemented => "ImplementedGate",
        CompletionClaimKind::Fixed => "FixedGate",
        CompletionClaimKind::Ingested | CompletionClaimKind::Indexed => "IngestedGate",
        CompletionClaimKind::AnswerGrounded => "AnswerGroundedGate",
        CompletionClaimKind::Verified => "VerifiedGate",
        CompletionClaimKind::Documented => "DocumentedGate",
        CompletionClaimKind::Done => "DoneGate",
    }
}

pub fn claim_has_passed_gate(
    claim: &CompletionClaim,
    gate_results: &[VerificationGateResult],
) -> bool {
    let gate_name = gate_name_for_claim_kind(&claim.claim_kind);
    gate_results
        .iter()
        .any(|gate| gate.gate_name == gate_name && gate.passed)
}

struct EvidenceBackedGate {
    name: &'static str,
    claim_kinds: ClaimKindSet,
    required_evidence: EvidenceKindSet,
}

#[async_trait]
impl VerificationGate for EvidenceBackedGate {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn evaluate(
        &self,
        request: &VerificationGateRequest,
    ) -> Result<VerificationGateResult, EvidenceEngineError> {
        let claims: Vec<&CompletionClaim> = request
            .claims
            .iter()
            .filter(|claim| self.claim_kinds.contains(&claim.claim_kind))
            .collect();

        let gate_id = format!("gate-{}", uuid::Uuid::new_v4());
        if claims.is_empty() {
            return Ok(VerificationGateResult {
                gate_id,
                gate_name: self.name().into(),
                passed: true,
                resulting_state: CompletionState::NotRun,
                blocked_claims: vec![],
                required_missing_evidence: vec![],
                explanation: format!("no {} claims to evaluate", self.name()),
                provenance_record_id: String::new(),
            });
        }

        let mut missing = Vec::new();
        let mut failed = Vec::new();
        let mut non_passed = Vec::new();

        for required_kind in self.required_evidence {
            let matching: Vec<&CompletionEvidence> = request
                .available_evidence
                .iter()
                .filter(|evidence| evidence.evidence_kind == *required_kind)
                .collect();

            if matching.is_empty()
                || matching
                    .iter()
                    .all(|evidence| evidence.status == EvidenceStatus::Missing)
            {
                missing.push(required_kind.clone());
                continue;
            }

            if matching
                .iter()
                .any(|evidence| evidence.status == EvidenceStatus::Failed)
            {
                failed.push(required_kind.clone());
                continue;
            }

            if !matching
                .iter()
                .any(|evidence| evidence.status == EvidenceStatus::Passed)
            {
                non_passed.push(required_kind.clone());
            }
        }

        let blocked_claims: Vec<String> =
            claims.iter().map(|claim| claim.claim_id.clone()).collect();
        if !failed.is_empty() {
            return Ok(VerificationGateResult {
                gate_id,
                gate_name: self.name().into(),
                passed: false,
                resulting_state: CompletionState::Failed,
                blocked_claims,
                required_missing_evidence: vec![],
                explanation: format!(
                    "{} evidence failed for {}",
                    failed
                        .iter()
                        .map(evidence_kind_label)
                        .collect::<Vec<_>>()
                        .join(", "),
                    self.name()
                ),
                provenance_record_id: String::new(),
            });
        }

        if !missing.is_empty() || !non_passed.is_empty() {
            let mut required_missing_evidence = missing.clone();
            required_missing_evidence.extend(non_passed.clone());
            let missing_labels = required_missing_evidence
                .iter()
                .map(evidence_kind_label)
                .collect::<Vec<_>>()
                .join(", ");
            return Ok(VerificationGateResult {
                gate_id,
                gate_name: self.name().into(),
                passed: false,
                resulting_state: CompletionState::NotRun,
                blocked_claims,
                required_missing_evidence,
                explanation: format!(
                    "missing passed evidence for {}: {missing_labels}",
                    self.name()
                ),
                provenance_record_id: String::new(),
            });
        }

        let claim_labels = claims
            .iter()
            .map(|claim| claim_kind_label(&claim.claim_kind))
            .collect::<Vec<_>>()
            .join("/");
        let evidence_labels = self
            .required_evidence
            .iter()
            .map(evidence_kind_label)
            .collect::<Vec<_>>()
            .join(", ");
        Ok(VerificationGateResult {
            gate_id,
            gate_name: self.name().into(),
            passed: true,
            resulting_state: CompletionState::Verified,
            blocked_claims: vec![],
            required_missing_evidence: vec![],
            explanation: format!(
                "{} {} claims verified by passed evidence: {evidence_labels}",
                claims.len(),
                claim_labels,
            ),
            provenance_record_id: String::new(),
        })
    }
}

// ── TestsPassGate ────────────────────────────────────────────────────────────

pub struct TestsPassGate;

#[async_trait]
impl VerificationGate for TestsPassGate {
    fn name(&self) -> &'static str {
        "TestsPassGate"
    }

    async fn evaluate(
        &self,
        request: &VerificationGateRequest,
    ) -> Result<VerificationGateResult, EvidenceEngineError> {
        let test_claims: Vec<&CompletionClaim> = request
            .claims
            .iter()
            .filter(|c| c.claim_kind == CompletionClaimKind::TestsPass)
            .collect();

        let test_evidence: Vec<&CompletionEvidence> = request
            .available_evidence
            .iter()
            .filter(|e| e.evidence_kind == EvidenceKind::TestRun)
            .collect();

        let gate_id = format!("gate-{}", uuid::Uuid::new_v4());

        if test_claims.is_empty() {
            return Ok(VerificationGateResult {
                gate_id,
                gate_name: self.name().into(),
                passed: true,
                resulting_state: CompletionState::NotRun,
                blocked_claims: vec![],
                required_missing_evidence: vec![],
                explanation: "no TestsPass claims to evaluate".into(),
                provenance_record_id: String::new(),
            });
        }

        if test_evidence.is_empty() {
            return Ok(VerificationGateResult {
                gate_id,
                gate_name: self.name().into(),
                passed: false,
                resulting_state: CompletionState::NotRun,
                blocked_claims: test_claims.iter().map(|c| c.claim_id.clone()).collect(),
                required_missing_evidence: vec![EvidenceKind::TestRun],
                explanation: "no TestRun evidence found for TestsPass claims".into(),
                provenance_record_id: String::new(),
            });
        }

        // Check if any evidence has Failed status
        let has_failed = test_evidence
            .iter()
            .any(|e| e.status == EvidenceStatus::Failed);
        if has_failed {
            return Ok(VerificationGateResult {
                gate_id,
                gate_name: self.name().into(),
                passed: false,
                resulting_state: CompletionState::Failed,
                blocked_claims: test_claims.iter().map(|c| c.claim_id.clone()).collect(),
                required_missing_evidence: vec![],
                explanation: "TestRun evidence has Failed status — tests did not pass".into(),
                provenance_record_id: String::new(),
            });
        }

        // Check if any evidence is Missing (not yet collected)
        let has_missing = test_evidence
            .iter()
            .any(|e| e.status == EvidenceStatus::Missing);
        if has_missing {
            return Ok(VerificationGateResult {
                gate_id,
                gate_name: self.name().into(),
                passed: false,
                resulting_state: CompletionState::NotRun,
                blocked_claims: test_claims.iter().map(|c| c.claim_id.clone()).collect(),
                required_missing_evidence: vec![EvidenceKind::TestRun],
                explanation: "TestRun evidence is missing — no test results available".into(),
                provenance_record_id: String::new(),
            });
        }

        // Check that all evidence is Passed (not Skipped, Unknown, etc.)
        let all_passed = test_evidence
            .iter()
            .all(|e| e.status == EvidenceStatus::Passed);
        if !all_passed {
            return Ok(VerificationGateResult {
                gate_id,
                gate_name: self.name().into(),
                passed: false,
                resulting_state: CompletionState::NotRun,
                blocked_claims: test_claims.iter().map(|c| c.claim_id.clone()).collect(),
                required_missing_evidence: vec![EvidenceKind::TestRun],
                explanation: "TestRun evidence has non-Passed status".into(),
                provenance_record_id: String::new(),
            });
        }

        // Passed — all TestRun evidence has Passed status
        Ok(VerificationGateResult {
            gate_id,
            gate_name: self.name().into(),
            passed: true,
            resulting_state: CompletionState::Verified,
            blocked_claims: vec![],
            required_missing_evidence: vec![],
            explanation: format!(
                "{} TestsPass claims verified by {} TestRun evidence records",
                test_claims.len(),
                test_evidence.len()
            ),
            provenance_record_id: String::new(),
        })
    }
}

// ── IngestedGate ─────────────────────────────────────────────────────────────

pub struct IngestedGate;

#[async_trait]
impl VerificationGate for IngestedGate {
    fn name(&self) -> &'static str {
        "IngestedGate"
    }

    async fn evaluate(
        &self,
        request: &VerificationGateRequest,
    ) -> Result<VerificationGateResult, EvidenceEngineError> {
        let ingest_claims: Vec<&CompletionClaim> = request
            .claims
            .iter()
            .filter(|c| {
                c.claim_kind == CompletionClaimKind::Ingested
                    || c.claim_kind == CompletionClaimKind::Indexed
            })
            .collect();

        let ingest_evidence: Vec<&CompletionEvidence> = request
            .available_evidence
            .iter()
            .filter(|e| e.evidence_kind == EvidenceKind::IngestionJob)
            .collect();

        let gate_id = format!("gate-{}", uuid::Uuid::new_v4());

        if ingest_claims.is_empty() {
            return Ok(VerificationGateResult {
                gate_id,
                gate_name: self.name().into(),
                passed: true,
                resulting_state: CompletionState::NotRun,
                blocked_claims: vec![],
                required_missing_evidence: vec![],
                explanation: "no Ingested/Indexed claims to evaluate".into(),
                provenance_record_id: String::new(),
            });
        }

        if ingest_evidence.is_empty() {
            return Ok(VerificationGateResult {
                gate_id,
                gate_name: self.name().into(),
                passed: false,
                resulting_state: CompletionState::NotRun,
                blocked_claims: ingest_claims.iter().map(|c| c.claim_id.clone()).collect(),
                required_missing_evidence: vec![EvidenceKind::IngestionJob],
                explanation: "no IngestionJob evidence found".into(),
                provenance_record_id: String::new(),
            });
        }

        let has_failed = ingest_evidence
            .iter()
            .any(|e| e.status == EvidenceStatus::Failed);
        if has_failed {
            return Ok(VerificationGateResult {
                gate_id,
                gate_name: self.name().into(),
                passed: false,
                resulting_state: CompletionState::Failed,
                blocked_claims: ingest_claims.iter().map(|c| c.claim_id.clone()).collect(),
                required_missing_evidence: vec![],
                explanation: "IngestionJob evidence has Failed status".into(),
                provenance_record_id: String::new(),
            });
        }

        let has_missing = ingest_evidence
            .iter()
            .any(|e| e.status == EvidenceStatus::Missing);
        if has_missing {
            return Ok(VerificationGateResult {
                gate_id,
                gate_name: self.name().into(),
                passed: false,
                resulting_state: CompletionState::NotRun,
                blocked_claims: ingest_claims.iter().map(|c| c.claim_id.clone()).collect(),
                required_missing_evidence: vec![EvidenceKind::IngestionJob],
                explanation: "IngestionJob evidence is missing — no ingestion results available"
                    .into(),
                provenance_record_id: String::new(),
            });
        }

        let all_passed = ingest_evidence
            .iter()
            .all(|e| e.status == EvidenceStatus::Passed);
        if !all_passed {
            return Ok(VerificationGateResult {
                gate_id,
                gate_name: self.name().into(),
                passed: false,
                resulting_state: CompletionState::NotRun,
                blocked_claims: ingest_claims.iter().map(|c| c.claim_id.clone()).collect(),
                required_missing_evidence: vec![EvidenceKind::IngestionJob],
                explanation: "IngestionJob evidence has non-Passed status".into(),
                provenance_record_id: String::new(),
            });
        }

        Ok(VerificationGateResult {
            gate_id,
            gate_name: self.name().into(),
            passed: true,
            resulting_state: CompletionState::Verified,
            blocked_claims: vec![],
            required_missing_evidence: vec![],
            explanation: format!(
                "{} ingest/index claims verified by {} IngestionJob records",
                ingest_claims.len(),
                ingest_evidence.len()
            ),
            provenance_record_id: String::new(),
        })
    }
}

// ── AnswerGroundedGate ───────────────────────────────────────────────────────

pub struct AnswerGroundedGate;

#[async_trait]
impl VerificationGate for AnswerGroundedGate {
    fn name(&self) -> &'static str {
        "AnswerGroundedGate"
    }

    async fn evaluate(
        &self,
        request: &VerificationGateRequest,
    ) -> Result<VerificationGateResult, EvidenceEngineError> {
        let grounded_claims: Vec<&CompletionClaim> = request
            .claims
            .iter()
            .filter(|c| c.claim_kind == CompletionClaimKind::AnswerGrounded)
            .collect();

        let citation_evidence: Vec<&CompletionEvidence> = request
            .available_evidence
            .iter()
            .filter(|e| e.evidence_kind == EvidenceKind::CitationTrace)
            .collect();

        let gate_id = format!("gate-{}", uuid::Uuid::new_v4());

        if grounded_claims.is_empty() {
            return Ok(VerificationGateResult {
                gate_id,
                gate_name: self.name().into(),
                passed: true,
                resulting_state: CompletionState::NotRun,
                blocked_claims: vec![],
                required_missing_evidence: vec![],
                explanation: "no AnswerGrounded claims to evaluate".into(),
                provenance_record_id: String::new(),
            });
        }

        if citation_evidence.is_empty()
            || citation_evidence
                .iter()
                .all(|e| e.status == EvidenceStatus::Missing)
        {
            return Ok(VerificationGateResult {
                gate_id,
                gate_name: self.name().into(),
                passed: false,
                resulting_state: CompletionState::NotRun,
                blocked_claims: grounded_claims.iter().map(|c| c.claim_id.clone()).collect(),
                required_missing_evidence: vec![EvidenceKind::CitationTrace],
                explanation: "no CitationTrace evidence found for AnswerGrounded claims".into(),
                provenance_record_id: String::new(),
            });
        }

        Ok(VerificationGateResult {
            gate_id,
            gate_name: self.name().into(),
            passed: true,
            resulting_state: CompletionState::Verified,
            blocked_claims: vec![],
            required_missing_evidence: vec![],
            explanation: format!(
                "{} AnswerGrounded claims supported by citation evidence",
                grounded_claims.len()
            ),
            provenance_record_id: String::new(),
        })
    }
}

// ── Run all gates ────────────────────────────────────────────────────────────

pub async fn run_all_gates(
    claims: &[CompletionClaim],
    evidence: &[CompletionEvidence],
    run_id: &str,
    task_type: &str,
) -> Result<Vec<VerificationGateResult>, EvidenceEngineError> {
    let request = VerificationGateRequest {
        run_id: run_id.to_string(),
        task_type: task_type.to_string(),
        claims: claims.to_vec(),
        available_evidence: evidence.to_vec(),
        policy_profile: "default".into(),
    };

    let gates: Vec<Box<dyn VerificationGate>> = vec![
        Box::new(TestsPassGate),
        Box::new(EvidenceBackedGate {
            name: "BuildPassesGate",
            claim_kinds: &[CompletionClaimKind::BuildPasses],
            required_evidence: &[EvidenceKind::BuildResult],
        }),
        Box::new(EvidenceBackedGate {
            name: "ImplementedGate",
            claim_kinds: &[CompletionClaimKind::Implemented],
            required_evidence: &[EvidenceKind::FileDiff, EvidenceKind::GeneratedArtifact],
        }),
        Box::new(EvidenceBackedGate {
            name: "FixedGate",
            claim_kinds: &[CompletionClaimKind::Fixed],
            required_evidence: &[EvidenceKind::FileDiff, EvidenceKind::TestRun],
        }),
        Box::new(IngestedGate),
        Box::new(AnswerGroundedGate),
        Box::new(EvidenceBackedGate {
            name: "VerifiedGate",
            claim_kinds: &[CompletionClaimKind::Verified],
            required_evidence: &[EvidenceKind::ReviewFinding],
        }),
        Box::new(EvidenceBackedGate {
            name: "DocumentedGate",
            claim_kinds: &[CompletionClaimKind::Documented],
            required_evidence: &[EvidenceKind::GeneratedArtifact],
        }),
        Box::new(EvidenceBackedGate {
            name: "DoneGate",
            claim_kinds: &[CompletionClaimKind::Done],
            required_evidence: &[EvidenceKind::GateResult],
        }),
    ];

    let mut results = Vec::new();
    for gate in gates {
        results.push(gate.evaluate(&request).await?);
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_claim(id: &str, run_id: &str, kind: CompletionClaimKind) -> CompletionClaim {
        CompletionClaim {
            claim_id: id.into(),
            run_id: run_id.into(),
            agent_key: None,
            model: None,
            task_type: "test".into(),
            claim_text: "test claim".into(),
            claim_kind: kind,
            required_evidence: vec![],
            linked_evidence_ids: vec![],
            verified: false,
            contradiction_ids: vec![],
            created_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn make_evidence(
        id: &str,
        run_id: &str,
        kind: EvidenceKind,
        status: EvidenceStatus,
    ) -> CompletionEvidence {
        CompletionEvidence {
            evidence_id: id.into(),
            run_id: run_id.into(),
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
            provenance_record_id: String::new(),
            started_at: "2026-01-01T00:00:00Z".into(),
            completed_at: Some("2026-01-01T00:01:00Z".into()),
        }
    }

    #[tokio::test]
    async fn test_tests_pass_gate_passes_with_evidence() {
        let gate = TestsPassGate;
        let claims = vec![make_claim("cl-1", "run-1", CompletionClaimKind::TestsPass)];
        let evidence = vec![make_evidence(
            "ev-1",
            "run-1",
            EvidenceKind::TestRun,
            EvidenceStatus::Passed,
        )];
        let req = VerificationGateRequest {
            run_id: "run-1".into(),
            task_type: "test".into(),
            claims,
            available_evidence: evidence,
            policy_profile: "default".into(),
        };
        let result = gate.evaluate(&req).await.unwrap();
        assert!(result.passed);
        assert_eq!(result.resulting_state, CompletionState::Verified);
    }

    #[tokio::test]
    async fn test_tests_pass_gate_fails_with_no_evidence() {
        let gate = TestsPassGate;
        let claims = vec![make_claim("cl-1", "run-1", CompletionClaimKind::TestsPass)];
        let req = VerificationGateRequest {
            run_id: "run-1".into(),
            task_type: "test".into(),
            claims,
            available_evidence: vec![],
            policy_profile: "default".into(),
        };
        let result = gate.evaluate(&req).await.unwrap();
        assert!(!result.passed);
        assert_eq!(result.resulting_state, CompletionState::NotRun);
        assert!(result.blocked_claims.contains(&"cl-1".to_string()));
    }

    #[tokio::test]
    async fn test_tests_pass_gate_fails_with_failed_evidence() {
        let gate = TestsPassGate;
        let claims = vec![make_claim("cl-1", "run-1", CompletionClaimKind::TestsPass)];
        let evidence = vec![make_evidence(
            "ev-1",
            "run-1",
            EvidenceKind::TestRun,
            EvidenceStatus::Failed,
        )];
        let req = VerificationGateRequest {
            run_id: "run-1".into(),
            task_type: "test".into(),
            claims,
            available_evidence: evidence,
            policy_profile: "default".into(),
        };
        let result = gate.evaluate(&req).await.unwrap();
        assert!(!result.passed);
        assert_eq!(result.resulting_state, CompletionState::Failed);
    }

    #[tokio::test]
    async fn test_ingested_gate_requires_ingestion_job() {
        let gate = IngestedGate;
        let claims = vec![make_claim("cl-1", "run-1", CompletionClaimKind::Ingested)];
        let evidence = vec![make_evidence(
            "ev-1",
            "run-1",
            EvidenceKind::IngestionJob,
            EvidenceStatus::Passed,
        )];
        let req = VerificationGateRequest {
            run_id: "run-1".into(),
            task_type: "test".into(),
            claims: claims.clone(),
            available_evidence: evidence,
            policy_profile: "default".into(),
        };
        let passed = gate.evaluate(&req).await.unwrap();
        assert!(passed.passed);

        // Without evidence — fails
        let req2 = VerificationGateRequest {
            run_id: "run-1".into(),
            task_type: "test".into(),
            claims,
            available_evidence: vec![],
            policy_profile: "default".into(),
        };
        let failed = gate.evaluate(&req2).await.unwrap();
        assert!(!failed.passed);
    }

    #[tokio::test]
    async fn test_answer_grounded_gate_requires_citations() {
        let gate = AnswerGroundedGate;
        let claims = vec![make_claim(
            "cl-1",
            "run-1",
            CompletionClaimKind::AnswerGrounded,
        )];
        let evidence = vec![make_evidence(
            "ev-1",
            "run-1",
            EvidenceKind::CitationTrace,
            EvidenceStatus::Passed,
        )];
        let req = VerificationGateRequest {
            run_id: "run-1".into(),
            task_type: "test".into(),
            claims,
            available_evidence: evidence,
            policy_profile: "default".into(),
        };
        let result = gate.evaluate(&req).await.unwrap();
        assert!(result.passed);
    }

    #[tokio::test]
    async fn test_no_claims_passes_vacuously() {
        let gate = TestsPassGate;
        let req = VerificationGateRequest {
            run_id: "run-1".into(),
            task_type: "test".into(),
            claims: vec![],
            available_evidence: vec![],
            policy_profile: "default".into(),
        };
        let result = gate.evaluate(&req).await.unwrap();
        assert!(result.passed);
    }

    #[tokio::test]
    async fn test_every_claim_kind_blocks_without_required_evidence() {
        let kinds = [
            CompletionClaimKind::Done,
            CompletionClaimKind::Implemented,
            CompletionClaimKind::Fixed,
            CompletionClaimKind::TestsPass,
            CompletionClaimKind::BuildPasses,
            CompletionClaimKind::Verified,
            CompletionClaimKind::Documented,
            CompletionClaimKind::Ingested,
            CompletionClaimKind::Indexed,
            CompletionClaimKind::AnswerGrounded,
        ];
        let claims: Vec<CompletionClaim> = kinds
            .iter()
            .enumerate()
            .map(|(idx, kind)| make_claim(&format!("cl-{idx}"), "run-1", *kind))
            .collect();

        let results = run_all_gates(&claims, &[], "run-1", "test").await.unwrap();
        let blocked_claims: std::collections::HashSet<String> = results
            .iter()
            .flat_map(|result| result.blocked_claims.iter().cloned())
            .collect();

        for claim in &claims {
            assert!(
                blocked_claims.contains(&claim.claim_id),
                "{:?} must be blocked without required evidence",
                claim.claim_kind
            );
            assert!(
                !claim_has_passed_gate(claim, &results),
                "{:?} must not have a passed relevant gate without evidence",
                claim.claim_kind
            );
        }
    }
}

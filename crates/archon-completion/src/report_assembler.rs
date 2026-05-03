//! Report assembler — produces a [`CompletionReport`] from claims, evidence, and gate results.
//!
//! The calibrated summary rewrites the original answer text to downgrade unsupported claims.

use crate::errors::EvidenceEngineError;
use crate::models::*;

/// Assemble a completion report from claims, evidence, and gate results.
pub fn assemble_report(
    claims: Vec<CompletionClaim>,
    evidence: Vec<CompletionEvidence>,
    gate_results: &[VerificationGateResult],
    run_id: &str,
    original_answer_hint: Option<&str>,
) -> Result<CompletionReport, EvidenceEngineError> {
    let report_id = format!(
        "rep-{}",
        uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
    );

    // Collect failed gates
    let failed_gates: Vec<String> = gate_results
        .iter()
        .filter(|g| !g.passed)
        .map(|g| g.gate_name.clone())
        .collect();

    // Collect unverified (blocked) claims
    let unverified_claims: Vec<String> = gate_results
        .iter()
        .flat_map(|g| g.blocked_claims.clone())
        .collect();

    // Determine final state
    let final_state = if failed_gates.is_empty() && claims.is_empty() {
        CompletionState::NotRun
    } else if failed_gates.is_empty() {
        CompletionState::Verified
    } else if gate_results.iter().any(|g| g.resulting_state == CompletionState::Failed) {
        CompletionState::Failed
    } else if gate_results.iter().any(|g| g.resulting_state == CompletionState::NotRun) {
        if claims.iter().any(|c| c.verified) {
            CompletionState::Partial
        } else {
            CompletionState::Attempted
        }
    } else {
        CompletionState::Partial
    };

    // Build calibrated summary
    let calibrated_summary = build_calibrated_summary(
        &claims,
        &evidence,
        gate_results,
        &final_state,
        original_answer_hint,
    );

    let now = chrono::Utc::now().to_rfc3339();

    Ok(CompletionReport {
        report_id,
        run_id: run_id.to_string(),
        final_state,
        claims,
        evidence,
        failed_gates,
        unverified_claims,
        contradictions: vec![],
        calibrated_summary,
        provenance_record_id: String::new(),
        created_at: now,
    })
}

fn build_calibrated_summary(
    claims: &[CompletionClaim],
    evidence: &[CompletionEvidence],
    gate_results: &[VerificationGateResult],
    final_state: &CompletionState,
    original_hint: Option<&str>,
) -> String {
    let mut summary = String::new();

    summary.push_str(&format!(
        "## Completion Report — State: {}\n\n",
        completion_state_label(final_state)
    ));

    // Claims section
    summary.push_str("### Claims\n\n");
    if claims.is_empty() {
        summary.push_str("No completion-sensitive claims detected.\n\n");
    } else {
        for claim in claims {
            let status = if claim.verified {
                "VERIFIED"
            } else if gate_results.iter().any(|g| g.blocked_claims.contains(&claim.claim_id)) {
                "BLOCKED"
            } else {
                "UNCHECKED"
            };
            summary.push_str(&format!(
                "- [{status}] `{kind}` — \"{text}\"\n",
                kind = claim_kind_label(&claim.claim_kind),
                text = claim.claim_text,
            ));
        }
        summary.push_str("\n");
    }

    // Evidence section
    summary.push_str("### Evidence\n\n");
    if evidence.is_empty() {
        summary.push_str("No evidence collected.\n\n");
    } else {
        for ev in evidence {
            summary.push_str(&format!(
                "- [{status}] `{kind}` from {producer}",
                status = evidence_status_label(&ev.status),
                kind = evidence_kind_label(&ev.evidence_kind),
                producer = ev.producer,
            ));
            if let Some(ref summary_text) = ev.stdout_summary {
                summary.push_str(&format!(" — {summary_text}"));
            }
            summary.push('\n');
        }
        summary.push_str("\n");
    }

    // Gate results section
    summary.push_str("### Verification Gates\n\n");
    if gate_results.is_empty() {
        summary.push_str("No gates evaluated.\n\n");
    } else {
        for gate in gate_results {
            let status = if gate.passed { "PASS" } else { "FAIL" };
            summary.push_str(&format!(
                "- [{status}] {} — {}\n",
                gate.gate_name, gate.explanation,
            ));
        }
        summary.push_str("\n");
    }

    // Downgrade unsupported claims in the calibrated rewrite
    summary.push_str("### Calibrated Assessment\n\n");
    match final_state {
        CompletionState::Verified => {
            summary.push_str(
                "All completion-sensitive claims are supported by evidence.\n",
            );
        }
        CompletionState::Partial => {
            summary.push_str(
                "Some claims are supported by evidence, but others could not be verified. "
            );
            summary.push_str(
                "Claims without evidence should be treated as unconfirmed.\n",
            );
        }
        CompletionState::Attempted => {
            summary.push_str(
                "Completion-sensitive claims were detected but evidence is insufficient to verify them. "
            );
            summary.push_str(
                "The claims represent intent or attempt, not confirmed completion.\n",
            );
        }
        CompletionState::Failed => {
            summary.push_str(
                "One or more completion claims are contradicted by evidence. "
            );
            summary.push_str(
                "Claims of completion should not be relied upon without additional verification.\n",
            );
        }
        CompletionState::NotRun => {
            summary.push_str(
                "No completion-sensitive operations were executed or no claims were made.\n",
            );
        }
    }

    if let Some(hint) = original_hint {
        if *final_state != CompletionState::Verified {
            summary.push_str(&format!(
                "\nOriginal answer claimed completion, but evidence does not fully support it. "
            ));
            summary.push_str(&format!("Original text: \"{}\"\n", hint));
        }
    }

    summary
}

fn completion_state_label(state: &CompletionState) -> &'static str {
    match state {
        CompletionState::Verified => "Verified",
        CompletionState::Partial => "Partial",
        CompletionState::Attempted => "Attempted",
        CompletionState::Failed => "Failed",
        CompletionState::NotRun => "NotRun",
    }
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

fn evidence_status_label(status: &EvidenceStatus) -> &'static str {
    match status {
        EvidenceStatus::Passed => "PASSED",
        EvidenceStatus::Failed => "FAILED",
        EvidenceStatus::Missing => "MISSING",
        EvidenceStatus::Skipped => "SKIPPED",
        EvidenceStatus::Unknown => "UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calibrated_summary_downgrades_unsupported() {
        let claims = vec![CompletionClaim {
            claim_id: "cl-1".into(),
            run_id: "run-1".into(),
            agent_key: None,
            model: None,
            task_type: "test".into(),
            claim_text: "All tests pass.".into(),
            claim_kind: CompletionClaimKind::TestsPass,
            required_evidence: vec![EvidenceKind::TestRun],
            linked_evidence_ids: vec![],
            verified: false,
            contradiction_ids: vec![],
            created_at: "2026-01-01T00:00:00Z".into(),
        }];

        let gate_results = vec![VerificationGateResult {
            gate_id: "gate-1".into(),
            gate_name: "TestsPassGate".into(),
            passed: false,
            resulting_state: CompletionState::NotRun,
            blocked_claims: vec!["cl-1".into()],
            required_missing_evidence: vec![EvidenceKind::TestRun],
            explanation: "no TestRun evidence found".into(),
            provenance_record_id: String::new(),
        }];

        let report = assemble_report(claims, vec![], &gate_results, "run-1", None).unwrap();
        assert_eq!(report.final_state, CompletionState::Attempted);
        assert!(report.calibrated_summary.contains("insufficient"));
        assert!(report.calibrated_summary.contains("not confirmed"));
    }

    #[test]
    fn test_calibrated_summary_keeps_verified_claims_intact() {
        let claims = vec![CompletionClaim {
            claim_id: "cl-1".into(),
            run_id: "run-1".into(),
            agent_key: None,
            model: None,
            task_type: "test".into(),
            claim_text: "All tests pass.".into(),
            claim_kind: CompletionClaimKind::TestsPass,
            required_evidence: vec![EvidenceKind::TestRun],
            linked_evidence_ids: vec!["ev-1".into()],
            verified: true,
            contradiction_ids: vec![],
            created_at: "2026-01-01T00:00:00Z".into(),
        }];

        let evidence = vec![CompletionEvidence {
            evidence_id: "ev-1".into(),
            run_id: "run-1".into(),
            evidence_kind: EvidenceKind::TestRun,
            producer: "test".into(),
            command_or_operation: None,
            status: EvidenceStatus::Passed,
            exit_code: Some(0),
            input_hash: None,
            output_hash: None,
            stdout_summary: Some("42 tests passed".into()),
            stderr_summary: None,
            artifact_ids: vec![],
            provenance_record_id: String::new(),
            started_at: "2026-01-01T00:00:00Z".into(),
            completed_at: Some("2026-01-01T00:01:00Z".into()),
        }];

        let gate_results = vec![VerificationGateResult {
            gate_id: "gate-1".into(),
            gate_name: "TestsPassGate".into(),
            passed: true,
            resulting_state: CompletionState::Verified,
            blocked_claims: vec![],
            required_missing_evidence: vec![],
            explanation: "1 claim verified by 1 evidence record".into(),
            provenance_record_id: String::new(),
        }];

        let report = assemble_report(claims, evidence, &gate_results, "run-1", None).unwrap();
        assert_eq!(report.final_state, CompletionState::Verified);
        assert!(report.calibrated_summary.contains("supported by evidence"));
    }
}

//! A branch demoted for a failed declared contract, with every finding the
//! contract reported (Issue 219): none dropped, each bounded with a mark.

use super::*;

/// Longest single finding a demotion's residual gap quotes; a longer one is
/// cut with a mark, and every finding is kept whole in
/// `data.declared_contract_findings`.
const GAP_FINDING_CHARS: usize = 500;

/// Every finding, each bounded with a clear mark, none dropped (Issue 219).
fn gap_findings(findings: &[String]) -> String {
    let quoted: Vec<String> = (findings.iter())
        .map(|finding| {
            let length = finding.chars().count();
            if length <= GAP_FINDING_CHARS {
                return finding.clone();
            }
            let kept: String = finding.chars().take(GAP_FINDING_CHARS).collect();
            format!("{kept} [finding cut at {GAP_FINDING_CHARS} of {length} chars; whole in data.declared_contract_findings]")
        })
        .collect();
    format!("{} finding(s): {}", findings.len(), quoted.join("; "))
}

pub(in crate::v2::verification) fn demote_failed_contract(outcome: &mut WorkflowV2BranchOutcome, findings: &[String]) {
    let quoted = gap_findings(findings);
    if let Some(result) = outcome.result.as_mut() {
        result.status = WorkflowV2Status::NeedsReview;
        result.residual_gaps.push(crate::WorkflowV2ResidualGap {
            id: "declared_contract_verification_failed".to_string(),
            description: format!(
                "host-executed declared deliverable contract verification failed: {quoted}"
            ),
            severity: Some("review".to_string()),
        });
        result.evidence.push(WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Blocker,
            "accepted branch demoted: the host ran the declared deliverable contract verifier and it failed",
        ));
        let mut data = result.data.as_object().cloned().unwrap_or_default();
        data.insert(
            "declared_contract_verification".to_string(),
            serde_json::json!("failed"),
        );
        data.insert(
            "declared_contract_findings".to_string(),
            serde_json::json!(findings),
        );
        data.insert(
            "verification_failure_class".to_string(),
            serde_json::json!("declared_contract_violation"),
        );
        result.data = serde_json::Value::Object(data);
    }
    outcome.status = WorkflowV2Status::NeedsReview;
    outcome.failure_kind = Some(BranchFailureKind::Semantic);
}

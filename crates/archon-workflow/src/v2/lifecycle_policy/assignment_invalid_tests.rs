use serde_json::Value;

use super::{VERDICT, admit, escalation, evidence_references, invalid_task_ids};
use crate::v2::lifecycle_policy::cross_cutting::merge_review;

fn admissible() -> Value {
    serde_json::json!({
        "id": "finding-1",
        "verdict": "assignment_invalid",
        "canonical_task_ids": ["TASK-001"],
        "attribution_source": "review_branch",
        "reason": "the criterion demands a hook on a sealed trait that has no interception seam",
        "evidence": ["crates/archon-llm/src/provider.rs:212 the trait is sealed here"],
    })
}

#[test]
fn an_evidenced_branch_verdict_is_admitted() {
    let admission = admit(&[admissible()]);
    assert_eq!(admission.admitted.len(), 1);
    assert!(admission.rejected.is_empty());
    assert_eq!(admission.findings[0]["assignment_invalid_admitted"], true);
    assert_eq!(invalid_task_ids(&admission.admitted), vec!["TASK-001"]);
}

#[test]
fn the_verdict_spelling_is_tolerant_but_the_conditions_are_not() {
    let mut finding = admissible();
    finding["verdict"] = serde_json::json!("Assignment-Invalid");
    assert_eq!(admit(&[finding]).admitted.len(), 1);
}

#[test]
fn a_verdict_without_a_file_line_reference_is_refused() {
    let mut finding = admissible();
    finding["evidence"] = serde_json::json!(["the trait cannot be extended"]);
    let admission = admit(&[finding]);

    assert!(admission.admitted.is_empty());
    assert_eq!(admission.rejected.len(), 1);
    // Refused, not dropped: the claim continues into the round as ordinary
    // remediable work, with the verdict key stripped so nothing downstream
    // re-reads it as a verdict.
    assert_eq!(admission.findings.len(), 1);
    assert!(admission.findings[0].get("verdict").is_none());
    assert!(
        admission.findings[0]["assignment_invalid_rejected"]
            .to_string()
            .contains("file:line")
    );
}

#[test]
fn a_bare_file_path_is_not_a_file_line_reference() {
    let mut finding = admissible();
    finding["evidence"] = serde_json::json!(["crates/archon-llm/src/provider.rs"]);
    assert!(evidence_references(&finding).is_empty());
    assert!(admit(&[finding]).admitted.is_empty());
}

#[test]
fn a_verdict_without_a_stated_reason_is_refused() {
    let mut finding = admissible();
    finding["reason"] = serde_json::json!("invalid");
    // Every other reason-bearing key has to be empty too, or the finding would
    // legitimately have a diagnosis under a different name.
    let admission = admit(&[finding]);
    assert!(admission.admitted.is_empty());
}

#[test]
fn a_verdict_naming_no_task_is_refused() {
    let mut finding = admissible();
    finding["canonical_task_ids"] = serde_json::json!([]);
    assert!(admit(&[finding]).admitted.is_empty());
}

#[test]
fn a_verdict_without_branch_attribution_is_refused() {
    let mut finding = admissible();
    finding
        .as_object_mut()
        .unwrap()
        .remove("attribution_source");
    let admission = admit(&[finding]);
    assert!(admission.admitted.is_empty());
    assert!(
        admission.rejected[0]["admission_failures"]
            .to_string()
            .contains("review_branch")
    );
}

#[test]
fn merge_promotes_an_admitted_verdict_to_the_review_status() {
    let review = merge_review(
        &[admissible()],
        &serde_json::json!({ "status": "accepted" }),
    );
    assert_eq!(review["status"], VERDICT);
    assert_eq!(escalation(&review).map(|items| items.len()), Some(1));
}

#[test]
fn merge_refuses_a_cross_cutting_verdict() {
    // Same finding, arriving from the terminal reduce instead of a per-task
    // branch. `merge_review` stamps it `cross_cutting`, which is the refusal.
    let cross = serde_json::json!({ "status": "needs_remediation", "items": [admissible()] });
    let review = merge_review(&[], &cross);

    assert_eq!(review["status"], "needs_remediation");
    assert!(escalation(&review).is_none());
    assert_eq!(
        review["assignment_invalid_rejected"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn the_original_two_verdicts_merge_exactly_as_before() {
    // Structural merge, unchanged: a finding forces remediation over an
    // accepting reducer, and an empty finding set accepts only when the reducer
    // itself accepted.
    let finding = serde_json::json!({ "id": "f1", "claim": "not wired" });
    let accepting = serde_json::json!({ "status": "accepted" });
    let remediating = merge_review(&[finding], &accepting);
    assert_eq!(remediating["status"], "needs_remediation");
    assert!(
        remediating["assignment_invalid"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    assert_eq!(merge_review(&[], &accepting)["status"], "accepted");
    assert_eq!(
        merge_review(&[], &serde_json::json!({ "status": "needs_review" }))["status"],
        "cross_cutting_review_not_accepted"
    );
}

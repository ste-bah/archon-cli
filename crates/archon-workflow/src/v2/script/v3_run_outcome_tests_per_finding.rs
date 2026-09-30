//! Batch O: the terminal rule closes each finding only on a verifier's own
//! evidenced verdict on it.

use super::super::remediation_dispositions::DispositionFact;
use super::*;
use crate::v2::review_finding_ids::finding_id_of;

fn said(disposition: &str, evidence: bool, mutation: bool) -> DispositionFact {
    DispositionFact {
        disposition: disposition.into(),
        evidence,
        mutation_failed: mutation,
        said: "scripted".into(),
        ..Default::default()
    }
}

/// TASK-A's finding judged by a verifier that said `verdict` of it.
fn judged(finding: serde_json::Value, verdict: DispositionFact, refutation: bool) -> Case {
    let id = finding_id_of(&finding);
    let mut case = Case::clean();
    case.result = accounting(serde_json::json!({
        "adversarial_findings": [finding],
        "review_remediation": { "resolved": [{ "taskId": A, "findingIds": [id] }], "unresolved": [], "unassigned": [] },
    }));
    let mut fixed = fix(A, 1, Accepted);
    fixed.remediation.finding_ids = vec![id.clone()];
    fixed.landed_nothing = refutation;
    let mut check = rverify(A, 1, Accepted, true);
    check.remediation.finding_ids = vec![id.clone()];
    check.remediation.refutation = refutation;
    check.remediation.dispositions.insert(id, verdict);
    case.replace("fix-TASK-A-1", fixed);
    case.replace("rverify-TASK-A-1-true", check);
    case
}

fn plain() -> serde_json::Value {
    serde_json::json!({ "id": "f1", "canonical_task_ids": [A], "claim": "the reader drops the error" })
}

fn about_a_check() -> serde_json::Value {
    serde_json::json!({ "id": "f2", "canonical_task_ids": [A],
        "claim": "the artifact test only checks that the report exists, never its status" })
}

#[test]
fn an_accepted_verifier_closes_only_what_it_proved() {
    let closed = judged(plain(), said("resolved", true, false), false).decide();
    assert_eq!(closed.status, Accepted, "{}", closed.explanation());
    // Accepted overall, but the finding itself left open: it holds the run.
    judged(plain(), said("open", true, false), false).holds("is open after");
    // No evidence closes nothing.
    judged(plain(), said("resolved", false, false), false).holds("no evidence");
    // No entry at all.
    judged(plain(), DispositionFact::default(), false).holds("no disposition");
}

#[test]
fn a_check_finding_needs_a_failing_mutation() {
    judged(about_a_check(), said("resolved", true, false), false).holds("mutation");
    let proven = judged(about_a_check(), said("resolved", true, true), false).decide();
    assert_eq!(proven.status, Accepted, "{}", proven.explanation());
}

#[test]
fn a_fix_that_landed_nothing_is_closed_only_by_its_own_evidenced_verifier() {
    // Batch O: a verifier judging an empty fix judges whether the finding
    // still holds on the tree as it is, under the same evidence rule.
    let gone = judged(plain(), said("resolved", true, false), true).decide();
    assert_eq!(gone.status, Accepted, "{}", gone.explanation());
    judged(plain(), said("resolved", false, false), true).holds("no evidence");
    let refuted = judged(plain(), said("invalid", true, false), true).decide();
    assert_eq!(refuted.status, Accepted, "{}", refuted.explanation());
    // Without the refutation contract, a verifier after a no-op fix judges
    // nothing.
    let mut case = judged(plain(), said("resolved", true, false), false);
    case.calls
        .iter_mut()
        .find(|call| call.id == "fix-TASK-A-1")
        .unwrap()
        .landed_nothing = true;
    case.holds("landed nothing");
}

#[test]
fn a_verdict_of_another_unit_or_round_closes_nothing() {
    let mut case = judged(plain(), said("resolved", true, false), false);
    case.calls
        .iter_mut()
        .find(|call| call.id == "rverify-TASK-A-1-true")
        .unwrap()
        .remediation
        .unit = Some("TASK-A#2of2".into());
    case.holds("judged no fix of its unit and round");
}

// Tests for the per-task adversarial review stage and the narrowed terminal
// reduce. The two load-bearing claims are proved here directly:
//
//   * attribution is STRUCTURAL — a reviewer that emits a finding with no task
//     key of any kind is still attributed correctly, because the branch it ran
//     in identifies exactly one task;
//   * the terminal reduce cannot duplicate per-task findings — a restated
//     finding is dropped by identity, not by instruction.

use std::collections::BTreeSet;

use serde_json::json;

use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::lifecycle_policy::cross_cutting;

use super::*;

fn task(id: &str, notes: &[&str]) -> crate::task_universe::WorkflowV2TaskUniverseTask {
    crate::task_universe::WorkflowV2TaskUniverseTask {
        canonical_task_id: id.to_string(),
        source_path: format!("/prd/TASK-{id}.md"),
        acceptance_criteria: vec![format!("{id} must be provably done")],
        adversarial_review_notes: notes.iter().map(|note| (*note).to_string()).collect(),
        ..Default::default()
    }
}

fn universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "1".to_string(),
        source_roots: vec!["/prd".to_string()],
        tasks: vec![
            task("TASK-TDL-001", &["Verify residual gaps fail closed."]),
            task("TASK-TDL-002", &[]),
        ],
    }
}

fn ids(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|id| (*id).to_string()).collect()
}

#[test]
fn each_task_gets_its_own_review_item_carrying_its_own_notes() {
    let items = per_task_review_items(&universe(), &ids(&["TASK-TDL-001", "TASK-TDL-002"]), &[]);
    assert_eq!(
        items.len(),
        2,
        "one review branch per task, not one for all"
    );
    assert_eq!(items[0]["item_id"], "adversarial-review-TASK-TDL-001");
    assert_eq!(items[0]["canonical_task_ids"], json!(["TASK-TDL-001"]));
    assert_eq!(
        items[0]["adversarial_review_notes"],
        json!(["Verify residual gaps fail closed."]),
        "the task file's own Adversarial Review Notes must reach its reviewer"
    );
    // A task that declares no notes still gets a reviewer; it simply has no
    // extra hypotheses to answer.
    assert_eq!(items[1]["adversarial_review_notes"], json!([]));
}

#[test]
fn a_reviewer_only_sees_its_own_tasks_evidence() {
    let implementation = vec![
        json!({"canonical_task_ids": ["TASK-TDL-001"], "changed": ["src/one.rs"]}),
        json!({"canonical_task_ids": ["TASK-TDL-002"], "changed": ["src/two.rs"]}),
    ];
    let items = per_task_review_items(&universe(), &ids(&["TASK-TDL-001"]), &[&implementation]);
    let evidence = items[0]["task_evidence"].as_array().expect("evidence");
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0]["changed"], json!(["src/one.rs"]));
}

/// THE DEFECT THIS STAGE EXISTS TO FIX.
///
/// On run `wf-ee4a92fc` all 43 adversarial findings came back carrying no task
/// key of any kind, so every one was routed to `unassigned` and remediated by
/// nobody. Here the reviewers emit exactly that — findings with no task key —
/// and every finding is still attributed, because the branch it came out of
/// reviews exactly one task and the host stamps the branch id.
#[test]
fn a_finding_with_no_task_key_is_still_attributed_to_its_branchs_task() {
    let items = per_task_review_items(&universe(), &ids(&["TASK-TDL-001", "TASK-TDL-002"]), &[]);
    let envelope = json!({
        "outcomes": [
            {
                "item_id": "adversarial-review-TASK-TDL-001",
                "result": {"data": {"findings": [
                    {"claim": "the artifact is empty", "severity": "high"},
                ]}},
            },
            {
                "item_id": "adversarial-review-TASK-TDL-002",
                "result": {"data": {"findings": [
                    {"claim": "the test asserts nothing", "severity": "medium"},
                ]}},
            },
        ]
    });
    let findings = attributed_findings(&items, &envelope);
    assert_eq!(findings.len(), 2);
    assert_eq!(findings[0]["canonical_task_ids"], json!(["TASK-TDL-001"]));
    assert_eq!(findings[1]["canonical_task_ids"], json!(["TASK-TDL-002"]));
    assert_eq!(findings[0]["attribution_source"], "review_branch");
}

#[test]
fn a_finding_that_names_other_tasks_keeps_them_and_gains_its_own() {
    let items = per_task_review_items(&universe(), &ids(&["TASK-TDL-001"]), &[]);
    let envelope = json!({
        "outcomes": [{
            "item_id": "adversarial-review-TASK-TDL-001",
            "result": {"data": {"findings": [
                {"claim": "contradicts a sibling", "canonical_task_ids": ["TASK-TDL-002"]},
            ]}},
        }]
    });
    let findings = attributed_findings(&items, &envelope);
    assert_eq!(
        findings[0]["canonical_task_ids"],
        json!(["TASK-TDL-001", "TASK-TDL-002"]),
        "the branch's own task is always present; a declared cross-reference is never removed"
    );
}

/// Fail-closed: when the branch cannot be identified at all, the finding is
/// returned unstamped so it surfaces as unassigned rather than being attached
/// to whichever task happened to be first.
#[test]
fn an_unidentifiable_branch_leaves_the_finding_unattributed_rather_than_guessing() {
    let items = per_task_review_items(&universe(), &ids(&["TASK-TDL-001", "TASK-TDL-002"]), &[]);
    let envelope = json!({
        "outcomes": [
            {"result": {"data": {"findings": [{"claim": "orphan"}]}}},
            {"item_id": "unknown-branch", "result": {"data": {"findings": []}}},
            {"item_id": "also-unknown", "result": {"data": {"findings": []}}},
        ]
    });
    let findings = attributed_findings(&items, &envelope);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].get("canonical_task_ids").is_none());
}

#[test]
fn per_task_findings_are_read_back_out_of_review_evidence() {
    let evidence = vec![
        json!({"kind": "adversarial-review-task", "findings": [{"claim": "one"}]}),
        json!({"kind": "review", "result": {"items": [{"claim": "not a per-task finding"}]}}),
        json!({"kind": "adversarial-review-task", "findings": [{"claim": "two"}]}),
    ];
    let findings = collected_per_task_findings(&evidence);
    assert_eq!(findings.len(), 2);
    assert_eq!(findings[1]["claim"], "two");
}

/// The terminal reduce must NOT re-review per-task work. Proved structurally:
/// a cross-cutting item restating a per-task finding is dropped by identity.
#[test]
fn the_terminal_reduce_cannot_duplicate_a_per_task_finding() {
    let per_task = vec![
        json!({"id": "F1", "claim": "the artifact is empty", "canonical_task_ids": ["TASK-TDL-001"]}),
    ];
    let cross = json!({
        "status": "needs_review",
        "summary": "cross-task pass",
        "items": [
            {"id": "F1", "claim": "the artifact is empty"},
            {"id": "X1", "claim": "TDL-001 and TDL-002 disagree on the schema"},
        ],
    });
    let merged = cross_cutting::merge_review(&per_task, &cross);
    let items = merged["items"].as_array().expect("items");
    assert_eq!(
        items.len(),
        2,
        "one per-task finding plus one genuinely new one"
    );
    assert_eq!(merged["duplicate_cross_cutting_findings_dropped"], 1);
    assert_eq!(merged["per_task_finding_count"], 1);
    assert_eq!(merged["cross_cutting_finding_count"], 1);
    assert_eq!(items[0]["id"], "F1");
    assert_eq!(items[1]["finding_scope"], "cross_cutting");
}

/// `review_needs_remediation` short-circuits on an accepted status, so a
/// cross-cutting reduce returning "accepted" over outstanding per-task findings
/// would have discarded every one of them. Status is derived, not copied.
#[test]
fn an_accepting_cross_cutting_reduce_cannot_bury_outstanding_per_task_findings() {
    let per_task =
        vec![json!({"id": "F1", "claim": "unproven", "canonical_task_ids": ["TASK-TDL-001"]})];
    let merged =
        cross_cutting::merge_review(&per_task, &json!({"status": "accepted", "items": []}));
    assert_eq!(merged["status"], "needs_remediation");
    assert!(crate::generated_lifecycle_remediation::review_needs_remediation(&merged));
}

#[test]
fn a_clean_run_accepts() {
    let merged = cross_cutting::merge_review(&[], &json!({"status": "accepted", "items": []}));
    assert_eq!(merged["status"], "accepted");
    assert!(!crate::generated_lifecycle_remediation::review_needs_remediation(&merged));
}

/// A reduce that never ran returns no items because it produced nothing, not
/// because it found nothing. Treating an empty result as acceptance would turn
/// a transport failure into a clean bill of health.
#[test]
fn a_cross_cutting_reduce_that_did_not_accept_cannot_produce_an_accepted_review() {
    let merged = cross_cutting::merge_review(
        &[],
        &json!({"status": "failed", "summary": "agent transport failed", "items": []}),
    );
    assert_eq!(merged["status"], "cross_cutting_review_not_accepted");
    assert!(!crate::generated_lifecycle_support::outcome_accepted_or_noop(&merged));
}

/// The narrow input is the other half of "it must not re-review per-task work":
/// the reduce is never handed the implementation or verification evidence.
#[test]
fn the_cross_cutting_reduce_is_given_a_digest_and_not_the_runs_evidence() {
    let per_task = vec![json!({
        "id": "F1",
        "claim": "the artifact is empty",
        "counter_evidence": "a very long blob that must not be forwarded",
        "canonical_task_ids": ["TASK-TDL-001"],
    })];
    let input = cross_cutting::cross_cutting_input(&json!({"tasks": []}), &per_task);
    assert_eq!(input["scope"], "cross_task_only");
    assert_eq!(input["perTaskFindingCount"], 1);
    assert!(input.get("implementationEvidence").is_none());
    assert!(input.get("verificationEvidence").is_none());
    let digest = input["perTaskFindingDigest"].as_array().expect("digest");
    assert_eq!(digest[0]["claim"], "the artifact is empty");
    assert!(
        digest[0].get("counter_evidence").is_none(),
        "the digest is for spotting contradictions, not for re-adjudicating a finding"
    );
}

#[test]
fn re_review_targets_exactly_the_tasks_the_round_remediated() {
    let inventory = json!({"items": [
        {"item_id": "fix-1", "canonical_task_ids": ["TASK-TDL-001"]},
        {"item_id": "fix-2", "canonical_task_ids": ["TASK-TDL-001", "TASK-TDL-002"]},
    ]});
    assert_eq!(
        cross_cutting::remediated_task_ids(&inventory),
        ids(&["TASK-TDL-001", "TASK-TDL-002"])
    );
}

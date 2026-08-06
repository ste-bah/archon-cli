// Regression cover for the two ORIGINAL verdicts.
//
// Adding a third verdict is only safe if the first two behave exactly as they
// did, so this drives the ordinary needs-remediation loop end to end and
// asserts the shape it has always had: seven review rounds (the initial one
// plus the `review_iteration <= 6` bound), six remediation waves, and a
// `blocked-review-unresolved` report at the end.

use serde_json::Value;

use super::LifecycleEvidence;
use super::review_test_host::{
    RecordingHost, accepted_report, driver, preamble_reply, seeded_review_evidence,
};

fn ordinary_finding() -> Value {
    serde_json::json!({
        "id": "finding-1",
        "claim": "the retry path is never exercised",
        "counter_evidence": "src/llm_retry.rs:88 has no test",
        "canonical_task_ids": ["TASK-001"],
        "attribution_source": "review_branch",
        "severity": "major",
    })
}

fn remediation_inventory() -> Value {
    serde_json::json!({
        "status": "needs_remediation",
        "items": [{
            "item_id": "review-fix-1",
            "canonical_task_ids": ["TASK-001"],
            "source_item_id": "finding-1",
            "failure_status": "needs_remediation",
            "failure_evidence": "src/llm_retry.rs:88 has no test",
            "required_fix": "cover the retry path",
            "focused_verification": ["cargo test -p archon-workflow retry"],
            "target_files": ["src/llm_retry.rs"],
            "artifact_requirements": [],
        }],
        "unresolved_issues": [],
    })
}

fn responder(method: &str, id: &str) -> Value {
    if let Some(reply) = preamble_reply(id) {
        return reply;
    }
    if id.starts_with("cross-cutting-review-") {
        // Accepts every round. The per-task finding is what keeps the loop
        // going, which is the structural-merge property this asserts.
        return serde_json::json!({ "status": "accepted", "items": [] });
    }
    if id.starts_with("review-remediation-inventory-") {
        return remediation_inventory();
    }
    if id.starts_with("review-remediation-wave-") {
        return serde_json::json!({
            "status": "accepted",
            "outcomes": [{ "item_id": "review-fix-1", "status": "accepted" }],
        });
    }
    if id.starts_with("review-verification-plan-") {
        return serde_json::json!({
            "status": "accepted",
            "items": [{
                "item_id": "verify-1",
                "canonical_task_ids": ["TASK-001"],
                "focused_verification": ["cargo test -p archon-workflow retry"],
            }],
        });
    }
    if id.starts_with("review-verification-wave-") {
        return serde_json::json!({
            "status": "accepted",
            "outcomes": [{ "item_id": "verify-1", "status": "accepted" }],
        });
    }
    if id.starts_with("adversarial-review-round-") {
        // The fix never actually closes the finding, so every re-review returns
        // it again — the worst case the bound exists for.
        return serde_json::json!({
            "status": "needs_remediation",
            "outcomes": [{
                "item_id": "adversarial-review-TASK-001",
                "status": "needs_review",
                "findings": [ordinary_finding()],
            }],
        });
    }
    if method == "finalReport" {
        return accepted_report();
    }
    panic!("unexpected host call: {method} {id}");
}

#[tokio::test]
async fn unremediated_findings_still_stop_at_six_rounds() {
    let host = RecordingHost::new(Box::new(responder));
    let driver = driver(host.clone());
    let mut evidence = LifecycleEvidence::default();
    evidence
        .review
        .push(seeded_review_evidence(vec![ordinary_finding()]));

    driver
        .run_review_and_final_gates(&serde_json::json!({ "items": [] }), &mut evidence)
        .await
        .expect("review gates run to a report");

    assert_eq!(
        host.count_starting_with("cross-cutting-review-"),
        7,
        "calls: {:?}",
        host.call_ids()
    );
    assert_eq!(host.count_starting_with("review-remediation-wave-"), 6);
    assert_eq!(host.count_starting_with("blocked-assignment-invalid-"), 0);
    assert!(
        host.call_ids()
            .contains(&"blocked-review-unresolved".to_string())
    );
}

#[tokio::test]
async fn a_clean_review_still_reaches_the_final_gates() {
    // The other original verdict. Nothing found, cross-cutting accepts, the
    // merge accepts, and the loop is never entered.
    let host = RecordingHost::new(Box::new(|method: &str, id: &str| {
        if let Some(reply) = preamble_reply(id) {
            return reply;
        }
        if id.starts_with("cross-cutting-review-") {
            return serde_json::json!({ "status": "accepted", "items": [] });
        }
        if id.starts_with("final-evidence-reconciliation-") {
            return serde_json::json!({ "status": "accepted", "items": [] });
        }
        if id == "require-final-artifacts" || id == "final-zero-gap-audit" {
            return serde_json::json!({ "status": "accepted", "items": [] });
        }
        if id == "final-acceptance-gate" {
            return serde_json::json!({ "status": "accepted" });
        }
        if method == "finalReport" {
            return accepted_report();
        }
        panic!("unexpected host call: {method} {id}");
    }));
    let driver = driver(host.clone());
    let mut evidence = LifecycleEvidence::default();

    driver
        .run_review_and_final_gates(&serde_json::json!({ "items": [] }), &mut evidence)
        .await
        .expect("review gates run to a report");

    assert_eq!(host.count_starting_with("cross-cutting-review-"), 1);
    assert!(
        host.call_ids()
            .contains(&"final-acceptance-report".to_string())
    );
}

//! Issue-107: the executed sequence may carry exactly one escalated round,
//! one past the unit's budget, and nothing else past it.
use super::*;

fn escalated(id: &str, stage: &str, round: u64) -> WorkflowV2HostCall {
    let mut call = remediation_call(id, stage, "TASK-EX-001", round, 2);
    set_remediation_field(
        &mut call,
        "escalation",
        serde_json::json!({"ownerTaskIds": ["TASK-EX-002"], "blockerPaths": ["src/b.rs"]}),
    );
    call
}

fn plan(calls: Vec<WorkflowV2HostCall>) -> Result<(), String> {
    validate_map_reduce_review_calls(&remediation_details(calls), &task_set(["TASK-EX-001"]))
}

#[test]
fn the_escalated_round_one_past_the_budget_is_accepted() {
    plan(vec![
        remediation_call(
            "review-remediate-task-ex-001-2-5",
            "remediate",
            "TASK-EX-001",
            2,
            2,
        ),
        remediation_call(
            "verification-wave-review-verify-task-ex-001-2-6",
            "verify",
            "TASK-EX-001",
            2,
            2,
        ),
        escalated("review-remediate-task-ex-001-esc-7", "remediate", 3),
        escalated(
            "verification-wave-review-verify-task-ex-001-esc-8",
            "verify",
            3,
        ),
    ])
    .expect("one escalated round is within bounds");
}

#[test]
fn an_unmarked_round_past_the_budget_and_an_escalation_elsewhere_are_rejected() {
    let error = plan(vec![
        remediation_call(
            "review-remediate-task-ex-001-3-5",
            "remediate",
            "TASK-EX-001",
            3,
            2,
        ),
        remediation_call(
            "verification-wave-review-verify-task-ex-001-3-6",
            "verify",
            "TASK-EX-001",
            3,
            2,
        ),
    ])
    .expect_err("an unmarked third round is out of bounds");
    assert!(error.contains("outside its own bound"), "{error}");
    for round in [2, 4] {
        let error = plan(vec![
            escalated("review-remediate-task-ex-001-esc-7", "remediate", round),
            escalated(
                "verification-wave-review-verify-task-ex-001-esc-8",
                "verify",
                round,
            ),
        ])
        .expect_err("an escalation is exactly one past the budget");
        assert!(error.contains("outside its own bound"), "{error}");
    }
}

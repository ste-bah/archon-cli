//! The authored run's terminal rule: status from the final accounting, hard
//! stops unchanged.

use super::*;
use crate::v2::acceptance_stage::ACCEPTANCE_STAGE_TOOL;

fn gate(failing: &[&str]) -> AuthoredAcceptanceGateV1 {
    AuthoredAcceptanceGateV1 {
        final_round: 2,
        attempt: 1,
        record_path: "v2/acceptance/round-02/attempt-01.json".into(),
        contract_present: true,
        failing_check_ids: failing.iter().map(|id| id.to_string()).collect(),
        unowned_failing_check_ids: Vec::new(),
        operational_errors: Vec::new(),
    }
}

/// The fact for a gate whose record belongs to the last round this run ran.
fn recorded(gate: &AuthoredAcceptanceGateV1) -> AuthoredAcceptanceGateFact<'_> {
    AuthoredAcceptanceGateFact::Recorded {
        gate,
        record_call_id: "acceptance-contract-run-2",
        last_call_id: "acceptance-contract-run-2",
        last_call_status: Some(if gate.blocks_completion() {
            WorkflowV2Status::NeedsReview
        } else {
            WorkflowV2Status::Accepted
        }),
    }
}

/// Two tasks accepted; the adversarial review found one issue on TASK-A that
/// review remediation resolved.
fn accounting(remediation: serde_json::Value, blocked: serde_json::Value) -> String {
    serde_json::json!({
        "accepted": ["TASK-A", "TASK-B"],
        "blocked": blocked,
        "adversarial_findings": [{ "id": "f1", "canonical_task_ids": ["TASK-A"], "severity": "high" }],
        "uncovered_requirements": [],
        "review_remediation": remediation,
        "acceptance_gate": { "complete": true },
    })
    .to_string()
}

fn resolved_a() -> serde_json::Value {
    serde_json::json!({ "resolved": [{ "taskId": "TASK-A", "findingCount": 1 }], "unresolved": [], "unassigned": [] })
}

fn verified(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

fn decide(
    accumulated: WorkflowV2Status,
    failed_call: Option<&str>,
    result: Option<&str>,
    gate: AuthoredAcceptanceGateFact<'_>,
    verified: &BTreeSet<String>,
) -> AuthoredRunOutcome {
    authored_run_terminal_status(&AuthoredRunFacts {
        accumulated_status: accumulated,
        host_terminal_failure: failed_call,
        script_result: result,
        acceptance_gate: gate,
        verified_remediation_tasks: verified,
    })
}

#[test]
fn a_finding_review_call_does_not_pin_a_run_whose_accounting_closed() {
    // The accumulator saw a needs_review review map call; everything the run
    // ended with is closed and the host's acceptance round passed.
    let passed = gate(&[]);
    let result = accounting(resolved_a(), serde_json::json!([]));
    let outcome = decide(
        WorkflowV2Status::NeedsReview,
        None,
        Some(&result),
        recorded(&passed),
        &verified(&["TASK-A"]),
    );
    assert_eq!(outcome.status, WorkflowV2Status::Accepted, "{outcome:?}");
    assert!(outcome.from_accounting);
    assert!(outcome.blocking.is_empty());
    assert!(outcome.explanation().contains("acceptance round 2 passed"));
}

#[test]
fn an_unverified_remediation_holds_the_run_for_review() {
    let passed = gate(&[]);
    let remediation = serde_json::json!({
        "resolved": [],
        "unresolved": [{ "taskId": "TASK-A", "findingCount": 1, "outcome": "unverified", "reason": "verifier rejected" }],
        "unassigned": [],
    });
    let result = accounting(remediation, serde_json::json!([]));
    let outcome = decide(
        WorkflowV2Status::Accepted,
        None,
        Some(&result),
        recorded(&passed),
        &verified(&[]),
    );
    assert_eq!(outcome.status, WorkflowV2Status::NeedsReview);
    assert!(
        outcome
            .explanation()
            .contains("TASK-A review remediation is unverified")
    );
}

#[test]
fn only_not_task_actionable_and_unassigned_findings_do_not_block() {
    let passed = gate(&[]);
    let remediation = serde_json::json!({
        "resolved": [],
        "unresolved": [{ "taskId": "TASK-A", "findingCount": 1, "outcome": "not_task_actionable", "reason": "no writable target" }],
        "unassigned": [{ "id": "prd-level" }],
    });
    let result = accounting(remediation, serde_json::json!([]));
    let outcome = decide(
        WorkflowV2Status::NeedsReview,
        None,
        Some(&result),
        recorded(&passed),
        &verified(&[]),
    );
    assert_eq!(outcome.status, WorkflowV2Status::Accepted, "{outcome:?}");
    let line = outcome.explanation();
    assert!(line.contains("not task-actionable") && line.contains("1 finding(s) name no task"));
}

#[test]
fn a_failing_or_missing_acceptance_round_holds_the_run() {
    let result = accounting(resolved_a(), serde_json::json!([]));
    let failing = gate(&["AC-1"]);
    let outcome = decide(
        WorkflowV2Status::Accepted,
        None,
        Some(&result),
        recorded(&failing),
        &verified(&["TASK-A"]),
    );
    assert_eq!(outcome.status, WorkflowV2Status::NeedsReview);
    assert!(outcome.explanation().contains("failing checks: AC-1"));
    let missing = decide(
        WorkflowV2Status::Accepted,
        None,
        Some(&result),
        AuthoredAcceptanceGateFact::Missing,
        &verified(&["TASK-A"]),
    );
    assert_eq!(missing.status, WorkflowV2Status::NeedsReview);
}

#[test]
fn hard_stops_keep_the_accumulated_status() {
    let result = accounting(resolved_a(), serde_json::json!([]));
    let passed = gate(&[]);
    let recorded = recorded(&passed);
    let tasks = verified(&["TASK-A"]);
    // A genuine failure the host recorded as terminal (gate, script error,
    // repository audit) stays Failed whatever the accounting says.
    let failed = decide(
        WorkflowV2Status::Failed,
        Some("repository-audit-final"),
        Some(&result),
        recorded,
        &tasks,
    );
    assert_eq!(failed.status, WorkflowV2Status::Failed);
    assert!(!failed.from_accounting);
    let stopped = decide(
        WorkflowV2Status::Failed,
        Some("workflow.js"),
        None,
        recorded,
        &tasks,
    );
    assert_eq!(stopped.status, WorkflowV2Status::Failed);
    let cancelled = decide(
        WorkflowV2Status::Cancelled,
        None,
        Some(&result),
        recorded,
        &tasks,
    );
    assert_eq!(cancelled.status, WorkflowV2Status::Cancelled);
}

#[test]
fn a_blocked_task_holds_the_run_unless_remediation_verifiably_resolved_it() {
    let passed = gate(&[]);
    let recorded = recorded(&passed);
    let blocked = serde_json::json!([{ "taskId": "TASK-B", "reason": "verifier rejected twice" }]);
    let result = accounting(resolved_a(), blocked.clone());
    let outcome = decide(
        WorkflowV2Status::Accepted,
        None,
        Some(&result),
        recorded,
        &verified(&["TASK-A"]),
    );
    assert_eq!(outcome.status, WorkflowV2Status::NeedsReview);
    assert!(outcome.explanation().contains("task TASK-B is blocked"));
    // The prelude folds blocked tasks into review remediation; a host-backed
    // resolution there finishes the task.
    let remediation = serde_json::json!({
        "resolved": [{ "taskId": "TASK-A" }, { "taskId": "TASK-B" }],
        "unresolved": [], "unassigned": [],
    });
    let result = accounting(remediation, blocked);
    let cleared = decide(
        WorkflowV2Status::NeedsReview,
        None,
        Some(&result),
        recorded,
        &verified(&["TASK-A", "TASK-B"]),
    );
    assert_eq!(cleared.status, WorkflowV2Status::Accepted, "{cleared:?}");
}

#[test]
fn a_resolution_without_a_host_verify_record_does_not_count() {
    let passed = gate(&[]);
    let result = accounting(resolved_a(), serde_json::json!([]));
    let outcome = decide(
        WorkflowV2Status::Accepted,
        None,
        Some(&result),
        recorded(&passed),
        &verified(&[]),
    );
    assert_eq!(outcome.status, WorkflowV2Status::NeedsReview);
    assert!(
        outcome
            .explanation()
            .contains("no accepted remediation verify call")
    );
}

#[test]
fn a_finding_task_with_no_remediation_outcome_holds_the_run() {
    let passed = gate(&[]);
    let remediation = serde_json::json!({ "resolved": [], "unresolved": [], "unassigned": [] });
    let result = accounting(remediation, serde_json::json!([]));
    let outcome = decide(
        WorkflowV2Status::Accepted,
        None,
        Some(&result),
        recorded(&passed),
        &verified(&[]),
    );
    assert_eq!(outcome.status, WorkflowV2Status::NeedsReview);
    assert!(outcome.explanation().contains("reports no outcome for it"));
}

#[test]
fn an_open_item_that_failed_on_transport_is_blocked_not_needs_review() {
    let passed = gate(&[]);
    let blocked =
        serde_json::json!([{ "taskId": "TASK-B", "reason": "agent transport failed: 520" }]);
    let result = accounting(resolved_a(), blocked);
    let outcome = decide(
        WorkflowV2Status::Failed,
        None,
        Some(&result),
        recorded(&passed),
        &verified(&["TASK-A"]),
    );
    assert_eq!(outcome.status, WorkflowV2Status::Blocked);
}

fn call(id: &str, extra: serde_json::Value) -> WorkflowV2HostCall {
    let mut options = WorkflowV2HostOptions::default();
    if let serde_json::Value::Object(map) = extra {
        options.extra = map.into_iter().collect();
    }
    WorkflowV2HostCall {
        id: id.into(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options,
    }
}

#[test]
fn verified_tasks_come_from_accepted_verify_records_before_acceptance() {
    let verify = |id: &str, task: &str| {
        call(
            id,
            serde_json::json!({ "remediationContract": { "stage": "verify", "taskId": task } }),
        )
    };
    let mut acceptance = call(
        "acceptance-contract-run-1",
        serde_json::json!({ "tool": ACCEPTANCE_STAGE_TOOL }),
    );
    acceptance.method = WorkflowV2HostMethod::Tool;
    let calls = vec![
        call(
            "review-remediate-a-1",
            serde_json::json!({ "remediationContract": { "stage": "remediate", "taskId": "TASK-A" } }),
        ),
        verify("review-verify-a-1", "TASK-A"),
        verify("review-verify-b-1", "TASK-B"),
        acceptance,
        // Acceptance-stage remediation is judged by its round record.
        verify("review-verify-c-1", "TASK-C"),
    ];
    let tasks = review_remediation_verified_tasks(&calls, |id| {
        Ok(Some(match id {
            "review-verify-b-1" => WorkflowV2Status::NeedsReview,
            _ => WorkflowV2Status::Accepted,
        }))
    })
    .expect("statuses");
    assert_eq!(tasks, verified(&["TASK-A"]));
}

#[test]
fn a_passing_record_from_a_round_this_run_never_reached_does_not_pass_the_gate() {
    // An earlier process recorded a clean round 2; this process's last
    // executed acceptance call was a re-run round 1 that did not pass.
    let result = accounting(resolved_a(), serde_json::json!([]));
    let stale = gate(&[]);
    let outcome = decide(
        WorkflowV2Status::Accepted,
        None,
        Some(&result),
        AuthoredAcceptanceGateFact::Recorded {
            gate: &stale,
            record_call_id: "acceptance-contract-run-2",
            last_call_id: "acceptance-contract-run-1",
            last_call_status: Some(WorkflowV2Status::NeedsReview),
        },
        &verified(&["TASK-A"]),
    );
    assert_eq!(outcome.status, WorkflowV2Status::NeedsReview);
    assert!(
        outcome
            .explanation()
            .contains("not to `acceptance-contract-run-1`")
    );
    // Same record, but the host's own call record did not accept.
    let outcome = decide(
        WorkflowV2Status::Accepted,
        None,
        Some(&result),
        AuthoredAcceptanceGateFact::Recorded {
            gate: &stale,
            record_call_id: "acceptance-contract-run-2",
            last_call_id: "acceptance-contract-run-2",
            last_call_status: None,
        },
        &verified(&["TASK-A"]),
    );
    assert_eq!(outcome.status, WorkflowV2Status::NeedsReview);
}

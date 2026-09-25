//! The authored run's terminal rule over host call facts. Each defect the
//! hostile review named has a test that fails on a rule which trusts the
//! script's lists.

use std::collections::BTreeMap;

use super::*;

const A: &str = "TASK-A";
const B: &str = "TASK-B";

fn outcome(status: WorkflowV2Status) -> AuthoredTaskOutcome {
    AuthoredTaskOutcome {
        status,
        transport: false,
    }
}

fn fact(
    id: &str,
    role: AuthoredCallRole,
    status: WorkflowV2Status,
    tasks: &[(&str, WorkflowV2Status)],
) -> AuthoredCallFact {
    AuthoredCallFact {
        id: id.into(),
        role,
        status: Some(status),
        transport: false,
        tasks: tasks
            .iter()
            .map(|(task, status)| (task.to_string(), outcome(*status)))
            .collect::<BTreeMap<_, _>>(),
        record_path: None,
    }
}

use WorkflowV2Status::{Accepted, Failed, NeedsReview, Noop};

fn review(id: &str, kind: &str, stage: &str, status: WorkflowV2Status) -> AuthoredCallFact {
    let role = AuthoredCallRole::Review {
        kind: kind.into(),
        stage: stage.into(),
    };
    fact(id, role, status, &[(A, Accepted), (B, Accepted)])
}

fn fix(task: &str, round: u64, status: WorkflowV2Status) -> AuthoredCallFact {
    let role = AuthoredCallRole::RemediationFix {
        task: task.into(),
        round,
    };
    fact(
        &format!("fix-{task}-{round}"),
        role,
        status,
        &[(task, status)],
    )
}

fn rverify(task: &str, round: u64, status: WorkflowV2Status, agent: bool) -> AuthoredCallFact {
    let role = AuthoredCallRole::RemediationVerify {
        task: task.into(),
        round,
        agent,
    };
    fact(
        &format!("rverify-{task}-{round}-{agent}"),
        role,
        status,
        &[(task, status)],
    )
}

/// The clean shape: both tasks written (B as a typed no-op) and verified,
/// both reviews run (the adversarial map found something: `needs_review`),
/// TASK-A's finding remediated and re-verified, acceptance reached.
fn clean_calls() -> Vec<AuthoredCallFact> {
    vec![
        fact(
            "agents-1",
            AuthoredCallRole::Write,
            Accepted,
            &[(A, Accepted), (B, Noop)],
        ),
        fact(
            "verify-a",
            AuthoredCallRole::TaskVerify,
            Accepted,
            &[(A, Accepted)],
        ),
        fact(
            "verify-b",
            AuthoredCallRole::TaskVerify,
            Accepted,
            &[(B, Accepted)],
        ),
        review("adv-map", "adversarial_findings", "map", NeedsReview),
        review(
            "adv-reduce",
            "adversarial_findings",
            "reduce_final",
            NeedsReview,
        ),
        review("cov-map", "uncovered_requirements", "map", Accepted),
        review(
            "cov-reduce",
            "uncovered_requirements",
            "reduce_final",
            Accepted,
        ),
        fix(A, 1, Accepted),
        rverify(A, 1, Accepted, true),
        fact(
            "acceptance-contract-run-1",
            AuthoredCallRole::Acceptance,
            Accepted,
            &[],
        ),
    ]
}

fn accounting(patch: serde_json::Value) -> String {
    let mut base = serde_json::json!({
        "accepted": [A, B],
        "blocked": [],
        "adversarial_findings": [{ "id": "f1", "canonical_task_ids": [A], "severity": "high" }],
        "uncovered_requirements": [],
        "review_remediation": { "resolved": [{ "taskId": A }], "unresolved": [], "unassigned": [] },
    });
    if let serde_json::Value::Object(patch) = patch {
        for (key, value) in patch {
            base[key] = value;
        }
    }
    base.to_string()
}

fn passing_gate() -> AuthoredAcceptanceGateV1 {
    AuthoredAcceptanceGateV1 {
        final_round: 1,
        attempt: 1,
        record_path: "v2/acceptance/round-01/attempt-01.json".into(),
        contract_present: true,
        failing_check_ids: Vec::new(),
        unowned_failing_check_ids: Vec::new(),
        operational_errors: Vec::new(),
    }
}

struct Case {
    calls: Vec<AuthoredCallFact>,
    result: String,
    writable: BTreeSet<String>,
    gate: AuthoredAcceptanceGateV1,
    record_call_id: &'static str,
    accumulated: WorkflowV2Status,
    failed_call: Option<&'static str>,
}

impl Case {
    fn clean() -> Self {
        Self {
            calls: clean_calls(),
            result: accounting(serde_json::json!({})),
            writable: [A, B].iter().map(|id| id.to_string()).collect(),
            gate: passing_gate(),
            record_call_id: "acceptance-contract-run-1",
            accumulated: NeedsReview,
            failed_call: None,
        }
    }

    fn decide(&self) -> AuthoredRunOutcome {
        authored_run_terminal_status(&AuthoredRunFacts {
            accumulated_status: self.accumulated,
            host_terminal_failure: self.failed_call,
            script_result: Some(&self.result),
            acceptance_gate: AuthoredAcceptanceGateFact::Recorded {
                gate: &self.gate,
                record_call_id: self.record_call_id,
                last_call_id: "acceptance-contract-run-1",
                last_call_status: Some(Accepted),
            },
            calls: &self.calls,
            writable_tasks: &self.writable,
        })
    }

    fn holds(&self, expected: &str) {
        let outcome = self.decide();
        assert_eq!(outcome.status, NeedsReview, "{}", outcome.explanation());
        assert!(
            outcome.explanation().contains(expected),
            "{}",
            outcome.explanation()
        );
    }

    fn replace(&mut self, id: &str, with: AuthoredCallFact) {
        let at = self
            .calls
            .iter()
            .position(|call| call.id == id)
            .expect("call");
        self.calls[at] = with;
    }
}

#[test]
fn a_run_whose_host_records_close_is_accepted_despite_a_finding_review_call() {
    let outcome = Case::clean().decide();
    assert_eq!(outcome.status, Accepted, "{}", outcome.explanation());
    assert!(outcome.from_accounting);
    assert!(outcome.explanation().contains("acceptance round 1 passed"));
}

// 1: `accepted` is a claim the host records must back.
#[test]
fn an_accepted_task_needs_an_accepted_write_and_a_later_accepted_verify() {
    let mut case = Case::clean();
    case.replace(
        "verify-b",
        fact(
            "verify-b",
            AuthoredCallRole::TaskVerify,
            NeedsReview,
            &[(B, NeedsReview)],
        ),
    );
    case.holds("TASK-B is reported accepted but its latest verify `verify-b` is NeedsReview");

    let mut case = Case::clean();
    case.calls.retain(|call| call.id != "verify-b");
    case.holds("no host verify record names it");

    // Per-task, not wave-level: B's branch of the shared write failed.
    let mut case = Case::clean();
    case.replace(
        "agents-1",
        fact(
            "agents-1",
            AuthoredCallRole::Write,
            NeedsReview,
            &[(A, Accepted), (B, Failed)],
        ),
    );
    case.holds("TASK-B is reported accepted but its latest write `agents-1` is Failed");

    // A write after the last verify is unverified work.
    let mut case = Case::clean();
    case.calls.insert(
        3,
        fact(
            "remediate-b",
            AuthoredCallRole::Write,
            Accepted,
            &[(B, Accepted)],
        ),
    );
    case.holds("nothing verified it after its latest write `remediate-b`");

    let mut case = Case::clean();
    case.result = accounting(serde_json::json!({ "accepted": [A, B, "TASK-C"] }));
    case.holds("TASK-C is reported accepted but no host write record names it");
}

#[test]
fn a_later_accepted_verify_supersedes_an_earlier_rejection() {
    let mut case = Case::clean();
    case.calls.insert(
        2,
        fact(
            "verify-b-0",
            AuthoredCallRole::TaskVerify,
            NeedsReview,
            &[(B, NeedsReview)],
        ),
    );
    assert_eq!(case.decide().status, Accepted);
}

// 2: a review call that did not run is no review.
#[test]
fn a_failed_or_missing_review_call_or_branch_holds_the_run() {
    let mut case = Case::clean();
    case.replace(
        "cov-map",
        review("cov-map", "uncovered_requirements", "map", Failed),
    );
    case.holds("review call `cov-map` (uncovered_requirements map) is Failed");

    let mut case = Case::clean();
    let mut reduce = review("adv-reduce", "adversarial_findings", "reduce_final", Failed);
    reduce.transport = true;
    case.replace("adv-reduce", reduce);
    assert_eq!(case.decide().status, WorkflowV2Status::Blocked);

    let mut case = Case::clean();
    let mut map = review("adv-map", "adversarial_findings", "map", NeedsReview);
    map.tasks.insert(B.into(), outcome(Failed));
    case.replace("adv-map", map);
    case.holds("review call `adv-map` did not review task TASK-B");

    let mut case = Case::clean();
    case.calls[5].status = None;
    case.holds("review call `cov-map` (uncovered_requirements map) has no host record");
}

// 3: not_task_actionable only for a task with nothing it may write.
#[test]
fn not_task_actionable_stands_only_for_a_task_without_writable_files() {
    let mut case = Case::clean();
    case.result = accounting(serde_json::json!({
        "review_remediation": { "resolved": [], "unresolved": [{ "taskId": A, "outcome": "not_task_actionable" }], "unassigned": [] },
    }));
    case.holds("task TASK-A is reported not_task_actionable, but the task universe declares writable files");
    case.writable.remove(A);
    assert_eq!(
        case.decide().status,
        Accepted,
        "{}",
        case.decide().explanation()
    );
}

// 4: findings naming no task.
#[test]
fn unassigned_findings_block_by_kind_and_severity() {
    let with_findings = |adversarial: serde_json::Value, uncovered: serde_json::Value| {
        let mut case = Case::clean();
        case.result = accounting(serde_json::json!({
            "adversarial_findings": adversarial,
            "uncovered_requirements": uncovered,
            "review_remediation": { "resolved": [], "unresolved": [], "unassigned": [] },
        }));
        case
    };
    with_findings(
        serde_json::json!([{ "id": "x", "severity": "Critical" }]),
        serde_json::json!([]),
    )
    .holds("critical finding `x` names no task");
    with_findings(serde_json::json!([]), serde_json::json!(["REQ-9"]))
        .holds("uncovered requirement `REQ-9` names no task");
    with_findings(
        serde_json::json!([{ "id": "u", "review_outcome": "unreviewed", "attributable_to_task": false }]),
        serde_json::json!([]),
    )
    .holds("the host recorded `u` as unreviewed");
    let low = with_findings(
        serde_json::json!([{ "id": "note", "severity": "low" }]),
        serde_json::json!([]),
    );
    let outcome = low.decide();
    assert_eq!(outcome.status, Accepted, "{}", outcome.explanation());
    assert!(
        outcome
            .explanation()
            .contains("1 non-blocking finding(s) name no task: `note`")
    );
}

#[test]
fn finding_attribution_reads_task_ids_the_way_the_host_does() {
    // An empty `canonical_task_ids` falls through to `task_ids`, as
    // `review_findings::task_ids_of` reads it.
    let mut case = Case::clean();
    case.result = accounting(serde_json::json!({
        "adversarial_findings": [{ "canonical_task_ids": [], "task_ids": [B] }],
        "review_remediation": { "resolved": [], "unresolved": [], "unassigned": [] },
    }));
    case.holds("review findings name task TASK-B but review remediation reports no outcome");
}

// 5: resolved needs the LAST round's fix and a real verifier, both accepted.
#[test]
fn a_resolution_needs_the_last_rounds_accepted_fix_and_verifier_agent() {
    let mut case = Case::clean();
    case.replace("rverify-TASK-A-1-true", rverify(A, 1, Accepted, false));
    case.holds("is a no-patch checkpoint, not a verifier");

    // A later rejected round overrides the accepted one.
    let mut case = Case::clean();
    case.calls.insert(9, fix(A, 2, Accepted));
    case.calls.insert(10, rverify(A, 2, NeedsReview, true));
    case.holds("its last round's `rverify-TASK-A-2-true` is NeedsReview");

    let mut case = Case::clean();
    case.replace("fix-TASK-A-1", fix(A, 1, Failed));
    case.holds("its last round's `fix-TASK-A-1` is Failed");

    let mut case = Case::clean();
    case.calls.insert(9, fix(A, 2, Accepted));
    case.holds("was not verified by");
}

#[test]
fn a_blocked_task_holds_the_run_unless_remediation_verifiably_finished_it() {
    let mut case = Case::clean();
    case.result = accounting(serde_json::json!({
        "accepted": [A],
        "blocked": [{ "taskId": B, "reason": "verifier rejected twice" }],
    }));
    case.holds("task TASK-B is blocked");
    case.result = accounting(serde_json::json!({
        "accepted": [A],
        "blocked": [{ "taskId": B, "reason": "verifier rejected twice" }],
        "review_remediation": { "resolved": [{ "taskId": A }, { "taskId": B }], "unresolved": [], "unassigned": [] },
    }));
    case.calls.insert(9, fix(B, 1, Accepted));
    case.calls.insert(10, rverify(B, 1, Accepted, true));
    assert_eq!(
        case.decide().status,
        Accepted,
        "{}",
        case.decide().explanation()
    );
    let mut transport = Case::clean();
    transport.result = accounting(serde_json::json!({
        "accepted": [A],
        "blocked": [{ "taskId": B, "reason": "agent transport failed: 520" }],
    }));
    assert_eq!(transport.decide().status, WorkflowV2Status::Blocked);
}

// 7: honest outcomes stay open.
#[test]
fn refuted_and_unverified_outcomes_hold_the_run() {
    for outcome in ["refuted", "unverified", "failed"] {
        let mut case = Case::clean();
        case.result = accounting(serde_json::json!({
            "review_remediation": { "resolved": [], "unresolved": [{ "taskId": A, "outcome": outcome, "reason": "r" }], "unassigned": [] },
        }));
        case.holds(&format!("task TASK-A review remediation is {outcome}"));
    }
}

// 8 and the gate itself.
#[test]
fn the_gate_must_pass_on_the_record_bound_to_the_last_acceptance_call() {
    let mut case = Case::clean();
    case.record_call_id = "acceptance-contract-run-2";
    case.holds("the acceptance record belongs to `acceptance-contract-run-2`");
    let mut case = Case::clean();
    case.gate.failing_check_ids = vec!["AC-1".into()];
    case.holds("acceptance round 1 has failing checks: AC-1");
}

#[test]
fn hard_stops_keep_the_accumulated_status() {
    let mut case = Case::clean();
    case.accumulated = Failed;
    case.failed_call = Some("repository-audit-final");
    let outcome = case.decide();
    assert_eq!(outcome.status, Failed);
    assert!(!outcome.from_accounting);
    case.failed_call = None;
    case.accumulated = WorkflowV2Status::Cancelled;
    assert_eq!(case.decide().status, WorkflowV2Status::Cancelled);
    let stopped = authored_run_terminal_status(&AuthoredRunFacts {
        accumulated_status: Failed,
        host_terminal_failure: Some("workflow.js"),
        script_result: None,
        acceptance_gate: AuthoredAcceptanceGateFact::Missing,
        calls: &[],
        writable_tasks: &BTreeSet::new(),
    });
    assert_eq!(stopped.status, Failed);
}

/// The exemption must cost more than it grants: a forged contract naming a
/// reduce that does not exist buys nothing.
#[test]
fn remediation_naming_a_nonexistent_reduce_is_rejected() {
    let mut call = remediation_call("sneak-work-in", "remediate", "TASK-EX-001", 1, 2);
    set_remediation_field(
        &mut call,
        "sourceReduceCallIds",
        serde_json::json!(["no-such-reduce"]),
    );
    let details = remediation_details(vec![
        call,
        remediation_call(
            "verification-wave-review-verify-task-ex-001-1",
            "verify",
            "TASK-EX-001",
            1,
            2,
        ),
    ]);
    let error = validate_map_reduce_review_calls(&details, &task_set(["TASK-EX-001"]))
        .expect_err("forged source must be rejected");
    assert!(
        error.contains("no final review reduce with that id"),
        "{error}"
    );
}

#[test]
fn remediation_write_call_marked_verify_is_rejected() {
    let mut verify = remediation_call(
        "verification-wave-review-verify-task-ex-001-1",
        "verify",
        "TASK-EX-001",
        1,
        2,
    );
    verify.write_mode = Some(WorkflowV2WriteMode::Worktree);
    let details = remediation_details(vec![
        remediation_call(
            "review-remediate-task-ex-001-1",
            "remediate",
            "TASK-EX-001",
            1,
            2,
        ),
        verify,
    ]);

    let error = validate_map_reduce_review_calls(&details, &task_set(["TASK-EX-001"]))
        .expect_err("write-capable remediation verification must be rejected");
    assert!(error.contains("must be read-only"), "{error}");
}

/// A fix nothing re-checks is exactly the unreviewed work the ordering rule
/// exists to prevent.
#[test]
fn remediation_without_a_following_verifier_is_rejected() {
    let details = remediation_details(vec![remediation_call(
        "review-remediate-task-ex-001-1",
        "remediate",
        "TASK-EX-001",
        1,
        2,
    )]);
    let error = validate_map_reduce_review_calls(&details, &task_set(["TASK-EX-001"]))
        .expect_err("unverified remediation must be rejected");
    assert!(error.contains("no later remediation verifier"), "{error}");
}

/// Bounded means bounded: the pass closes findings, it does not grind a task
/// until something reports green.
#[test]
fn remediation_beyond_the_round_ceiling_is_rejected() {
    let details = remediation_details(vec![
        remediation_call(
            "review-remediate-task-ex-001-9",
            "remediate",
            "TASK-EX-001",
            9,
            9,
        ),
        remediation_call(
            "verification-wave-review-verify-task-ex-001-9",
            "verify",
            "TASK-EX-001",
            9,
            9,
        ),
    ]);
    let error = validate_map_reduce_review_calls(&details, &task_set(["TASK-EX-001"]))
        .expect_err("unbounded remediation must be rejected");
    assert!(error.contains("maxRounds"), "{error}");
}

/// Ordinary task work after review is still forbidden — the exemption is keyed
/// to the contract, not to position.
#[test]
fn plain_task_work_after_review_is_still_rejected() {
    let mut details = remediation_details(vec![]);
    details.calls.push(work_call("verify-task-late"));
    let error = validate_map_reduce_review_calls(&details, &task_set(["TASK-EX-001"]))
        .expect_err("late plain work still rejected");
    assert!(error.contains("BEFORE task work"), "{error}");
}

/// A complete, valid mandatory-review plan, plus whatever remediation calls the
/// test wants to append after the reduces.
fn remediation_details(extra: Vec<WorkflowV2HostCall>) -> WorkflowDryRunPlanDetails {
    let mut details = review_details(
        vec![work_call("implement-task-1")],
        vec![
            review_map_claim(
                "adversarial_findings",
                "adversarial-review-map",
                "TASK-EX-001",
            ),
            review_map_claim(
                "uncovered_requirements",
                "coverage-audit-map",
                "TASK-EX-001",
            ),
        ],
        vec![
            review_reduce(
                "adversarial_findings",
                "adversarial-review-reduce",
                "adversarial_findings",
                ["adversarial-review-map"],
                [],
            ),
            review_reduce(
                "uncovered_requirements",
                "coverage-audit-reduce",
                "uncovered_requirements",
                ["coverage-audit-map"],
                [],
            ),
        ],
    );
    details.calls.extend(extra);
    details
}

fn remediation_call(
    id: &str,
    stage: &str,
    task_id: &str,
    round: u64,
    max_rounds: u64,
) -> WorkflowV2HostCall {
    let mut call = WorkflowV2HostCall {
        id: id.to_string(),
        method: WorkflowV2HostMethod::Fanout,
        write_mode: if stage == "remediate" {
            Some(WorkflowV2WriteMode::Worktree)
        } else {
            None
        },
        options: WorkflowV2HostOptions::default(),
    };
    call.options.extra.insert(
        "remediationContract".to_string(),
        serde_json::json!({
            "version": 1,
            "stage": stage,
            "taskId": task_id,
            "round": round,
            "maxRounds": max_rounds,
            "sourceReduceCallIds": ["adversarial-review-reduce", "coverage-audit-reduce"],
        }),
    );
    call
}

fn set_remediation_field(call: &mut WorkflowV2HostCall, key: &str, value: serde_json::Value) {
    if let Some(contract) = call
        .options
        .extra
        .get_mut("remediationContract")
        .and_then(serde_json::Value::as_object_mut)
    {
        contract.insert(key.to_string(), value);
    }
}

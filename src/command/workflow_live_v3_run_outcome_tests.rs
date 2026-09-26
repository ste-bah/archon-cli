//! The authored run's terminal rule through the real stores: host call
//! records and acceptance round records on disk, then the shared finalizer.

use super::*;
use archon_workflow::v2::acceptance_stage::ACCEPTANCE_STAGE_TOOL;

const TASK: &str = "TASK-G-001";

fn host_call(
    id: &str,
    method: WorkflowV2HostMethod,
    extra: serde_json::Value,
) -> WorkflowV2HostCall {
    let mut call = call();
    call.id = id.into();
    call.method = method;
    if let serde_json::Value::Object(map) = extra {
        call.options.extra = map.into_iter().collect();
    }
    call
}

fn record_call(
    run: &Run,
    call: &WorkflowV2HostCall,
    status: WorkflowV2Status,
    data: serde_json::Value,
) {
    let record = WorkflowV2CallRecord::new(
        run.v2_store.run_id(),
        call.clone(),
        1,
        "input".into(),
        WorkflowV2Result {
            status,
            summary: "recorded".into(),
            data,
            ..WorkflowV2Result::default()
        },
        Vec::new(),
    );
    run.v2_store.save_call_record(&record).unwrap();
}

fn outcome(status: &str) -> serde_json::Value {
    serde_json::json!({ "outcomes": [{
        "item_id": "i", "canonical_task_ids": [TASK], "status": status,
        "result": { "status": status, "summary": "branch" },
    }] })
}

/// A run whose adversarial review found an issue on TASK-G-001 (a
/// `needs_review` map call), remediated and re-verified it, and passed its
/// acceptance round. Returns the run and the summary a live process leaves.
fn closed_run(task_verify: &str, acceptance_round: u32) -> (Run, WorkflowV2ScriptSummary) {
    let run = run();
    let mut calls = Vec::new();
    let mut add = |call: WorkflowV2HostCall, status: WorkflowV2Status, data: serde_json::Value| {
        record_call(&run, &call, status, data);
        calls.push(call);
    };
    let mut write = host_call(
        "agents-1",
        WorkflowV2HostMethod::Fanout,
        serde_json::json!({}),
    );
    write.write_mode = Some(archon_workflow::WorkflowV2WriteMode::Worktree);
    write.options.item_kind = Some("implementation".into());
    add(write, WorkflowV2Status::Accepted, outcome("accepted"));
    let mut verify = host_call(
        "verification-wave-verify-task-g-001-2",
        WorkflowV2HostMethod::Parallel,
        serde_json::json!({}),
    );
    verify.options.item_kind = Some("focused_verification".into());
    let verify_status = serde_json::from_value(serde_json::json!(task_verify)).unwrap();
    add(verify, verify_status, outcome(task_verify));
    for (kind, stage) in [
        ("adversarial_findings", "map"),
        ("adversarial_findings", "reduce_final"),
        ("uncovered_requirements", "map"),
        ("uncovered_requirements", "reduce_final"),
    ] {
        let review = host_call(
            &format!("{kind}-{stage}"),
            WorkflowV2HostMethod::Parallel,
            serde_json::json!({ "reviewContract": { "kind": kind, "stage": stage } }),
        );
        add(review, WorkflowV2Status::NeedsReview, outcome("accepted"));
    }
    let contract = |stage: &str| serde_json::json!({ "remediationContract": { "stage": stage, "taskId": TASK, "round": 1 } });
    let mut fix = host_call(
        "review-remediate-task-g-001-1-3",
        WorkflowV2HostMethod::Fanout,
        contract("remediate"),
    );
    fix.write_mode = Some(archon_workflow::WorkflowV2WriteMode::Worktree);
    add(fix, WorkflowV2Status::Accepted, outcome("accepted"));
    let rverify = host_call(
        "verification-wave-review-verify-task-g-001-1-4",
        WorkflowV2HostMethod::Parallel,
        contract("verify"),
    );
    add(rverify, WorkflowV2Status::Accepted, outcome("accepted"));
    record_round(
        &run,
        acceptance_round,
        vec![check("REQ-1", AcceptanceCheckStatus::Passed, &[TASK])],
        true,
    );
    let acceptance = host_call(
        &format!("acceptance-contract-run-{acceptance_round}"),
        WorkflowV2HostMethod::Tool,
        serde_json::json!({ "tool": ACCEPTANCE_STAGE_TOOL }),
    );
    add(
        acceptance,
        WorkflowV2Status::Accepted,
        serde_json::json!({ "record_path": format!("v2/acceptance/round-{acceptance_round:02}/attempt-01.json") }),
    );
    let mut summary = summary(WorkflowV2Status::NeedsReview);
    summary.calls = calls;
    summary.script_result = Some(
        serde_json::json!({
            "accepted": [TASK],
            "blocked": [],
            "adversarial_findings": [{ "id": "f1", "canonical_task_ids": [TASK], "severity": "high" }],
            "uncovered_requirements": [],
            "review_remediation": { "resolved": [{ "taskId": TASK, "findingCount": 1 }], "unresolved": [], "unassigned": [] },
        })
        .to_string(),
    );
    (run, summary)
}

fn decide(run: &Run, summary: WorkflowV2ScriptSummary) -> WorkflowV2ScriptSummary {
    let universe = archon_workflow::task_universe::WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![archon_workflow::task_universe::WorkflowV2TaskUniverseTask {
            canonical_task_id: TASK.into(),
            files_expected_to_change: vec!["src/g.rs".into()],
            ..Default::default()
        }],
    };
    apply_authored_run_outcome(
        &run.store,
        &run.run_id,
        &run.v2_store,
        Some(&universe),
        None,
        true,
        summary,
    )
    .unwrap()
}

async fn finalize_authored(run: &Run, summary: WorkflowV2ScriptSummary) -> WorkflowV2ScriptSummary {
    finalize_run(
        &run.store,
        &run.run_id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        None,
        summary,
        &run.v2_store,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn an_intermediate_needs_review_call_does_not_pin_a_closed_run() {
    let (run, summary) = closed_run("accepted", 1);
    let decided = decide(&run, summary.clone());
    assert_eq!(
        decided.status,
        WorkflowV2Status::Accepted,
        "{:?}",
        decided.next_action
    );
    // A resumed process replays the accepted calls, so its accumulator never
    // saw the needs_review call; the verdict must not depend on that history.
    let mut resumed = summary;
    resumed.status = WorkflowV2Status::Accepted;
    assert_eq!(decide(&run, resumed).status, decided.status);
    let events = std::fs::read_to_string(run.store.events_path(&run.run_id)).unwrap();
    assert!(
        events.contains("authored_run_outcome") && events.contains("acceptance round 1 passed"),
        "{events}"
    );
    let finalized = finalize_authored(&run, decided).await;
    assert_eq!(finalized.status, WorkflowV2Status::Accepted);
    assert_eq!(finalization(&run).terminal_status, RunStatus::Completed);
}

#[tokio::test]
async fn a_task_the_host_never_verified_cannot_finalize_complete() {
    let (run, summary) = closed_run("needs_review", 1);
    let decided = decide(&run, summary);
    assert_eq!(decided.status, WorkflowV2Status::NeedsReview);
    let next = decided.next_action.clone().expect("names the open item");
    assert!(
        next.contains("TASK-G-001 is reported accepted but its latest verify"),
        "{next}"
    );
    finalize_authored(&run, decided).await;
    assert_eq!(finalization(&run).terminal_status, RunStatus::NeedsReview);
}

/// An earlier process left a failing round-2 record; this run's last
/// acceptance call is round 1, whose own record names its clean round. The
/// stale record neither passes nor pins the run, and the finalization record
/// carries the bound round.
#[tokio::test]
async fn a_stale_higher_round_record_does_not_pin_the_run() {
    let (run, summary) = closed_run("accepted", 1);
    record_round(
        &run,
        2,
        vec![check("REQ-1", AcceptanceCheckStatus::Failed, &[TASK])],
        true,
    );
    let decided = decide(&run, summary);
    assert_eq!(
        decided.status,
        WorkflowV2Status::Accepted,
        "{:?}",
        decided.next_action
    );
    finalize_authored(&run, decided).await;
    let record = finalization(&run);
    assert_eq!(record.terminal_status, RunStatus::Completed);
    assert_eq!(record.acceptance_gate.expect("gate").final_round, 1);
}

#[test]
fn a_host_recorded_terminal_failure_is_not_overridden() {
    let (run, mut summary) = closed_run("accepted", 1);
    summary.status = WorkflowV2Status::Failed;
    summary.failed_call = Some("repository-audit-final".into());
    summary.next_action = Some("audit".into());
    let decided = decide(&run, summary);
    assert_eq!(decided.status, WorkflowV2Status::Failed);
    assert_eq!(decided.next_action.as_deref(), Some("audit"));
}

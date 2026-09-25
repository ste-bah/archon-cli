//! Contract of the authored lifecycle's terminal rule (Obs-32): a run whose
//! acceptance stage recorded a failing check cannot finalize as `Complete`.

use super::*;
use archon_workflow::v2::acceptance_stage::{
    ACCEPTANCE_ROUND_RECORD_SCHEMA_VERSION, AcceptanceCheckRecordV1, AcceptanceCheckStatus,
    AcceptanceRoundRecordV1, write_round_record,
};
use archon_workflow::{
    FinalizationRecordV1, RunStatus, WorkflowSpec, WorkflowV2CallRecord, WorkflowV2HostCall,
    WorkflowV2HostMethod, WorkflowV2Result,
};

use super::super::workflow_live_v2_finalizer::FINALIZATION_RECORD_PATH;

fn spec() -> WorkflowSpec {
    WorkflowSpec {
        schema: archon_workflow::spec::WORKFLOW_SCHEMA.into(),
        name: "acceptance-gate".into(),
        task: "prove the terminal rule".into(),
        target_repository_root: None,
        max_parallelism: 1,
        max_agents: 1,
        stages: vec![],
        permissions: Default::default(),
        learning_hooks: Vec::new(),
    }
}

fn call() -> WorkflowV2HostCall {
    WorkflowV2HostCall {
        id: "call-1".into(),
        method: WorkflowV2HostMethod::Agent,
        write_mode: None,
        options: Default::default(),
    }
}

fn summary(status: WorkflowV2Status) -> WorkflowV2ScriptSummary {
    WorkflowV2ScriptSummary {
        status,
        completed: 1,
        executed: 1,
        reused: 0,
        calls: vec![call()],
        failed_call: None,
        failed_result_path: None,
        next_action: None,
        script_result: None,
    }
}

struct Run {
    _temp: tempfile::TempDir,
    store: WorkflowStore,
    v2_store: WorkflowV2ResultStore,
    run_id: String,
}

fn run() -> Run {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::project(temp.path());
    let run = store.create_run(spec()).unwrap();
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let record = WorkflowV2CallRecord::new(
        v2_store.run_id(),
        call(),
        1,
        "input".into(),
        WorkflowV2Result {
            status: WorkflowV2Status::Accepted,
            summary: "terminal call".into(),
            ..WorkflowV2Result::default()
        },
        Vec::new(),
    );
    v2_store.save_call_record(&record).unwrap();
    Run {
        _temp: temp,
        store,
        v2_store,
        run_id: run.id,
    }
}

fn check(id: &str, status: AcceptanceCheckStatus, owners: &[&str]) -> AcceptanceCheckRecordV1 {
    AcceptanceCheckRecordV1 {
        check_id: id.into(),
        criterion: format!("criterion {id}"),
        kind: "command".into(),
        status,
        exit_code: Some(if status == AcceptanceCheckStatus::Passed {
            0
        } else {
            1
        }),
        operational_error: None,
        owning_tasks: owners.iter().map(|s| s.to_string()).collect(),
        stdout_tail: String::new(),
        stderr_tail: String::new(),
    }
}

fn record_round(run: &Run, round: u32, checks: Vec<AcceptanceCheckRecordV1>, final_round: bool) {
    let record = AcceptanceRoundRecordV1 {
        schema_version: ACCEPTANCE_ROUND_RECORD_SCHEMA_VERSION,
        run_id: run.run_id.clone(),
        call_id: format!("acceptance-contract-run-{round}"),
        round,
        attempt: 1,
        max_rounds: 3,
        contract_present: true,
        requested_check_ids: Vec::new(),
        execution: None,
        checks,
        operational_errors: Vec::new(),
        final_round,
    };
    write_round_record(&run.store.run_dir(&run.run_id), &record).unwrap();
}

fn finalization(run: &Run) -> FinalizationRecordV1 {
    serde_json::from_slice(
        &std::fs::read(
            run.store
                .run_dir(&run.run_id)
                .join(FINALIZATION_RECORD_PATH),
        )
        .unwrap(),
    )
    .unwrap()
}

async fn finalize(run: &Run, status: WorkflowV2Status) -> WorkflowV2ScriptSummary {
    finalize_run(
        &run.store,
        &run.run_id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        None,
        summary(status),
        &run.v2_store,
    )
    .await
    .expect("finalizes")
}

/// The contract: an accepted script whose last acceptance round has a
/// failing check ends `NeedsReview`, with the check ids in the summary and on
/// the finalization record — never `Completed`.
#[tokio::test]
async fn a_failing_final_round_cannot_finalize_complete() {
    let run = run();
    record_round(
        &run,
        1,
        vec![check(
            "REQ-1",
            AcceptanceCheckStatus::Passed,
            &["TASK-G-001"],
        )],
        false,
    );
    record_round(
        &run,
        2,
        vec![
            check("REQ-1", AcceptanceCheckStatus::Passed, &["TASK-G-001"]),
            check("REQ-2", AcceptanceCheckStatus::Failed, &["TASK-G-002"]),
            check("REQ-9", AcceptanceCheckStatus::Failed, &[]),
        ],
        true,
    );
    let summary = finalize(&run, WorkflowV2Status::Accepted).await;
    assert_eq!(summary.status, WorkflowV2Status::NeedsReview);
    assert_eq!(
        summary.failed_call.as_deref(),
        Some("acceptance-contract-run-2")
    );
    let next = summary.next_action.expect("names the failing checks");
    assert!(
        next.contains("REQ-2") && next.contains("TASK-G-002"),
        "{next}"
    );
    assert!(
        next.contains("REQ-9") && next.contains("no task implements it"),
        "{next}"
    );
    assert!(
        !next.contains("REQ-1"),
        "a passing check is not a finding: {next}"
    );
    let state = run.store.load_state(&run.run_id).unwrap();
    assert_eq!(state.status, RunStatus::NeedsReview);
    let record = finalization(&run);
    assert_eq!(record.terminal_status, RunStatus::NeedsReview);
    assert_eq!(
        record.terminal_v2_status,
        Some(WorkflowV2Status::NeedsReview)
    );
    let gate = record.acceptance_gate.expect("gate recorded");
    assert_eq!(gate.final_round, 2);
    assert_eq!(gate.failing_check_ids, vec!["REQ-2", "REQ-9"]);
    assert_eq!(gate.unowned_failing_check_ids, vec!["REQ-9"]);
    assert_eq!(gate.record_path, "v2/acceptance/round-02/attempt-01.json");
}

#[tokio::test]
async fn a_clean_final_round_finalizes_complete_with_the_gate_recorded() {
    let run = run();
    record_round(
        &run,
        1,
        vec![check(
            "REQ-2",
            AcceptanceCheckStatus::Failed,
            &["TASK-G-002"],
        )],
        false,
    );
    record_round(
        &run,
        2,
        vec![check(
            "REQ-2",
            AcceptanceCheckStatus::Passed,
            &["TASK-G-002"],
        )],
        true,
    );
    let summary = finalize(&run, WorkflowV2Status::Accepted).await;
    assert_eq!(summary.status, WorkflowV2Status::Accepted);
    assert!(summary.next_action.is_none());
    assert_eq!(
        run.store.load_state(&run.run_id).unwrap().status,
        RunStatus::Completed
    );
    let gate = finalization(&run).acceptance_gate.expect("gate recorded");
    assert!(gate.failing_check_ids.is_empty());
    assert_eq!(gate.final_round, 2);
}

/// A stage that could not evaluate is not a pass either.
#[tokio::test]
async fn an_unevaluable_stage_finalizes_needs_review() {
    let run = run();
    let record = AcceptanceRoundRecordV1 {
        schema_version: ACCEPTANCE_ROUND_RECORD_SCHEMA_VERSION,
        run_id: run.run_id.clone(),
        call_id: "acceptance-contract-run-1".into(),
        round: 1,
        attempt: 1,
        max_rounds: 3,
        contract_present: true,
        requested_check_ids: Vec::new(),
        execution: None,
        checks: Vec::new(),
        operational_errors: vec!["the contract is not frozen".into()],
        final_round: true,
    };
    write_round_record(&run.store.run_dir(&run.run_id), &record).unwrap();
    let summary = finalize(&run, WorkflowV2Status::Noop).await;
    assert_eq!(summary.status, WorkflowV2Status::NeedsReview);
    assert!(summary.next_action.unwrap().contains("could not evaluate"));
    assert_eq!(
        run.store.load_state(&run.run_id).unwrap().status,
        RunStatus::NeedsReview
    );
}

/// A run that never reached the stage (an older persisted script) and a
/// non-authored run finalize exactly as before: no gate.
#[tokio::test]
async fn runs_without_a_record_or_of_another_kind_are_untouched() {
    let legacy = run();
    let finalized = finalize(&legacy, WorkflowV2Status::Accepted).await;
    assert_eq!(finalized.status, WorkflowV2Status::Accepted);
    assert_eq!(
        legacy.store.load_state(&legacy.run_id).unwrap().status,
        RunStatus::Completed
    );
    assert!(finalization(&legacy).acceptance_gate.is_none());

    let other = run();
    record_round(
        &other,
        1,
        vec![check("REQ-2", AcceptanceCheckStatus::Failed, &[])],
        true,
    );
    let finalized = finalize_run(
        &other.store,
        &other.run_id,
        WorkflowRunKind::LegacyDecomposed,
        None,
        summary(WorkflowV2Status::Accepted),
        &other.v2_store,
    )
    .await
    .unwrap();
    assert_eq!(finalized.status, WorkflowV2Status::Accepted);
    assert!(finalization(&other).acceptance_gate.is_none());
}

/// A more severe script outcome is not softened into `NeedsReview`.
#[tokio::test]
async fn a_failed_script_keeps_its_own_status() {
    let run = run();
    record_round(
        &run,
        1,
        vec![check(
            "REQ-2",
            AcceptanceCheckStatus::Failed,
            &["TASK-G-002"],
        )],
        true,
    );
    let (gated, gate) =
        apply_acceptance_gate(&run.store, &run.run_id, summary(WorkflowV2Status::Failed)).unwrap();
    assert_eq!(gated.status, WorkflowV2Status::Failed);
    assert!(gate.expect("gate").blocks_completion());
}

// -- The authored run's terminal rule over its final accounting ------------

fn verify_call(task: &str) -> WorkflowV2HostCall {
    let mut call = call();
    call.id = format!("review-verify-{}-1", task.to_ascii_lowercase());
    call.options.extra.insert(
        "remediationContract".into(),
        serde_json::json!({ "version": 1, "stage": "verify", "taskId": task, "round": 1 }),
    );
    call
}

fn record_call(run: &Run, call: &WorkflowV2HostCall, status: WorkflowV2Status) {
    let record = WorkflowV2CallRecord::new(
        run.v2_store.run_id(),
        call.clone(),
        1,
        "input".into(),
        WorkflowV2Result {
            status,
            summary: "recorded".into(),
            ..WorkflowV2Result::default()
        },
        Vec::new(),
    );
    run.v2_store.save_call_record(&record).unwrap();
}

/// A run whose review found an issue on TASK-G-001 (`needs_review` on the
/// map call), remediated and re-verified it, and passed acceptance.
fn closed_run(verify_status: WorkflowV2Status) -> (Run, WorkflowV2ScriptSummary) {
    let run = run();
    let verify = verify_call("TASK-G-001");
    record_call(&run, &verify, verify_status);
    record_round(
        &run,
        1,
        vec![check(
            "REQ-1",
            AcceptanceCheckStatus::Passed,
            &["TASK-G-001"],
        )],
        true,
    );
    let mut acceptance = call();
    acceptance.id = "acceptance-contract-run-1".into();
    acceptance.method = WorkflowV2HostMethod::Tool;
    acceptance.options.extra.insert(
        "tool".into(),
        serde_json::json!(archon_workflow::v2::acceptance_stage::ACCEPTANCE_STAGE_TOOL),
    );
    record_call(&run, &acceptance, WorkflowV2Status::Accepted);
    let mut summary = summary(WorkflowV2Status::NeedsReview);
    summary.calls.push(verify);
    summary.calls.push(acceptance);
    summary.script_result = Some(
        serde_json::json!({
            "accepted": ["TASK-G-001"],
            "blocked": [],
            "adversarial_findings": [{ "id": "f1", "canonical_task_ids": ["TASK-G-001"] }],
            "uncovered_requirements": [],
            "review_remediation": { "resolved": [{ "taskId": "TASK-G-001", "findingCount": 1 }], "unresolved": [], "unassigned": [] },
        })
        .to_string(),
    );
    (run, summary)
}

#[tokio::test]
async fn an_intermediate_needs_review_call_does_not_pin_a_closed_run() {
    let (run, summary) = closed_run(WorkflowV2Status::Accepted);
    let decided = apply_authored_run_outcome(
        &run.store,
        &run.run_id,
        &run.v2_store,
        true,
        summary.clone(),
    )
    .unwrap();
    assert_eq!(decided.status, WorkflowV2Status::Accepted);
    assert!(decided.next_action.is_none());
    // A resumed process replays the accepted calls, so its accumulator never
    // saw the needs_review call; the verdict must not depend on that history.
    let mut resumed = summary;
    resumed.status = WorkflowV2Status::Accepted;
    let replayed =
        apply_authored_run_outcome(&run.store, &run.run_id, &run.v2_store, true, resumed).unwrap();
    assert_eq!(replayed.status, decided.status);
    let events = std::fs::read_to_string(run.store.events_path(&run.run_id)).unwrap();
    assert!(
        events.contains("authored_run_outcome") && events.contains("acceptance round 1 passed"),
        "{events}"
    );
    let finalized = finalize_run(
        &run.store,
        &run.run_id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        None,
        decided,
        &run.v2_store,
    )
    .await
    .unwrap();
    assert_eq!(finalized.status, WorkflowV2Status::Accepted);
    assert_eq!(
        run.store.load_state(&run.run_id).unwrap().status,
        RunStatus::Completed
    );
    assert_eq!(finalization(&run).terminal_status, RunStatus::Completed);
}

#[tokio::test]
async fn an_unbacked_resolution_finalizes_needs_review_and_says_why() {
    let (run, summary) = closed_run(WorkflowV2Status::NeedsReview);
    let decided =
        apply_authored_run_outcome(&run.store, &run.run_id, &run.v2_store, true, summary).unwrap();
    assert_eq!(decided.status, WorkflowV2Status::NeedsReview);
    let next = decided.next_action.clone().expect("names the open item");
    assert!(next.contains("TASK-G-001"), "{next}");
    let finalized = finalize_run(
        &run.store,
        &run.run_id,
        WorkflowRunKind::AuthoredTaskWorkflow,
        None,
        decided,
        &run.v2_store,
    )
    .await
    .unwrap();
    assert_eq!(finalized.status, WorkflowV2Status::NeedsReview);
    assert_eq!(finalization(&run).terminal_status, RunStatus::NeedsReview);
}

#[test]
fn a_host_recorded_terminal_failure_is_not_overridden() {
    let (run, mut summary) = closed_run(WorkflowV2Status::Accepted);
    summary.status = WorkflowV2Status::Failed;
    summary.failed_call = Some("repository-audit-final".into());
    summary.next_action = Some("audit".into());
    let decided =
        apply_authored_run_outcome(&run.store, &run.run_id, &run.v2_store, true, summary).unwrap();
    assert_eq!(decided.status, WorkflowV2Status::Failed);
    assert_eq!(decided.next_action.as_deref(), Some("audit"));
}

use super::*;
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

fn universe() -> WorkflowV2TaskUniverse {
    let task = |id: &str, implements: &[&str]| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.to_string(),
        source_path: format!("tasks/{id}.md"),
        implements: implements.iter().map(|s| s.to_string()).collect(),
        ..WorkflowV2TaskUniverseTask::default()
    };
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec!["tasks".into()],
        tasks: vec![
            task("TASK-B-002", &["REQ-2", "REQ-3"]),
            task("TASK-A-001", &["REQ-1", "REQ-3"]),
            task("TASK-C-003", &[]),
        ],
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

fn record(
    round: u32,
    attempt: u32,
    checks: Vec<AcceptanceCheckRecordV1>,
) -> AcceptanceRoundRecordV1 {
    AcceptanceRoundRecordV1 {
        schema_version: ACCEPTANCE_ROUND_RECORD_SCHEMA_VERSION,
        run_id: "wf-test".into(),
        call_id: format!("{ACCEPTANCE_STAGE_CALL_PREFIX}{round}"),
        round,
        attempt,
        max_rounds: ACCEPTANCE_MAX_ROUNDS,
        contract_present: true,
        requested_check_ids: Vec::new(),
        execution: None,
        checks,
        operational_errors: Vec::new(),
        final_round: false,
    }
}

/// An acceptance id maps to every task whose `implements` names it, in
/// sorted order; an id nobody implements maps to nothing.
#[test]
fn owning_tasks_follow_the_universe_implements_lists() {
    let universe = universe();
    assert_eq!(owning_tasks(Some(&universe), "REQ-1"), vec!["TASK-A-001"]);
    assert_eq!(
        owning_tasks(Some(&universe), "REQ-3"),
        vec!["TASK-A-001", "TASK-B-002"]
    );
    assert!(owning_tasks(Some(&universe), "REQ-9").is_empty());
    assert!(owning_tasks(None, "REQ-1").is_empty());
}

#[test]
fn a_round_blocks_completion_on_any_failing_or_erroring_check() {
    let passing = record(
        1,
        1,
        vec![check(
            "REQ-1",
            AcceptanceCheckStatus::Passed,
            &["TASK-A-001"],
        )],
    );
    assert!(!passing.blocks_completion());
    let failing = record(
        1,
        1,
        vec![
            check("REQ-1", AcceptanceCheckStatus::Passed, &["TASK-A-001"]),
            check("REQ-2", AcceptanceCheckStatus::Failed, &["TASK-B-002"]),
            check("REQ-9", AcceptanceCheckStatus::Error, &[]),
        ],
    );
    assert!(failing.blocks_completion());
    assert_eq!(failing.failing_check_ids(), vec!["REQ-2", "REQ-9"]);
    assert_eq!(failing.unowned_failing_check_ids(), vec!["REQ-9"]);
    assert_eq!(failing.passed_check_ids(), vec!["REQ-1"]);
    assert!(failing.has_remediable_failures());
    let mut errored = record(1, 1, vec![]);
    errored.operational_errors.push("no task root".into());
    assert!(errored.blocks_completion());
    let unowned_only = record(
        1,
        1,
        vec![check("REQ-9", AcceptanceCheckStatus::Failed, &[])],
    );
    assert!(!unowned_only.has_remediable_failures());
}

/// Records are append-only per round: a re-entered round writes the next
/// attempt beside the earlier one, and the latest lookup follows round then
/// attempt order.
#[test]
fn round_records_append_and_the_latest_is_the_highest_round_and_attempt() {
    let dir = tempfile::tempdir().expect("tmp");
    let run_dir = dir.path();
    assert!(latest_round_record(run_dir).expect("readable").is_none());
    assert_eq!(next_attempt(run_dir, 1), 1);
    let first = record(
        1,
        1,
        vec![check(
            "REQ-2",
            AcceptanceCheckStatus::Failed,
            &["TASK-B-002"],
        )],
    );
    let first_path = write_round_record(run_dir, &first).expect("write attempt 1");
    assert_eq!(next_attempt(run_dir, 1), 2);
    let second = record(
        1,
        2,
        vec![check(
            "REQ-2",
            AcceptanceCheckStatus::Failed,
            &["TASK-B-002"],
        )],
    );
    let second_path = write_round_record(run_dir, &second).expect("write attempt 2");
    assert!(first_path.exists() && second_path.exists());
    assert_ne!(first_path, second_path);
    // Overwriting an attempt is refused.
    let error = write_round_record(run_dir, &second).expect_err("append-only");
    assert!(error.to_string().contains("append-only"), "{error}");
    let (latest, path) = latest_round_record(run_dir)
        .expect("readable")
        .expect("record");
    assert_eq!((latest.round, latest.attempt), (1, 2));
    assert_eq!(path, second_path);
    let mut third = record(
        2,
        1,
        vec![check(
            "REQ-2",
            AcceptanceCheckStatus::Passed,
            &["TASK-B-002"],
        )],
    );
    third.final_round = true;
    let third_path = write_round_record(run_dir, &third).expect("write round 2");
    let (latest, path) = latest_round_record(run_dir)
        .expect("readable")
        .expect("record");
    assert_eq!((latest.round, latest.attempt), (2, 1));
    assert_eq!(path, third_path);
    assert_eq!(
        relative_record_path(run_dir, &third_path),
        "v2/acceptance/round-02/attempt-01.json"
    );
}

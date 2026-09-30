//! A failed check no task was named to fix is reassigned, never dropped.

use super::super::super::acceptance_regression::RegressionSearchV1;
use super::super::super::acceptance_stage::{AcceptanceCheckRecordV1, AcceptanceCheckStatus};
use super::super::{mark_blocked, route_failures};
use super::*;
use crate::task_universe::WorkflowV2TaskUniverseTask;

fn repo(files: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for path in files {
        let target = dir.path().join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, "//\n").unwrap();
    }
    dir
}

fn universe() -> WorkflowV2TaskUniverse {
    let task = |id: &str, owns: &[&str]| WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: owns.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    };
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task("TASK-STORE", &["`pkg/store/write.rs`"]),
            task("TASK-CLI", &["pkg/cli/args.rs"]),
        ],
    }
}

fn failing(stderr: &str) -> AcceptanceCheckRecordV1 {
    AcceptanceCheckRecordV1 {
        check_id: "AC-9".into(),
        criterion: "the command succeeds".into(),
        kind: "command".into(),
        status: AcceptanceCheckStatus::Failed,
        exit_code: Some(1),
        operational_error: None,
        owning_tasks: Vec::new(),
        stdout_tail: String::new(),
        stderr_tail: stderr.into(),
        regressed_by: None,
        contract_defect: false,
        routing: None,
        regression_search: Some(RegressionSearchV1::not_searched("no scratch policy")),
        blocked: None,
    }
}

fn round(checks: Vec<AcceptanceCheckRecordV1>) -> AcceptanceRoundRecordV1 {
    AcceptanceRoundRecordV1 {
        schema_version: 1,
        run_id: "run".into(),
        call_id: "acceptance-contract-run-1".into(),
        round: 1,
        attempt: 1,
        max_rounds: 3,
        contract_present: true,
        requested_check_ids: Vec::new(),
        execution: None,
        checks,
        operational_errors: Vec::new(),
        contract_repairs: Vec::new(),
        final_round: false,
    }
}

#[test]
fn an_unowned_check_goes_to_the_task_nearest_the_file_it_implicates() {
    let dir = repo(&[
        "pkg/store/write.rs",
        "pkg/store/index.rs",
        "pkg/cli/args.rs",
        "pkg/Cargo.toml",
    ]);
    let root = dir.path();
    // A file no task declares, beside one TASK-STORE declares.
    let mut record = round(vec![failing("pkg/store/index.rs:4: error: bad index\n")]);
    route_failures(Some(&universe()), root, &[], &mut record);
    mark_blocked(&mut record);
    reroute(Some(&universe()), &mut record);
    let check = &record.checks[0];
    assert_eq!(check.blocked, None);
    assert_eq!(check.owning_tasks, ["TASK-STORE"]);
    let routing = check.routing.as_ref().unwrap();
    assert_eq!(routing.reassigned_to, ["TASK-STORE"]);
    assert!(!routing.reassign_reason.is_empty());
    assert!(record.has_remediable_failures());
    assert_eq!(record.task_remediable_check_ids(), ["AC-9"]);
}

#[test]
fn a_check_whose_failure_names_no_file_goes_to_every_task_together() {
    let dir = repo(&["pkg/Cargo.toml"]);
    let mut record = round(vec![failing("Error: boom\n")]);
    route_failures(Some(&universe()), dir.path(), &[], &mut record);
    mark_blocked(&mut record);
    assert!(record.checks[0].blocked.is_some());
    reroute(Some(&universe()), &mut record);
    let check = &record.checks[0];
    assert_eq!(check.blocked, None);
    assert_eq!(check.owning_tasks, ["TASK-CLI", "TASK-STORE"]);
    assert!(record.blocked_checks().is_empty());
}

#[test]
fn only_an_empty_universe_leaves_a_check_blocked() {
    let dir = repo(&["pkg/Cargo.toml"]);
    let empty = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: Vec::new(),
    };
    let mut record = round(vec![failing("Error: boom\n")]);
    route_failures(Some(&empty), dir.path(), &[], &mut record);
    mark_blocked(&mut record);
    reroute(Some(&empty), &mut record);
    assert!(record.checks[0].blocked.is_some());
    assert!(record.checks[0].owning_tasks.is_empty());
}

#[test]
fn every_implicated_file_is_routed_with_no_cap() {
    let files: Vec<String> = (0..9).map(|n| format!("pkg/store/m{n}.rs")).collect();
    let mut all: Vec<&str> = files.iter().map(String::as_str).collect();
    all.push("pkg/Cargo.toml");
    all.push("pkg/store/write.rs");
    let dir = repo(&all);
    let stderr: String = files
        .iter()
        .map(|file| format!("{file}:1: error: broken\n"))
        .collect();
    let mut record = round(vec![failing(&stderr)]);
    route_failures(Some(&universe()), dir.path(), &[], &mut record);
    let routing = record.checks[0].routing.as_ref().unwrap();
    assert_eq!(routing.implicated_files.len(), 9, "{routing:?}");
}

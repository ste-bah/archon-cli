//! Batch E: a failing check reaches the tasks that can write its fix.

use super::*;
use crate::task_universe::WorkflowV2TaskUniverseTask;
use crate::v2::acceptance_regression::AcceptanceRegressionV1;
use crate::v2::acceptance_stage::{AcceptanceCheckRecordV1, AcceptanceCheckStatus};

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for path in [
        "pkg/core/engine.rs",
        "pkg/core/loose.rs",
        "pkg/core/sealed.rs",
        "pkg/cli/args.rs",
        "docs/notes.md",
    ] {
        let target = dir.path().join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, "//\n").unwrap();
    }
    dir
}

fn task(id: &str, owns: &[&str], forbids: &[&str]) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: owns.iter().map(|f| f.to_string()).collect(),
        files_forbidden_to_change: forbids.iter().map(|f| f.to_string()).collect(),
        implements: if id == "TASK-IMPL" {
            vec!["AC-1".into()]
        } else {
            Vec::new()
        },
        ..Default::default()
    }
}

fn universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            // Implements the check, and may not touch the sealed module.
            task("TASK-IMPL", &["pkg/cli/args.rs"], &["`pkg/core/sealed.rs`"]),
            // Owns the engine module the failure points into.
            task("TASK-OWNER", &["pkg/core/engine.rs"], &[]),
        ],
    }
}

fn failing(stderr: &str, owners: &[&str]) -> AcceptanceCheckRecordV1 {
    AcceptanceCheckRecordV1 {
        check_id: "AC-1".into(),
        criterion: "the command succeeds".into(),
        kind: "command".into(),
        status: AcceptanceCheckStatus::Failed,
        exit_code: Some(1),
        operational_error: None,
        owning_tasks: owners.iter().map(|s| s.to_string()).collect(),
        stdout_tail: String::new(),
        stderr_tail: stderr.into(),
        regressed_by: None,
        contract_defect: false,
        routing: None,
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
fn a_failure_in_another_tasks_file_routes_that_task_too() {
    let dir = repo();
    let root = dir.path();
    let stderr = format!(
        "thread 'main' panicked at {}/pkg/core/engine.rs:41:9:\nrejected input\n",
        root.display()
    );
    let mut record = round(vec![failing(&stderr, &["TASK-IMPL"])]);
    route_failures(Some(&universe()), root, &mut record);
    let routing = record.checks[0].routing.clone().expect("routed");
    assert_eq!(routing.implicated_files, ["pkg/core/engine.rs"]);
    assert_eq!(routing.writer_tasks, ["TASK-OWNER"]);
    assert!(routing.granted_files.is_empty());
    assert!(routing.describe().contains("routed also to TASK-OWNER"));
}

#[test]
fn an_unowned_file_is_granted_and_a_forbidden_or_protected_one_is_not() {
    let dir = repo();
    let root = dir.path();
    let stderr = "error at pkg/core/loose.rs:7\nsee pkg/core/sealed.rs:3 and docs/notes.md:1\n";
    let mut record = round(vec![failing(stderr, &["TASK-IMPL"])]);
    route_failures(Some(&universe()), root, &mut record);
    let routing = record.checks[0].routing.clone().expect("routed");
    assert_eq!(routing.granted_files, ["pkg/core/loose.rs"]);
    assert!(routing.writer_tasks.is_empty(), "{routing:?}");
    let unwritable: Vec<&str> = routing.unwritable.iter().map(|(f, _)| f.as_str()).collect();
    assert_eq!(unwritable, ["docs/notes.md", "pkg/core/sealed.rs"]);
}

#[test]
fn an_unimplemented_check_is_remediable_through_the_owner_of_its_broken_file() {
    let dir = repo();
    let root = dir.path();
    let mut check = failing("", &[]);
    check.regressed_by = Some(AcceptanceRegressionV1 {
        held_at: "a".into(),
        landing_commit: "b".into(),
        landing_stage: "s".into(),
        tasks: Vec::new(),
        changed_files: vec!["pkg/core/engine.rs".into(), "gone.rs".into()],
    });
    let mut record = round(vec![check]);
    assert!(!record.has_remediable_failures());
    route_failures(Some(&universe()), root, &mut record);
    let routing = record.checks[0].routing.clone().expect("routed");
    // A path that is no longer a repository file is not implicated.
    assert_eq!(routing.implicated_files, ["pkg/core/engine.rs"]);
    assert_eq!(routing.writer_tasks, ["TASK-OWNER"]);
    assert!(record.has_remediable_failures());
}

#[test]
fn locations_are_read_from_the_end_bounded_and_only_as_repository_files() {
    let dir = repo();
    let root = dir.path();
    let text = "pkg/cli/args.rs:1\nhttps://x.test/a.rs:2 ../escape.rs:3 /abs/elsewhere.rs:4\n\
                pkg/core/engine.rs:9:1 pkg/core/engine.rs:10 `pkg/core/loose.rs:2`,";
    assert_eq!(
        located_files(text, root, 6),
        ["pkg/core/loose.rs", "pkg/core/engine.rs", "pkg/cli/args.rs"]
    );
    assert_eq!(located_files(text, root, 1), ["pkg/core/loose.rs"]);
    // A path with no line location is not a failure location.
    assert!(located_files("pkg/core/engine.rs", root, 6).is_empty());
    // A passing check or no universe routes nothing.
    let mut record = round(vec![failing("pkg/core/engine.rs:1", &[])]);
    record.checks[0].status = AcceptanceCheckStatus::Passed;
    route_failures(Some(&universe()), root, &mut record);
    assert!(record.checks[0].routing.is_none());
    let mut record = round(vec![failing("pkg/core/engine.rs:1", &[])]);
    route_failures(None, root, &mut record);
    assert!(record.checks[0].routing.is_none());
}

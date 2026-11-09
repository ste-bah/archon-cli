//! Batch O2 (m2): a slot check the host cannot record fails its call.

use super::*;
use crate::task_universe::WorkflowV2TaskUniverseTask;
use crate::v2::WorkflowV2HostOptions;
use crate::v2::write::test_baseline_run_base::tests::FakeCargo;

#[cfg(unix)]
#[tokio::test]
async fn same_commit_without_failure_identity_still_blocks_the_slot() {
    use crate::v2::write::test_baseline_run_base::tests::{
        bind_run, freeze_runner_clock, head, world,
    };
    let _clock = freeze_runner_clock();
    let dir = tempfile::tempdir().unwrap();
    let (root, _, host, _) = world(dir.path());
    let store = WorkflowV2ResultStore::new(dir.path().join("run/v2"));
    bind_run(&store, &head(&root));
    let command = "result=red; exit 1";
    let mut task = WorkflowV2TaskUniverseTask {
        focused_tests: vec![command.into()],
        ..Default::default()
    };
    task.freeze_focused_commands(&root, std::ffi::OsStr::new(""));
    let universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![task],
    };
    let slot = check(&store, &host, &universe, &root, "same-tip").await;
    assert_eq!(slot.base, slot.tip);
    assert_eq!(slot.findings.len(), 1, "{slot:?}");
    assert!(matches!(
        slot.findings[0].finding,
        RegressionFinding::UnnamedNew { .. }
    ));
}

#[tokio::test]
async fn a_slot_check_that_cannot_be_recorded_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(dir.path().join("run/v2"));
    std::fs::create_dir_all(store.root()).unwrap();
    // Where the record would go is a file: nothing can be written there.
    std::fs::write(store.root().join("baseline-tests"), "x").unwrap();
    let universe = WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-A".into(),
            focused_commands: Some(vec!["cargo test -p app".into()]),
            focused_tests: vec!["cargo test -p app".into()],
            ..Default::default()
        }],
    };
    let mut options = WorkflowV2HostOptions::default();
    options
        .extra
        .insert("residualGaps".into(), serde_json::json!(true));
    options
        .extra
        .insert("residualPass".into(), serde_json::json!(2));
    let call = WorkflowV2HostCall {
        id: "residual-gaps-2".into(),
        method: WorkflowV2HostMethod::Checkpoint,
        write_mode: None,
        options,
    };
    let cargo = FakeCargo {
        bin: dir.path().join("bin"),
        resource_limits: archon_tools::workflow_resource_env::CargoResourceLimits::default(),
    };
    let outcome = prepare_slot(&store, &cargo, Some(&universe), dir.path(), &call).await;
    assert!(outcome.is_err(), "{outcome:?}");
}

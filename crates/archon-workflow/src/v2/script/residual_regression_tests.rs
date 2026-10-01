//! Batch O2 (M2, m2): a regression gap is built from the slot's record
//! alone, and a check that could not judge the tree is surfaced.

use serde_json::json;

use super::*;
use crate::task_universe::WorkflowV2TaskUniverseTask;
use crate::v2::verification::regression_slot::{RegressionSlot, write_slot};
use crate::v2::{
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Result,
};

const LIB: &str = "cargo test -p app --lib";

/// A store holding pass `pass`'s slot checkpoint and its check's `slot`.
fn slot_world(
    store: &WorkflowV2ResultStore,
    pass: u64,
    slot: RegressionSlot,
) -> WorkflowV2CallRecord {
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert("residualGaps".into(), json!(true));
    options.extra.insert("residualPass".into(), json!(pass));
    let call = WorkflowV2HostCall {
        id: slot.call_id.clone(),
        method: WorkflowV2HostMethod::Checkpoint,
        write_mode: None,
        options,
    };
    write_slot(store, &slot).unwrap();
    let record = WorkflowV2CallRecord::new(
        "run",
        call,
        1,
        "h".into(),
        WorkflowV2Result::accepted("residuals"),
        vec![],
    );
    store.save_call_record(&record).unwrap();
    record
}

fn universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-A".into(),
            focused_tests: vec![LIB.into()],
            ..Default::default()
        }],
    }
}

#[test]
fn a_regression_gap_names_the_files_its_check_recorded_whatever_the_live_tree_holds() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(dir.path().join("run/v2"));
    let finding = SlotFinding {
        finding: RegressionFinding::NewFailure {
            command: LIB.into(),
            test: "shared::tests::new".into(),
            base_had: true,
        },
        // Neither file exists in the live tree any more.
        file: Some("src/shared_tests.rs".into()),
        failure_files: vec!["src/shared.rs".into()],
    };
    let record = slot_world(
        &store,
        2,
        RegressionSlot {
            call_id: "residual-gaps-2".into(),
            findings: vec![finding],
            ..RegressionSlot::default()
        },
    );
    let gaps = regression_gaps(&store, &[record], &universe(), 2, &BTreeSet::new());
    assert_eq!(gaps.len(), 1, "{gaps:#?}");
    let gap = &gaps[0];
    assert_eq!(gap.files, ["src/shared_tests.rs", "src/shared.rs"]);
    assert!(gap.description.contains("src/shared_tests.rs"), "{gap:#?}");
    assert_eq!(gap.unit_tasks, BTreeSet::from(["TASK-A".to_string()]));
}

#[test]
fn a_check_that_could_not_judge_the_tree_is_surfaced_as_a_gap() {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(dir.path().join("run/v2"));
    let record = slot_world(
        &store,
        3,
        RegressionSlot {
            call_id: "residual-gaps-3".into(),
            unjudged: Some("the run's base commit or the checkout's HEAD is unreadable".into()),
            ..RegressionSlot::default()
        },
    );
    let gaps = regression_gaps(&store, &[record], &universe(), 3, &BTreeSet::new());
    assert_eq!(gaps.len(), 1, "{gaps:#?}");
    assert_eq!(gaps[0].id, format!("{REGRESSION_GAP_ID}@unjudged"));
    assert!(gaps[0].description.contains("unreadable"), "{gaps:#?}");
    assert!(is_regression_gap(&gaps[0]));
}

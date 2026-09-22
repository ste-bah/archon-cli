//! Issue-70: the stamp a verifier is held to comes from the record at the
//! VERIFICATION base when one exists, so a test another task's later commit
//! broke is exempt (and routed), while a red test in the task's own file
//! still refuses the verdict.
use std::collections::BTreeMap;

use serde_json::json;

use super::tests::{accepted_outcome, command};
use super::{BaselineStamp, enforce_baseline_tests, stamp_baseline_tests_input_at, stamped};
use crate::v2::write::test_baseline::{
    BaselineObligation, BranchBaseline, CommandBaseline, RoutedFailure, SCHEMA_VERSION, save_record,
};
use crate::v2::{WorkflowV2CommandStatus, WorkflowV2ResultStore, WorkflowV2Status};

const COMMAND: &str = "cargo test -p engine trading";
const IMPL_BASE: &str = "1111111111111111";
const VERIFICATION_BASE: &str = "2222222222222222";
const LATER_BASE: &str = "3333333333333333";

/// A record for TASK-A at `base` under `stage`: `own` red tests in its own
/// file, `theirs` red tests in TASK-B's file.
fn record(store: &WorkflowV2ResultStore, stage: &str, base: &str, own: &[&str], theirs: &[&str]) {
    let failing = own.iter().chain(theirs).map(|t| t.to_string()).collect();
    save_record(
        store,
        &BranchBaseline {
            schema_version: SCHEMA_VERSION,
            stage_id: stage.into(),
            branch_id: format!("{stage}-check"),
            base_commit: base.into(),
            canonical_task_ids: vec!["TASK-A".into()],
            commands: vec![CommandBaseline {
                command: COMMAND.into(),
                base_commit: base.into(),
                exit_code: Some(if own.is_empty() && theirs.is_empty() {
                    0
                } else {
                    101
                }),
                timed_out: false,
                duration_ms: 1,
                failing_tests: failing,
                tail: Vec::new(),
                error: None,
                cached: false,
                diagnostic_files: Vec::new(),
            }],
            obligations: own
                .iter()
                .map(|t| BaselineObligation {
                    test_id: Some(t.to_string()),
                    file: Some("crates/engine/src/trading.rs".into()),
                    command: COMMAND.into(),
                })
                .collect(),
            routed: theirs
                .iter()
                .map(|t| RoutedFailure {
                    test_id: t.to_string(),
                    file: "crates/engine/src/stooq.rs".into(),
                    owner_task: "TASK-B".into(),
                    command: COMMAND.into(),
                })
                .collect(),
            ignored: Vec::new(),
            inherited: Vec::new(),
            pre_existing: Vec::new(),
        },
    );
}

/// The live shape: the implementation record is green, the record at the
/// verification base is red on a test in TASK-B's file, and a newer record
/// from another wave exists too.
fn store_with_three_bases(temp: &std::path::Path) -> WorkflowV2ResultStore {
    let store = WorkflowV2ResultStore::new(temp.join("v2"));
    record(&store, "agents-1", IMPL_BASE, &[], &[]);
    std::thread::sleep(std::time::Duration::from_millis(20));
    record(
        &store,
        "verification-wave-verify-1",
        VERIFICATION_BASE,
        &[],
        &["stooq::tests::native_ingest"],
    );
    std::thread::sleep(std::time::Duration::from_millis(20));
    record(&store, "agents-2", LATER_BASE, &[], &[]);
    store
}

#[test]
fn for_tasks_at_prefers_the_record_at_the_verification_base_over_newer_and_older_ones() {
    let temp = tempfile::tempdir().unwrap();
    let store = store_with_three_bases(temp.path());
    let ids = ["TASK-A".to_string()];
    let stamp = BaselineStamp::for_tasks_at(&store, &ids, Some(VERIFICATION_BASE)).expect("stamp");
    assert_eq!(stamp.base_commit, VERIFICATION_BASE);
    assert!(stamp.verification_base);
    assert_eq!(stamp.other_owner.len(), 1);
    assert_eq!(stamp.other_owner[0].test_id, "stooq::tests::native_ingest");
    assert_eq!(stamp.other_owner[0].owner_task, "TASK-B");
    // Without a requested base the newest record wins, as before.
    let newest = BaselineStamp::for_tasks(&store, &ids).expect("stamp");
    assert_eq!(newest.base_commit, LATER_BASE);
    assert!(!newest.verification_base);
    assert!(newest.other_owner.is_empty());
    // A base no record was established at falls back to the newest record
    // and says so.
    let fallback =
        BaselineStamp::for_tasks_at(&store, &ids, Some("9999999999999999")).expect("stamp");
    assert_eq!(fallback.base_commit, LATER_BASE);
    assert!(!fallback.verification_base);
}

#[test]
fn a_red_test_owned_by_another_task_at_the_verification_base_is_exempt_and_does_not_demote() {
    let temp = tempfile::tempdir().unwrap();
    let store = store_with_three_bases(temp.path());
    let mut input =
        json!({"item": {"item_id": "verify-1-check", "canonical_task_ids": ["TASK-A"]}});
    // The item builder stamps first from the newest record: no exemption.
    stamp_baseline_tests_input_at("verification-wave-verify-1", &store, &mut input, None);
    assert!(stamped(&input).unwrap().other_owner.is_empty());
    // Re-stamped at the verification base: the routed test is exempt.
    stamp_baseline_tests_input_at(
        "verification-wave-verify-1",
        &store,
        &mut input,
        Some(VERIFICATION_BASE),
    );
    let stamp = stamped(&input).expect("re-stamped");
    assert_eq!(stamp.base_commit, VERIFICATION_BASE);
    let by_item = BTreeMap::from([("verify-1-check".to_string(), stamp)]);
    let mut outcomes = vec![accepted_outcome(
        vec![command(
            COMMAND,
            WorkflowV2CommandStatus::Failed,
            "test stooq::tests::native_ingest ... FAILED\n",
            true,
        )],
        json!({}),
    )];
    enforce_baseline_tests(&mut outcomes, &by_item);
    assert_eq!(outcomes[0].status, WorkflowV2Status::Accepted);
    assert!(
        outcomes[0]
            .result
            .as_ref()
            .unwrap()
            .residual_gaps
            .is_empty()
    );
}

#[test]
fn a_red_test_in_the_tasks_own_file_at_the_verification_base_still_demotes() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    record(&store, "agents-1", IMPL_BASE, &[], &[]);
    record(
        &store,
        "verification-wave-verify-1",
        VERIFICATION_BASE,
        &["trading::tests::gate"],
        &["stooq::tests::native_ingest"],
    );
    let stamp =
        BaselineStamp::for_tasks_at(&store, &["TASK-A".to_string()], Some(VERIFICATION_BASE))
            .expect("stamp");
    assert_eq!(stamp.must_pass, vec!["trading::tests::gate".to_string()]);
    let by_item = BTreeMap::from([("verify-1-check".to_string(), stamp)]);
    let mut outcomes = vec![accepted_outcome(
        vec![command(
            COMMAND,
            WorkflowV2CommandStatus::Failed,
            "test trading::tests::gate ... FAILED\ntest stooq::tests::native_ingest ... FAILED\n",
            true,
        )],
        json!({}),
    )];
    enforce_baseline_tests(&mut outcomes, &by_item);
    assert_eq!(outcomes[0].status, WorkflowV2Status::NeedsReview);
    assert_eq!(
        outcomes[0].result.as_ref().unwrap().data["baseline_red_tests"],
        json!(["trading::tests::gate"])
    );
}

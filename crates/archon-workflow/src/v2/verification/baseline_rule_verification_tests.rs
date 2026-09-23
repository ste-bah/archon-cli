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
use crate::v2::{
    WorkflowV2BranchOutcome, WorkflowV2CommandStatus, WorkflowV2ResultStore, WorkflowV2Status,
};

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

// The host reads only a SUMMARY of a declared command, and a summary is
// prose the parser will not mine for names. These cover the second
// exception: the verifier's TYPED failing names, cross-checked against the
// host's own routing table on the stamp.

/// What the verifier actually reports: a prose count line, the failing names
/// in a sentence, and its own attribution.
const PROSE: &str = "9 run: 1 passed, 8 failed. Failed names: stooq::tests::native_ingest, \
                     stooq::tests::second. Every one is on the host baseline list owned by \
                     TASK-B; no unlisted test red.";

fn typed_stamp(routed: &[(&str, &str)]) -> BaselineStamp {
    BaselineStamp {
        base_commit: VERIFICATION_BASE.into(),
        declared_commands: vec![COMMAND.into()],
        other_owner: routed
            .iter()
            .map(|(test_id, owner_task)| super::OtherOwnerTest {
                test_id: (*test_id).into(),
                owner_task: (*owner_task).into(),
            })
            .collect(),
        tasks: vec!["TASK-A".into()],
        ..Default::default()
    }
}

/// Run the rule over one accepted outcome whose single declared command
/// failed under a `pre_existing` claim with the prose summary above.
fn enforce(stamp: BaselineStamp, data: serde_json::Value) -> WorkflowV2BranchOutcome {
    let by_item = BTreeMap::from([("verify-1-check".to_string(), stamp)]);
    let mut outcomes = vec![accepted_outcome(
        vec![command(
            COMMAND,
            WorkflowV2CommandStatus::Failed,
            PROSE,
            true,
        )],
        data,
    )];
    enforce_baseline_tests(&mut outcomes, &by_item);
    outcomes.remove(0)
}

fn typed(failed: serde_json::Value) -> serde_json::Value {
    json!({"matched_test_check_names": {"failed": failed}})
}

#[test]
fn the_stamp_records_which_task_is_under_verification() {
    let temp = tempfile::tempdir().unwrap();
    let store = store_with_three_bases(temp.path());
    let ids = ["TASK-A".to_string()];
    let stamp = BaselineStamp::for_tasks_at(&store, &ids, Some(VERIFICATION_BASE)).expect("stamp");
    assert_eq!(stamp.tasks, vec!["TASK-A".to_string()]);
}

#[test]
fn typed_failing_names_all_routed_to_another_task_prove_the_claim_the_prose_cannot() {
    // The premise: the host's parser reads nothing out of that summary.
    assert!(
        crate::v2::write::test_baseline_parse::failing_tests(PROSE).is_empty(),
        "the parser must stay blind to prose"
    );
    let outcome = enforce(
        typed_stamp(&[
            ("stooq::tests::native_ingest", "TASK-B"),
            ("stooq::tests::second", "TASK-B"),
        ]),
        typed(json!([
            "stooq::tests::native_ingest",
            "stooq::tests::second"
        ])),
    );
    assert_eq!(outcome.status, WorkflowV2Status::Accepted);
    let result = outcome.result.as_ref().unwrap();
    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert!(result.residual_gaps.is_empty(), "{result:?}");
}

#[test]
fn a_typed_name_routed_to_the_task_under_verification_still_demotes() {
    let outcome = enforce(
        typed_stamp(&[
            ("stooq::tests::native_ingest", "TASK-B"),
            ("stooq::tests::second", "TASK-A"),
        ]),
        typed(json!([
            "stooq::tests::native_ingest",
            "stooq::tests::second"
        ])),
    );
    assert_demoted(&outcome);
}

#[test]
fn a_typed_name_in_no_routing_entry_still_demotes() {
    let outcome = enforce(
        typed_stamp(&[("stooq::tests::native_ingest", "TASK-B")]),
        typed(json!([
            "stooq::tests::native_ingest",
            "stooq::tests::second"
        ])),
    );
    assert_demoted(&outcome);
}

#[test]
fn an_empty_or_missing_typed_field_still_demotes() {
    let routed = [
        ("stooq::tests::native_ingest", "TASK-B"),
        ("stooq::tests::second", "TASK-B"),
    ];
    assert_demoted(&enforce(typed_stamp(&routed), typed(json!([]))));
    assert_demoted(&enforce(typed_stamp(&routed), json!({})));
    assert_demoted(&enforce(
        typed_stamp(&routed),
        json!({"matched_test_check_names": {}}),
    ));
}

#[test]
fn a_malformed_typed_field_still_demotes() {
    let routed = [
        ("stooq::tests::native_ingest", "TASK-B"),
        ("stooq::tests::second", "TASK-B"),
    ];
    // Not an array; an element that is not a string; an element that is blank.
    assert_demoted(&enforce(
        typed_stamp(&routed),
        typed(json!("stooq::tests::native_ingest, stooq::tests::second")),
    ));
    assert_demoted(&enforce(
        typed_stamp(&routed),
        typed(json!(["stooq::tests::native_ingest", 7])),
    ));
    assert_demoted(&enforce(
        typed_stamp(&routed),
        typed(json!(["stooq::tests::native_ingest", "  "])),
    ));
    // And with no routing table at all.
    assert_demoted(&enforce(
        typed_stamp(&[]),
        typed(json!(["stooq::tests::native_ingest"])),
    ));
}

/// Today's refusal, unchanged: the verdict is demoted under the base-commit
/// gap id and the unproven command is named.
fn assert_demoted(outcome: &WorkflowV2BranchOutcome) {
    assert_eq!(outcome.status, WorkflowV2Status::NeedsReview);
    let result = outcome.result.as_ref().unwrap();
    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    assert!(
        result
            .residual_gaps
            .iter()
            .any(|gap| gap.id == super::BASELINE_RED_TEST_GAP_ID),
        "{result:?}"
    );
    assert_eq!(
        result.data["baseline_unproven_pre_existing"],
        json!([COMMAND])
    );
}

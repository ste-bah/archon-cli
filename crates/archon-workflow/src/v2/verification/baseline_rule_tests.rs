use std::collections::BTreeMap;

use serde_json::json;

use super::{
    BASELINE_RED_TEST_GAP_ID, BASELINE_TESTS_INPUT_KEY, BaselineStamp, OtherOwnerTest,
    baseline_by_item, enforce_baseline_tests, stamp_baseline_tests_input, stamped,
};
use crate::WorkflowV2Result;
use crate::v2::write::test_baseline::{
    BaselineObligation, BranchBaseline, CommandBaseline, RoutedFailure, SCHEMA_VERSION,
    route_finding, save_record,
};
use crate::v2::{
    BranchFailureKind, WorkflowV2BranchOutcome, WorkflowV2CommandKind, WorkflowV2CommandRecord,
    WorkflowV2CommandStatus, WorkflowV2ResultStore, WorkflowV2Status,
};

fn stamp() -> BaselineStamp {
    BaselineStamp {
        base_commit: "abcdef0123456789".into(),
        declared_commands: vec!["cargo test -p engine grant".into()],
        must_pass: vec!["grant::tests::mine".into()],
        other_owner: vec![OtherOwnerTest {
            test_id: "plan::tests::theirs".into(),
            owner_task: "TASK-B".into(),
        }],
        ignored: vec!["gate::frozen".into()],
        unbaselined_commands: Vec::new(),
    }
}

fn command(
    text: &str,
    status: WorkflowV2CommandStatus,
    output: &str,
    pre_existing: bool,
) -> WorkflowV2CommandRecord {
    WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: text.into(),
        status,
        exit_code: None,
        output_summary: output.into(),
        pre_existing,
    }
}

fn accepted_outcome(
    commands: Vec<WorkflowV2CommandRecord>,
    data: serde_json::Value,
) -> WorkflowV2BranchOutcome {
    let mut result = WorkflowV2Result::accepted("verified");
    result.commands_run = commands;
    result.data = data;
    WorkflowV2BranchOutcome {
        item_id: "verify-1-check".into(),
        role: "coder".into(),
        status: WorkflowV2Status::Accepted,
        result: Some(result),
        error: None,
        failure_kind: None,
        item_input_hash: None,
        completion_evidence: Vec::new(),
    }
}

fn by_item() -> BTreeMap<String, BaselineStamp> {
    BTreeMap::from([("verify-1-check".to_string(), stamp())])
}

#[test]
fn a_red_test_not_owned_by_another_task_refuses_the_accepted_verdict_whatever_the_prose_says() {
    let output = "test grant::tests::mine ... FAILED\ntest plan::tests::theirs ... FAILED\n";
    let mut outcomes = vec![accepted_outcome(
        vec![command(
            "cargo test -p engine grant",
            WorkflowV2CommandStatus::Failed,
            &format!("{output}pre-existing on the base branch, out of scope for this task"),
            true,
        )],
        json!({}),
    )];
    enforce_baseline_tests(&mut outcomes, &by_item());
    let outcome = &outcomes[0];
    assert_eq!(outcome.status, WorkflowV2Status::NeedsReview);
    assert_eq!(outcome.failure_kind, Some(BranchFailureKind::Semantic));
    let result = outcome.result.as_ref().unwrap();
    assert!(
        result
            .residual_gaps
            .iter()
            .any(|g| g.id == BASELINE_RED_TEST_GAP_ID),
        "{result:?}"
    );
    assert_eq!(
        result.data["baseline_red_tests"],
        json!(["grant::tests::mine"])
    );
    assert_eq!(
        result.data["verification_failure_class"],
        json!("actionable_verification_failure")
    );
}

#[test]
fn a_red_test_on_the_other_owner_or_ignore_list_keeps_the_verdict() {
    let output = "test plan::tests::theirs ... FAILED\ntest gate::frozen ... FAILED\n";
    let mut outcomes = vec![accepted_outcome(
        vec![command(
            "cargo test -p engine grant",
            WorkflowV2CommandStatus::Failed,
            output,
            true,
        )],
        json!({}),
    )];
    enforce_baseline_tests(&mut outcomes, &by_item());
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
fn the_typed_failed_names_are_read_when_the_output_names_nothing() {
    let mut outcomes = vec![accepted_outcome(
        vec![command(
            "cargo test -p engine grant",
            WorkflowV2CommandStatus::Succeeded,
            "ok",
            false,
        )],
        json!({"matched_test_check_names": {"failed": ["grant::tests::mine"]}}),
    )];
    enforce_baseline_tests(&mut outcomes, &by_item());
    assert_eq!(outcomes[0].status, WorkflowV2Status::NeedsReview);
}

#[test]
fn a_pre_existing_claim_on_a_declared_command_that_names_no_test_is_not_accepted() {
    let mut outcomes = vec![accepted_outcome(
        vec![command(
            "cargo test -p engine grant",
            WorkflowV2CommandStatus::Failed,
            "error: linking failed; fails identically on the base branch",
            true,
        )],
        json!({}),
    )];
    enforce_baseline_tests(&mut outcomes, &by_item());
    let result = outcomes[0].result.as_ref().unwrap();
    assert_eq!(outcomes[0].status, WorkflowV2Status::NeedsReview);
    assert_eq!(
        result.data["baseline_unproven_pre_existing"],
        json!(["cargo test -p engine grant"])
    );
}

#[test]
fn an_item_without_a_stamp_and_a_non_accepted_outcome_are_left_alone() {
    let red = "test grant::tests::mine ... FAILED\n";
    let mut outcomes = vec![accepted_outcome(
        vec![command(
            "cargo test -p engine grant",
            WorkflowV2CommandStatus::Failed,
            red,
            false,
        )],
        json!({}),
    )];
    enforce_baseline_tests(&mut outcomes, &BTreeMap::new());
    assert_eq!(outcomes[0].status, WorkflowV2Status::Accepted);
    let mut failed = accepted_outcome(vec![], json!({}));
    failed.status = WorkflowV2Status::Failed;
    let mut outcomes = vec![failed];
    enforce_baseline_tests(&mut outcomes, &by_item());
    assert!(
        outcomes[0]
            .result
            .as_ref()
            .unwrap()
            .residual_gaps
            .is_empty()
    );
}

fn record(store: &WorkflowV2ResultStore, stage: &str, branch: &str, base: &str, task: &str) {
    save_record(
        store,
        &BranchBaseline {
            schema_version: SCHEMA_VERSION,
            stage_id: stage.into(),
            branch_id: branch.into(),
            base_commit: base.into(),
            canonical_task_ids: vec![task.into()],
            commands: vec![CommandBaseline {
                command: "cargo test -p engine grant".into(),
                base_commit: base.into(),
                exit_code: Some(101),
                timed_out: false,
                duration_ms: 1,
                failing_tests: vec!["grant::tests::mine".into(), "plan::tests::theirs".into()],
                tail: Vec::new(),
                error: None,
                cached: false,
            }],
            obligations: vec![BaselineObligation {
                test_id: Some("grant::tests::mine".into()),
                file: Some("crates/engine/src/grant_tests.rs".into()),
                command: "cargo test -p engine grant".into(),
            }],
            routed: vec![RoutedFailure {
                test_id: "plan::tests::theirs".into(),
                file: "crates/engine/src/plan/mod.rs".into(),
                owner_task: "TASK-B".into(),
                command: "cargo test -p engine grant".into(),
            }],
            ignored: Vec::new(),
            inherited: Vec::new(),
        },
    );
}

#[test]
fn the_stamp_is_assembled_from_the_task_records_and_routed_findings_and_lands_on_the_item() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    record(
        &store,
        "agents-1",
        "agents-1-a",
        "1111111111111111",
        "TASK-A",
    );
    route_finding(
        &store,
        "TASK-A",
        json!({"test_id": "other::routed_in", "file": "x.rs", "canonical_task_ids": ["TASK-A"]}),
    );
    let stamp = BaselineStamp::for_tasks(&store, &["TASK-A".to_string()]).expect("stamp");
    assert_eq!(stamp.base_commit, "1111111111111111");
    assert_eq!(
        stamp.must_pass,
        vec![
            "grant::tests::mine".to_string(),
            "other::routed_in".to_string()
        ]
    );
    assert_eq!(stamp.other_owner[0].owner_task, "TASK-B");
    assert_eq!(
        stamp.declared_commands,
        vec!["cargo test -p engine grant".to_string()]
    );
    assert!(BaselineStamp::for_tasks(&store, &["TASK-Z".to_string()]).is_none());

    let mut input =
        json!({"item": {"item_id": "verify-1-check", "canonical_task_ids": ["TASK-A"]}});
    stamp_baseline_tests_input("verification-wave-verify-1", &store, &mut input);
    assert_eq!(stamped(&input), Some(stamp.clone()));
    let mut other = json!({"item": {"canonical_task_ids": ["TASK-A"]}});
    stamp_baseline_tests_input("agents-1", &store, &mut other);
    assert!(
        other.get(BASELINE_TESTS_INPUT_KEY).is_none(),
        "only verification calls are stamped"
    );

    let items = vec![crate::v2::WorkflowV2FanoutItem::read_only(
        "verification-wave-verify-1-verify-1-check".to_string(),
        "coder".to_string(),
        crate::v2::WorkflowV2HostCall {
            id: "verification-wave-verify-1-verify-1-check".into(),
            method: crate::v2::WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        input,
    )];
    let by_item = baseline_by_item(&items);
    assert_eq!(
        by_item.get("verification-wave-verify-1-verify-1-check"),
        Some(&stamp)
    );
}

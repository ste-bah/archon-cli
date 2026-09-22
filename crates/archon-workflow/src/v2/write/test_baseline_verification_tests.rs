//! Issue-70: the baseline is established again at the VERIFICATION base
//! before a focused verification dispatches, so a test another task's later
//! commit broke is routed to that task and exempted for this verifier.
use std::path::Path;

use super::super::test_baseline_verification::{
    VerificationBaselineContext, establish_verification_baseline,
};
use super::super::test_baseline_wave::{WaveBaselineContext, establish_wave};
use super::tests::{Host, head, repository, request, universe};
use super::{load_record, routed_findings_for_task};
use crate::v2::verification::baseline_rule::{
    baseline_by_item, enforce_baseline_tests, stamp_baseline_tests_input, stamped,
};
use crate::v2::{
    WorkflowV2BranchOutcome, WorkflowV2CommandKind, WorkflowV2CommandRecord,
    WorkflowV2CommandStatus, WorkflowV2FanoutItem, WorkflowV2HostCall, WorkflowV2HostMethod,
    WorkflowV2ResultStore, WorkflowV2Status,
};

const CALL: &str = "verification-wave-verify-1";
const ITEM: &str = "verification-wave-verify-1-verify-1-check";
const RED: &str = "test theirs::tests::two ... FAILED\n";

/// A declared command that is green until a commit adds `src/broken`, after
/// which it fails a test in TASK-B's file: the shape of a regression other
/// tasks' commits landed after this task's base.
fn command() -> String {
    format!(": cargo test -p app ; if [ -f src/broken ]; then printf '{RED}'; exit 101; fi; exit 0")
}

fn commit_regression(canonical: &Path) {
    std::fs::write(canonical.join("src/broken"), "by TASK-B\n").unwrap();
    for args in [
        &["add", "."][..],
        &["commit", "-qm", "TASK-B lands and breaks its own test"][..],
    ] {
        crate::write_coordinator::worktree_isolation::run_git(args, canonical).expect("git");
    }
}

fn verification_item(command: &str) -> WorkflowV2FanoutItem {
    WorkflowV2FanoutItem::read_only(
        ITEM,
        "coder",
        WorkflowV2HostCall {
            id: ITEM.into(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        },
        serde_json::json!({"item": {
            "item_id": "verify-1-check",
            "canonical_task_ids": ["TASK-A"],
            "focused_verification": [command],
        }}),
    )
}

fn verifier_outcome(command: &str) -> WorkflowV2BranchOutcome {
    let mut result = crate::WorkflowV2Result::accepted("verified");
    result.commands_run = vec![WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: command.into(),
        status: WorkflowV2CommandStatus::Failed,
        exit_code: Some(101),
        output_summary: RED.into(),
        pre_existing: true,
    }];
    WorkflowV2BranchOutcome {
        item_id: ITEM.into(),
        role: "coder".into(),
        status: WorkflowV2Status::Accepted,
        result: Some(result),
        error: None,
        failure_kind: None,
        item_input_hash: None,
        completion_evidence: Vec::new(),
    }
}

#[tokio::test]
async fn establish_wave_under_a_verification_stage_persists_beside_the_implementation_record() {
    let temp = tempfile::tempdir().unwrap();
    let (canonical, ws) = repository(temp.path());
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let universe = universe();
    let command = command();
    let impl_base = head(&canonical);
    let ctx = WaveBaselineContext {
        store: &store,
        dispatch: &Host,
        universe: Some(&universe),
        stage_id: "agents-1",
        base_commit: &impl_base,
        parallelism: 1,
    };
    let green = establish_wave(&ctx, &[request("agents-1-a", "TASK-A", &command, &ws, &[])]).await;
    assert!(green[0].routed.is_empty() && green[0].must_pass().is_empty());

    commit_regression(&canonical);
    let verification_base = head(&canonical);
    assert_ne!(verification_base, impl_base);
    let ctx = WaveBaselineContext {
        stage_id: CALL,
        base_commit: &verification_base,
        ..ctx
    };
    let red = establish_wave(&ctx, &[request(ITEM, "TASK-A", &command, &canonical, &[])]).await;
    assert_eq!(red[0].routed.len(), 1, "{:?}", red[0]);
    assert_eq!(red[0].routed[0].owner_task, "TASK-B");
    assert!(red[0].must_pass().is_empty());
    // Persisted under the verification stage; the implementation record is
    // still there, untouched.
    let saved = load_record(&store, CALL, ITEM).expect("verification record");
    assert_eq!(saved.base_commit, verification_base);
    assert_eq!(
        load_record(&store, "agents-1", "agents-1-a").as_ref(),
        Some(&green[0])
    );
    let routed = routed_findings_for_task(&store, "TASK-B");
    assert_eq!(routed.len(), 1);
    assert_eq!(routed[0]["test_id"], "theirs::tests::two");
    assert_eq!(
        routed[0]["base_commit"],
        serde_json::json!(verification_base)
    );
    assert_eq!(routed[0]["reported_by"], serde_json::json!(["TASK-A"]));
}

#[tokio::test]
async fn the_verifier_is_restamped_at_the_verification_base_and_its_accepted_verdict_survives() {
    let temp = tempfile::tempdir().unwrap();
    let (canonical, ws) = repository(temp.path());
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let universe = universe();
    let command = command();
    // The implementation wave: green at the task's base.
    let impl_base = head(&canonical);
    establish_wave(
        &WaveBaselineContext {
            store: &store,
            dispatch: &Host,
            universe: Some(&universe),
            stage_id: "agents-1",
            base_commit: &impl_base,
            parallelism: 1,
        },
        &[request("agents-1-a", "TASK-A", &command, &ws, &[])],
    )
    .await;
    // Other work lands; the checkout the verifier runs in moves on.
    commit_regression(&canonical);
    let verification_base = head(&canonical);

    // The item builder's stamp: the implementation record, nothing exempt —
    // the stamp that demoted wf-0ddadd81's TASK-TRADING-001.
    let mut items = vec![verification_item(&command)];
    stamp_baseline_tests_input(CALL, &store, &mut items[0].input);
    let before = stamped(&items[0].input).expect("implementation stamp");
    assert_eq!(before.base_commit, impl_base);
    let mut demoted = vec![verifier_outcome(&command)];
    enforce_baseline_tests(&mut demoted, &baseline_by_item(&items));
    assert_eq!(demoted[0].status, WorkflowV2Status::NeedsReview);

    let ctx = VerificationBaselineContext {
        store: &store,
        dispatch: &Host,
        universe: Some(&universe),
        call_id: CALL,
        repository_root: &canonical,
        parallelism: 1,
    };
    assert_eq!(
        establish_verification_baseline(&ctx, &mut items).await,
        Some(verification_base.clone())
    );
    let after = stamped(&items[0].input).expect("verification stamp");
    assert_eq!(after.base_commit, verification_base);
    assert!(after.verification_base);
    assert_eq!(after.other_owner.len(), 1);
    assert_eq!(after.other_owner[0].test_id, "theirs::tests::two");
    assert_eq!(after.other_owner[0].owner_task, "TASK-B");
    assert!(after.must_pass.is_empty());
    assert_eq!(after.declared_commands, vec![command.clone()]);
    // The prompt names the verification base and the test to ignore.
    let prompt = crate::v2::agent_prompt::baseline_tests_prompt_section(&items[0].input);
    let sha: String = verification_base.chars().take(12).collect();
    assert!(
        prompt.contains(&format!("on the verification base commit {sha}")),
        "{prompt}"
    );
    assert!(
        prompt.contains("theirs::tests::two (owned by TASK-B)"),
        "{prompt}"
    );
    // The same red test in the verifier's own report no longer demotes.
    let mut kept = vec![verifier_outcome(&command)];
    enforce_baseline_tests(&mut kept, &baseline_by_item(&items));
    assert_eq!(kept[0].status, WorkflowV2Status::Accepted, "{:?}", kept[0]);
    // ...and the regression is queued for the task that owns the file.
    let routed = routed_findings_for_task(&store, "TASK-B");
    assert_eq!(routed.len(), 1);
    assert_eq!(
        routed[0]["canonical_task_ids"],
        serde_json::json!(["TASK-B"])
    );
    assert_eq!(routed[0]["finding_scope"], "baseline_regression");

    // A call that is not a focused verification is left alone.
    let mut other = vec![verification_item(&command)];
    let ctx = VerificationBaselineContext {
        call_id: "agents-2",
        ..ctx
    };
    assert_eq!(
        establish_verification_baseline(&ctx, &mut other).await,
        None
    );
    assert!(stamped(&other[0].input).is_none());
}

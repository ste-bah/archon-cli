//! Issue-118: a red test the judged branch cannot write, failing the same
//! way since the run began by the host's own two runs, refuses nothing --
//! and every other red test still refuses.
use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};

use super::{
    ExcusedRedTests, RunBaseRedContext, UNOWNED_RED_DATA_KEY, UNOWNED_RED_GAP_ID,
    excuse_run_base_red_tests, grouped_names,
};
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use crate::v2::verification::baseline_rule::{
    BASELINE_RED_TEST_GAP_ID, BaselineStamp, enforce_baseline_tests,
    enforce_baseline_tests_excusing,
};
use crate::v2::verification::unowned_paths::BranchScope;
use crate::v2::write::test_baseline_run_base::tests::{
    FakeCargo, bind_run, commit_files, head, runs, world,
};
use crate::v2::{
    WorkflowV2BranchOutcome, WorkflowV2CommandKind, WorkflowV2CommandRecord,
    WorkflowV2CommandStatus, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2ResultStore,
    WorkflowV2Status,
};

const ITEM: &str = "verification-wave-review-verify-c-residual-1-0";
const LIB: &str = "cargo test -p app --lib";
const OLD: &str = "shared::tests::old";
const NEW: &str = "shared::tests::new";

fn universe(own: &[&str]) -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-A".into(),
            source_path: "tasks/TASK-A.md".into(),
            files_expected_to_change: own.iter().map(|f| f.to_string()).collect(),
            ..Default::default()
        }],
    }
}

fn stamp() -> BaselineStamp {
    BaselineStamp {
        base_commit: "verificationbase".into(),
        tasks: vec!["TASK-A".into()],
        declared_commands: vec!["mcp__server__get_errors".into(), LIB.into()],
        ..Default::default()
    }
}

fn command(text: &str, pre_existing: bool) -> WorkflowV2CommandRecord {
    WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: text.into(),
        status: WorkflowV2CommandStatus::Failed,
        exit_code: Some(101),
        output_summary: "7 failed; all outside this task's scope and pre-existing".into(),
        pre_existing,
    }
}

/// A residual round's verifier, as on wf-0ddadd81: it accepted, ran the
/// crate's whole library suite, named the red tests only in its TYPED list,
/// and marked the runner (and any `extra` command) `pre_existing`.
fn verdict(failed: &[&str], extra: Vec<WorkflowV2CommandRecord>) -> WorkflowV2BranchOutcome {
    let mut result =
        crate::WorkflowV2Result::accepted("gap resolved; library tests fail elsewhere");
    result.commands_run = vec![command(LIB, true)];
    result.commands_run.extend(extra);
    result.data = json!({"matched_test_check_names": {"failed": failed}});
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

fn call(granted: &[&str]) -> WorkflowV2HostCall {
    let mut options = crate::v2::WorkflowV2HostOptions::default();
    options.extra.insert(
        "remediationContract".into(),
        json!({"stage": "verify", "residual": {"key": "residual-1", "files": granted}}),
    );
    WorkflowV2HostCall {
        id: "verification-wave-review-verify-c-residual-1".into(),
        method: WorkflowV2HostMethod::Parallel,
        write_mode: None,
        options,
    }
}

struct World {
    _temp: tempfile::TempDir,
    repo: PathBuf,
    host: FakeCargo,
    counter: PathBuf,
    store: WorkflowV2ResultStore,
}

fn setup(bound: bool) -> World {
    let temp = tempfile::tempdir().unwrap();
    let (repo, base, host, counter) = world(temp.path());
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    if bound {
        bind_run(&store, &base);
    }
    World {
        _temp: temp,
        repo,
        host,
        counter,
        store,
    }
}

/// The live order: the host's two runs, then the rule with what they proved.
async fn judge_in(
    w: &World,
    outcome: WorkflowV2BranchOutcome,
    own: &[&str],
    granted: &[&str],
) -> WorkflowV2BranchOutcome {
    let universe = universe(own);
    let by_item = BTreeMap::from([(ITEM.to_string(), stamp())]);
    let scope = BTreeMap::from([(ITEM.to_string(), BranchScope::default())]);
    let mut outcomes = vec![outcome];
    let call = call(granted);
    let judged = head(&w.repo);
    let excused: ExcusedRedTests = excuse_run_base_red_tests(
        &RunBaseRedContext {
            store: &w.store,
            dispatch: &w.host,
            universe: Some(&universe),
            repository_root: &w.repo,
            call: &call,
            judged_commit: Some(&judged),
        },
        &outcomes,
        &by_item,
        &scope,
    )
    .await;
    enforce_baseline_tests_excusing(&mut outcomes, &by_item, &excused);
    outcomes.remove(0)
}

async fn judge(
    outcome: WorkflowV2BranchOutcome,
    own: &[&str],
    granted: &[&str],
) -> (WorkflowV2BranchOutcome, usize) {
    let w = setup(true);
    let judged = judge_in(&w, outcome, own, granted).await;
    (judged, runs(&w.counter))
}

fn gap_ids(outcome: &WorkflowV2BranchOutcome) -> Vec<String> {
    let result = outcome.result.as_ref().unwrap();
    result.residual_gaps.iter().map(|g| g.id.clone()).collect()
}

fn excused(outcome: &WorkflowV2BranchOutcome) -> bool {
    gap_ids(outcome)
        .iter()
        .any(|id| id.starts_with(UNOWNED_RED_GAP_ID))
}

fn refused(outcome: &WorkflowV2BranchOutcome) -> bool {
    outcome.status == WorkflowV2Status::NeedsReview
        && gap_ids(outcome).contains(&BASELINE_RED_TEST_GAP_ID.to_string())
}

fn data(outcome: &WorkflowV2BranchOutcome) -> Value {
    outcome.result.as_ref().unwrap().data.clone()
}

#[tokio::test]
async fn an_unowned_test_failing_the_same_way_since_the_run_began_refuses_nothing() {
    let (outcome, runs) = judge(verdict(&[OLD], vec![]), &["src/mine.rs"], &[]).await;
    assert_eq!(outcome.status, WorkflowV2Status::Accepted, "{outcome:?}");
    assert_eq!(runs, 2, "the host ran the verifier's runner on both trees");
    assert!(excused(&outcome));
    let listed = &data(&outcome)[UNOWNED_RED_DATA_KEY][0];
    assert_eq!(listed["test_id"], OLD);
    assert_eq!(listed["file"], "src/shared_tests.rs");
    // The test's file and its parent module's (its panic is in its own).
    assert_eq!(
        listed["files"],
        json!(["src/shared_tests.rs", "src/shared.rs"])
    );
    let gap = &outcome.result.as_ref().unwrap().residual_gaps[0];
    assert_eq!(gap.severity.as_deref(), Some("medium"));
    assert!(
        gap.description.contains("src/shared_tests.rs"),
        "{}",
        gap.description
    );
}

#[tokio::test]
async fn the_old_code_refused_the_same_verdict() {
    let mut outcomes = vec![verdict(&[OLD], vec![])];
    enforce_baseline_tests(
        &mut outcomes,
        &BTreeMap::from([(ITEM.to_string(), stamp())]),
    );
    assert!(refused(&outcomes[0]));
}

#[tokio::test]
async fn the_same_test_inside_the_rounds_scope_still_refuses() {
    for (own, granted) in [
        (&["src/mine.rs", "src/shared_tests.rs"][..], &[][..]),
        (&["src/mine.rs"][..], &["src/shared_tests.rs"][..]),
    ] {
        let (outcome, _) = judge(verdict(&[OLD], vec![]), own, granted).await;
        assert!(refused(&outcome), "{own:?} {granted:?}");
        assert!(!excused(&outcome));
    }
}

#[tokio::test]
async fn a_test_green_at_the_run_base_is_a_new_failure_and_still_refuses() {
    let w = setup(true);
    commit_files(&w.repo, &["src/new_red"], "a task breaks another test");
    let outcome = judge_in(&w, verdict(&[OLD, NEW], vec![]), &["src/mine.rs"], &[]).await;
    assert!(refused(&outcome));
    assert_eq!(data(&outcome)["baseline_red_tests"], json!([NEW]));
    // The old one is still excused and still owed.
    assert!(excused(&outcome));
}

#[tokio::test]
async fn a_test_failing_differently_than_at_the_run_base_still_refuses() {
    let w = setup(true);
    commit_files(
        &w.repo,
        &["src/old_changed"],
        "the old test now fails for another reason",
    );
    let outcome = judge_in(&w, verdict(&[OLD], vec![]), &["src/mine.rs"], &[]).await;
    assert!(refused(&outcome));
    assert!(!excused(&outcome));
}

#[tokio::test]
async fn a_typed_only_claim_the_hosts_run_does_not_bear_out_still_refuses() {
    // The verifier names a test the host's judged run never saw fail: it
    // stays red on the report and refuses.
    let (outcome, _) = judge(
        verdict(&["shared::tests::ghost"], vec![]),
        &["src/mine.rs"],
        &[],
    )
    .await;
    assert!(refused(&outcome));
    // A report that names only OLD while the host's own run also fails NEW
    // (a new failure it left out): the runner's claim is not proven.
    let w = setup(true);
    commit_files(&w.repo, &["src/new_red"], "a task breaks another test");
    let under = judge_in(&w, verdict(&[OLD], vec![]), &["src/mine.rs"], &[]).await;
    assert_eq!(under.status, WorkflowV2Status::NeedsReview);
    assert_eq!(data(&under)["baseline_unproven_pre_existing"], json!([LIB]));
}

#[tokio::test]
async fn a_bare_pre_existing_claim_on_any_other_command_is_still_rejected() {
    let w = setup(true);
    let tool = command("mcp__server__get_errors()", true);
    let outcome = judge_in(&w, verdict(&[OLD], vec![tool]), &["src/mine.rs"], &[]).await;
    assert_eq!(outcome.status, WorkflowV2Status::NeedsReview);
    assert_eq!(
        data(&outcome)["baseline_unproven_pre_existing"],
        json!(["mcp__server__get_errors()"]),
        "the runner's claim is proven by the host's run; the tool call's is not"
    );
    assert_eq!(data(&outcome)["baseline_red_tests"], json!([]));
    // Without the tool call the same report is accepted.
    let alone = judge_in(&w, verdict(&[OLD], vec![]), &["src/mine.rs"], &[]).await;
    assert_eq!(alone.status, WorkflowV2Status::Accepted, "{alone:?}");
}

#[tokio::test]
async fn nothing_is_excused_without_a_recorded_run_base() {
    let w = setup(false);
    let outcome = judge_in(&w, verdict(&[OLD], vec![]), &["src/mine.rs"], &[]).await;
    assert!(refused(&outcome));
    assert_eq!(runs(&w.counter), 0);
}

#[test]
fn red_test_names_are_grouped_under_their_module_so_none_is_cut() {
    let ids: Vec<String> = ["a::t::one", "a::t::two", "b::three", "root"]
        .iter()
        .map(|id| id.to_string())
        .collect();
    assert_eq!(
        grouped_names(&ids),
        "`a::t::{one, two}`, `b::three`, `root`"
    );
}

#[tokio::test]
async fn nothing_is_run_when_no_red_test_or_runner_claim_needs_the_host() {
    let w = setup(true);
    let mut quiet = verdict(&[], vec![]);
    quiet.result.as_mut().unwrap().commands_run[0].pre_existing = false;
    let outcome = judge_in(&w, quiet, &["src/mine.rs"], &[]).await;
    assert_eq!(runs(&w.counter), 0);
    assert!(!excused(&outcome));
}

#[tokio::test]
async fn a_failure_no_test_id_names_leaves_the_runner_claim_unproven() {
    // A doc test fails beside OLD: the harness counts two failures, the
    // host can name one, so nothing proves the runner's claim.
    let w = setup(true);
    commit_files(&w.repo, &["src/doc_red"], "a doc example breaks");
    let outcome = judge_in(&w, verdict(&[OLD], vec![]), &["src/mine.rs"], &[]).await;
    assert_eq!(outcome.status, WorkflowV2Status::NeedsReview, "{outcome:?}");
    assert_eq!(
        data(&outcome)["baseline_unproven_pre_existing"],
        json!([LIB])
    );
}

#[tokio::test]
async fn a_test_failing_differently_under_another_command_is_not_excused_by_the_first() {
    let w = setup(true);
    commit_files(
        &w.repo,
        &["src/other_changed"],
        "the other binary fails it anew",
    );
    let other = command("cargo test -p app --test it", true);
    let outcome = judge_in(&w, verdict(&[OLD], vec![other]), &["src/mine.rs"], &[]).await;
    assert!(refused(&outcome), "{outcome:?}");
    assert!(!excused(&outcome));
}

#[tokio::test]
async fn a_failure_named_by_a_command_the_host_does_not_run_is_not_excused() {
    // The same id red in a piped run the host never runs: that run may be
    // failing it another way, so the host's own two runs excuse nothing.
    let w = setup(true);
    let mut piped = command("cargo test -p cli 2>&1 | tail -50", false);
    piped.output_summary = format!("test {OLD} ... FAILED\n");
    let outcome = judge_in(&w, verdict(&[OLD], vec![piped]), &["src/mine.rs"], &[]).await;
    assert!(refused(&outcome), "{outcome:?}");
    assert!(!excused(&outcome));
}

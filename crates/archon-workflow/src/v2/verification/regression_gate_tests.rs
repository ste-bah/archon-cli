//! Issue-114: a test failing at the final tip that did not fail at the run
//! base blocks; one failing at both is listed as pre-existing.
use super::{RegressionGate, declared_test_commands, regression_verdict};
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use crate::v2::WorkflowV2ResultStore;
use crate::v2::write::test_baseline_run_base::tests::{bind_run, commit_files, runs, world};
use crate::v2::write::test_baseline_run_base::{Tree, cached};

const LIB: &str = "cargo test -p app --lib";

fn universe(commands: &[&str]) -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: vec![],
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-S".into(),
            source_path: "tasks/TASK-S.md".into(),
            files_expected_to_change: vec!["src/shared_tests.rs".into()],
            focused_tests: commands.iter().map(|c| c.to_string()).collect(),
            ..Default::default()
        }],
    }
}

#[tokio::test]
async fn a_test_failing_only_at_the_tip_blocks_and_one_failing_at_both_is_listed() {
    let dir = tempfile::tempdir().unwrap();
    let (repo, base, cargo, counter) = world(dir.path());
    commit_files(&repo, &["src/new_red"], "a landing breaks a test");
    let store = WorkflowV2ResultStore::new(dir.path().join("run/v2"));
    bind_run(&store, &base);
    let universe = universe(&[LIB, "echo not a runner"]);
    let gate = RegressionGate {
        store: &store,
        dispatch: &cargo,
        universe: Some(&universe),
        repository_root: &repo,
    };
    let verdict = regression_verdict(&gate).await;
    // Batch O2: a command that is no plain cargo runner is COMPARED too --
    // run at both trees (it passes at both, so it blocks nothing) -- never
    // a NOT-COMPARED note.
    assert_eq!(verdict.blocking.len(), 1, "{verdict:#?}");
    assert!(
        !verdict
            .notes
            .iter()
            .any(|note| note.contains("NOT COMPARED")),
        "{verdict:#?}"
    );
    let tip = crate::repository_record::git_head(&repo).unwrap();
    for commit in [&base, &tip] {
        let run = cached(&store, Tree::RunBase, commit, "echo not a runner");
        assert_eq!(run.map(|run| run.exit_code), Some(Some(0)), "{commit}");
    }
    let clause = &verdict.blocking[0];
    assert!(
        clause.contains("shared::tests::new")
            && clause.contains("did not fail at the run base")
            && clause.contains("declared by TASK-S")
            && clause.contains("harness cap exhausted"),
        "{clause}"
    );
    assert!(
        verdict
            .notes
            .iter()
            .any(|n| n.starts_with("PRE-EXISTING") && n.contains("shared::tests::old")),
        "{verdict:#?}"
    );
    // Both trees are cached: judging again runs nothing.
    let ran = runs(&counter);
    assert_eq!(regression_verdict(&gate).await, verdict);
    assert_eq!(runs(&counter), ran);
}

#[tokio::test]
async fn nothing_new_red_blocks_nothing_and_pre_existing_failures_are_listed() {
    let dir = tempfile::tempdir().unwrap();
    let (repo, base, cargo, _) = world(dir.path());
    let store = WorkflowV2ResultStore::new(dir.path().join("run/v2"));
    bind_run(&store, &base);
    let universe = universe(&[LIB]);
    let verdict = regression_verdict(&RegressionGate {
        store: &store,
        dispatch: &cargo,
        universe: Some(&universe),
        repository_root: &repo,
    })
    .await;
    assert!(verdict.blocking.is_empty(), "{verdict:#?}");
    assert!(
        verdict
            .notes
            .iter()
            .any(|n| n.starts_with("PRE-EXISTING") && n.contains("shared::tests::old")),
        "{verdict:#?}"
    );
}

#[tokio::test]
async fn a_tip_with_no_verdict_blocks_and_an_unrecorded_base_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let (repo, base, cargo, _) = world(dir.path());
    commit_files(&repo, &["src/no_summary"], "a landing breaks the build");
    let store = WorkflowV2ResultStore::new(dir.path().join("run/v2"));
    let universe = universe(&[LIB]);
    let gate = RegressionGate {
        store: &store,
        dispatch: &cargo,
        universe: Some(&universe),
        repository_root: &repo,
    };
    // No `repository_bound` event: no base to compare with.
    let verdict = regression_verdict(&gate).await;
    assert!(
        verdict.blocking[0].contains("base commit is not recorded"),
        "{verdict:#?}"
    );
    bind_run(&store, &base);
    let verdict = regression_verdict(&gate).await;
    assert!(
        verdict
            .blocking
            .iter()
            .any(|b| b.contains("gave no verdict at the final tip")),
        "{verdict:#?}"
    );
}

/// Batch O: every declared command, however many, with no cap; Batch O2:
/// every runner's, not only the plain cargo ones.
#[test]
fn the_declared_commands_are_every_deduplicated_one() {
    let many: Vec<String> = (0..40)
        .map(|n| format!("cargo test -p app --test t{n:02}"))
        .collect();
    let mut declared: Vec<&str> = many.iter().map(String::as_str).collect();
    declared.extend([
        LIB,
        LIB,
        "mcp__tool__check()",
        "cargo test -p app; rm -rf x",
    ]);
    let commands = declared_test_commands(&universe(&declared));
    assert_eq!(commands.len(), 43);
    assert!(commands.iter().any(|c| c == "mcp__tool__check()"));
    assert!(commands.windows(2).all(|pair| pair[0] < pair[1]));
}

#[tokio::test]
async fn a_command_with_no_verdict_at_either_tree_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let (repo, base, cargo, _) = world(dir.path());
    // Red at the base too: the harness never reports.
    std::fs::write(repo.join("src/no_summary"), "x").unwrap();
    crate::write_coordinator::worktree_isolation::run_git(&["add", "."], &repo).unwrap();
    crate::write_coordinator::worktree_isolation::run_git(&["commit", "-qm", "tip"], &repo)
        .unwrap();
    let store = WorkflowV2ResultStore::new(dir.path().join("run/v2"));
    bind_run(&store, &base);
    let universe = universe(&[LIB, "npm run check"]);
    let verdict = regression_verdict(&RegressionGate {
        store: &store,
        dispatch: &cargo,
        universe: Some(&universe),
        repository_root: &repo,
    })
    .await;
    assert!(
        verdict
            .blocking
            .iter()
            .any(|b| b.contains("gave no verdict at the final tip")),
        "{verdict:#?}"
    );
    // Batch O2: a declared command of another runner is compared, never a
    // NOT-COMPARED note. Failing at the base and the tip alike and naming no
    // test (m3), it is labelled NOT JUDGEABLE -- never read as pre-existing
    // or passed -- and blocks nothing of its own.
    assert!(
        verdict
            .notes
            .iter()
            .any(|n| n.starts_with("NOT JUDGEABLE") && n.contains("npm run check")),
        "{verdict:#?}"
    );
    assert!(
        !verdict
            .notes
            .iter()
            .any(|n| n.starts_with("PRE-EXISTING") && n.contains("npm run check")),
        "{verdict:#?}"
    );
    assert!(
        !verdict.notes.iter().any(|n| n.contains("NOT COMPARED")),
        "{verdict:#?}"
    );
}

/// A test the base passed that the tip reports ignored was hidden: it
/// blocks. Batch O: one the tip no longer reports is gone, and blocks too.
#[tokio::test]
async fn a_test_ignored_at_the_tip_that_passed_at_the_base_blocks() {
    use crate::v2::write::test_baseline_run_base::{HostRunVerdict, Tree, cache, tests::head};
    let dir = tempfile::tempdir().unwrap();
    let (repo, base, cargo, counter) = world(dir.path());
    let tip = head(&repo);
    let store = WorkflowV2ResultStore::new(dir.path().join("run/v2"));
    bind_run(&store, &base);
    let verdict_at = |commit: &str, passed: &[&str], ignored: &[&str]| HostRunVerdict {
        command: LIB.into(),
        commit: commit.into(),
        exit_code: Some(0),
        passed_tests: passed.iter().map(|t| t.to_string()).collect(),
        ignored_tests: ignored.iter().map(|t| t.to_string()).collect(),
        failed_count: Some(0),
        ids_kept: true,
        ..Default::default()
    };
    cache(
        &store,
        Tree::RunBase,
        &verdict_at(&base, &["a::kept", "a::hidden", "a::renamed"], &[]),
    );
    cache(
        &store,
        Tree::RunBase,
        &verdict_at(&tip, &["a::kept", "a::renamed_now"], &["a::hidden"]),
    );
    let universe = universe(&[LIB]);
    let verdict = regression_verdict(&RegressionGate {
        store: &store,
        dispatch: &cargo,
        universe: Some(&universe),
        repository_root: &repo,
    })
    .await;
    assert_eq!(runs(&counter), 0, "both verdicts came from the cache");
    assert_eq!(verdict.blocking.len(), 2, "{verdict:#?}");
    assert!(
        verdict.blocking[0].contains("a::hidden") && verdict.blocking[0].contains("ignored"),
        "{verdict:#?}"
    );
    assert!(
        verdict.blocking[1].contains("`a::renamed`")
            && verdict.blocking[1].contains("not reported"),
        "{verdict:#?}"
    );
    assert!(
        !verdict.blocking.iter().any(|b| b.contains("a::kept")),
        "{verdict:#?}"
    );
}

/// Batch O: a tip that reports no passing test at all still shows every
/// test the base passed as gone; the old check skipped such a tip.
#[tokio::test]
async fn every_test_gone_from_a_tip_that_passes_nothing_blocks() {
    use crate::v2::write::test_baseline_run_base::{HostRunVerdict, Tree, cache, tests::head};
    let dir = tempfile::tempdir().unwrap();
    let (repo, base, cargo, _) = world(dir.path());
    let tip = head(&repo);
    let store = WorkflowV2ResultStore::new(dir.path().join("run/v2"));
    bind_run(&store, &base);
    let verdict_at = |commit: &str, passed: &[&str]| HostRunVerdict {
        command: LIB.into(),
        commit: commit.into(),
        exit_code: Some(0),
        passed_tests: passed.iter().map(|t| t.to_string()).collect(),
        failed_count: Some(0),
        ids_kept: true,
        ..Default::default()
    };
    cache(
        &store,
        Tree::RunBase,
        &verdict_at(&base, &["a::one", "a::two"]),
    );
    cache(&store, Tree::RunBase, &verdict_at(&tip, &[]));
    let universe = universe(&[LIB]);
    let verdict = regression_verdict(&RegressionGate {
        store: &store,
        dispatch: &cargo,
        universe: Some(&universe),
        repository_root: &repo,
    })
    .await;
    assert_eq!(verdict.blocking.len(), 2, "{verdict:#?}");
    assert!(
        verdict
            .blocking
            .iter()
            .all(|b| b.contains("not reported at the final tip")),
        "{verdict:#?}"
    );
}

/// Every command a recorded HIGH gap owes a test in runs at the tip, even
/// one no task declares, so the residual gate can judge the gap there.
#[tokio::test]
async fn a_high_gaps_owed_command_runs_at_the_tip() {
    use crate::v2::WorkflowV2Status;
    use crate::v2::write::test_baseline_run_base::{Tree, cached, tests::head};
    use crate::{
        WorkflowV2CallRecord, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions,
    };
    let dir = tempfile::tempdir().unwrap();
    let (repo, base, cargo, counter) = world(dir.path());
    let store = WorkflowV2ResultStore::new(dir.path().join("run/v2"));
    bind_run(&store, &base);
    let stage = "verification-wave-review-verify-x-1-2";
    let mut options = WorkflowV2HostOptions::default();
    options.extra.insert(
        "remediationContract".into(),
        serde_json::json!({"version": 1, "stage": "verify", "taskId": "TASK-S", "round": 1}),
    );
    let mut result = crate::WorkflowV2Result {
        status: WorkflowV2Status::NeedsReview,
        summary: "refused".into(),
        ..Default::default()
    };
    result.residual_gaps.push(crate::WorkflowV2ResidualGap {
        id: "gap-old".into(),
        description: "shared::tests::old is red".into(),
        severity: Some("high".into()),
    });
    let call = WorkflowV2HostCall {
        id: stage.into(),
        method: WorkflowV2HostMethod::Parallel,
        write_mode: None,
        options,
    };
    store
        .save_call_record(&WorkflowV2CallRecord::new(
            "run",
            call,
            1,
            "h".into(),
            result,
            vec![],
        ))
        .unwrap();
    let record: crate::v2::write::test_baseline::BranchBaseline = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "stage_id": stage, "branch_id": format!("{stage}-0"), "base_commit": base,
        "canonical_task_ids": ["TASK-S"], "commands": [{"command": LIB, "base_commit": base,
            "exit_code": 101, "timed_out": false, "duration_ms": 1,
            "failing_tests": ["shared::tests::old"], "cached": false}]}))
    .unwrap();
    crate::v2::write::test_baseline::save_record(&store, &record);
    // No task declares any command: only the gap's owed one runs.
    let universe = universe(&[]);
    regression_verdict(&RegressionGate {
        store: &store,
        dispatch: &cargo,
        universe: Some(&universe),
        repository_root: &repo,
    })
    .await;
    assert!(runs(&counter) > 0);
    let tip = head(&repo);
    let verdict = cached(&store, Tree::RunBase, &tip, LIB).expect("the tip verdict");
    assert_eq!(verdict.failing_tests, ["shared::tests::old"]);
}

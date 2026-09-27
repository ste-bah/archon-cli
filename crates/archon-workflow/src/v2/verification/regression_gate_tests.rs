//! Issue-114: a test failing at the final tip that did not fail at the run
//! base blocks; one failing at both is listed as pre-existing.
use super::{MAX_REGRESSION_COMMANDS, RegressionGate, declared_test_commands, regression_verdict};
use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use crate::v2::WorkflowV2ResultStore;
use crate::v2::write::test_baseline_run_base::tests::{bind_run, commit_files, runs, world};

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
    assert_eq!(verdict.blocking.len(), 1, "{verdict:#?}");
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

#[test]
fn the_declared_commands_are_the_deduplicated_runnable_ones_under_a_cap() {
    let many: Vec<String> = (0..MAX_REGRESSION_COMMANDS + 3)
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
    assert_eq!(commands.len(), MAX_REGRESSION_COMMANDS + 4);
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
    assert!(
        verdict
            .notes
            .iter()
            .any(|n| n.contains("were not compared") && n.contains("npm run check")),
        "{verdict:#?}"
    );
}

/// A test the base passed that the tip reports ignored was hidden: it
/// blocks. One the tip no longer reports is a warning.
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
        ..Default::default()
    };
    cache(
        &store,
        Tree::RunBase,
        &verdict_at(&base, &["a::kept", "a::hidden", "a::renamed"], &[]),
    );
    cache(
        &store,
        Tree::Judged,
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
    assert_eq!(verdict.blocking.len(), 1, "{verdict:#?}");
    assert!(
        verdict.blocking[0].contains("a::hidden") && verdict.blocking[0].contains("ignored"),
        "{verdict:#?}"
    );
    assert!(
        verdict
            .notes
            .iter()
            .any(|n| n.contains("a::renamed") && n.contains("not reported")),
        "{verdict:#?}"
    );
}

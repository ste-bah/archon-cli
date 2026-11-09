//! Real child harnesses stop between binaries after two failures.
use super::*;
use crate::v2::WorkflowV2ResultStore;
use crate::v2::write::test_baseline_run_base::tests::{freeze_runner_clock, head, world};
use crate::v2::write::test_baseline_run_base::{FrozenCommands, Tree, host_verdicts};

#[test]
fn regression_cached_count_only_footer_does_not_excuse_unnamed_failure() {
    use crate::v2::write::test_baseline_run_base::{cache, cached};
    let root = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(root.path().join("run/v2"));
    let footer = include_str!("../../../../../tests/fixtures/diagnostic_outputs/rustc-a.txt")
        .lines()
        .last()
        .unwrap();
    let run = HostRunVerdict {
        command: "check".into(),
        commit: "base".into(),
        exit_code: Some(1),
        verbatim: true,
        isolated: true,
        ids_kept: true,
        signatures: [(String::new(), footer.into())].into(),
        ..Default::default()
    };
    cache(&store, Tree::RunBase, &run);
    let cached = cached(&store, Tree::RunBase, "base", "check").unwrap();
    assert_eq!(cached.exit_code, Some(1));
    assert!(!cached.signatures.contains_key(""));
    let before = [("check".into(), cached.clone())].into();
    let after = [("check".into(), cached)].into();
    assert!(
        compare(&["check".into()], &before, &after)
            .iter()
            .any(RegressionFinding::blocks)
    );
}

#[test]
#[ignore = "child harness failure fixture"]
fn base_child_one() {
    panic!("first base failure");
}
#[test]
#[ignore = "child harness failure fixture"]
fn base_child_two() {
    panic!("second base failure");
}
#[test]
#[ignore = "child harness failure fixture"]
fn tip_child_one() {
    panic!("first tip failure");
}
#[test]
#[ignore = "child harness failure fixture"]
fn tip_child_two() {
    panic!("second tip failure");
}

#[tokio::test]
async fn fail_fast_subsets_are_unknown_and_block_tip_red() {
    let _clock = freeze_runner_clock();
    let temp = tempfile::tempdir().unwrap();
    let (root, _, host, _) = world(temp.path());
    let exe = std::env::current_exe().unwrap();
    let script = format!(
        "'{}' --ignored \"$(cat selector)\" && touch later-binary-ran\n",
        exe.display().to_string().replace('\'', "'\\''")
    );
    std::fs::write(root.join("run-check"), script).unwrap();
    let git = |args: &[&str]| {
        crate::write_coordinator::worktree_isolation::run_git(args, &root).unwrap();
    };
    std::fs::write(
        root.join("selector"),
        "regression_fail_fast_tests::base_child_",
    )
    .unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "base selection"]);
    let base = head(&root);
    std::fs::write(
        root.join("selector"),
        "regression_fail_fast_tests::tip_child_",
    )
    .unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "tip selection"]);
    let tip = head(&root);
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let commands = vec!["sh run-check".to_string()];
    let frozen = FrozenCommands::from_commands(&commands);
    let before = host_verdicts(
        &store,
        &host,
        &root,
        Tree::RunBase,
        &base,
        &commands,
        &frozen,
    )
    .await;
    let after = host_verdicts(&store, &host, &root, Tree::Judged, &tip, &commands, &frozen).await;
    for runs in [&before, &after] {
        assert_eq!(runs[&commands[0]].failing_tests.len(), 2);
        assert_eq!(runs[&commands[0]].failed_count, Some(2));
        assert_ne!(runs[&commands[0]].exit_code, Some(0));
    }
    assert!(!root.join("later-binary-ran").exists());
    let findings = compare(&commands, &before, &after);
    assert_eq!(findings.len(), 2, "{findings:?}");
    assert!(
        findings
            .iter()
            .all(|f| matches!(f, RegressionFinding::Unknown { .. }))
    );
    assert!(findings.iter().all(RegressionFinding::blocks));
    use super::super::baseline_rule::{BaselineStamp, enforce_baseline_tests_excusing};
    use super::super::baseline_run_base::{RunBaseRedContext, excuse_run_base_red_tests};
    use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
    use crate::v2::{
        WorkflowV2BranchOutcome, WorkflowV2CommandKind, WorkflowV2CommandRecord,
        WorkflowV2CommandStatus, WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2Status,
    };
    crate::v2::write::test_baseline_run_base::tests::bind_run(&store, &base);
    let universe = WorkflowV2TaskUniverse {
        tasks: vec![WorkflowV2TaskUniverseTask {
            canonical_task_id: "TASK-A".into(),
            focused_commands: Some(commands.clone()),
            focused_tests: commands.clone(),
            ..Default::default()
        }],
        schema_version: "test".into(),
        source_roots: vec![],
    };
    let stamp = BaselineStamp {
        declared_commands: commands.clone(),
        tasks: vec!["TASK-A".into()],
        ..Default::default()
    };
    let stamps = BTreeMap::from([("branch".into(), stamp)]);
    let mut result = crate::WorkflowV2Result::accepted("child harness completed");
    result.commands_run = vec![WorkflowV2CommandRecord {
        command: commands[0].clone(),
        kind: WorkflowV2CommandKind::Test,
        status: WorkflowV2CommandStatus::Failed,
        exit_code: Some(101),
        output_summary: String::new(),
        pre_existing: false,
    }];
    result.data = serde_json::json!({"matched_test_check_names": {"failed": after[&commands[0]].failing_tests}});
    let mut outcomes = vec![WorkflowV2BranchOutcome {
        item_id: "branch".into(),
        role: "coder".into(),
        status: WorkflowV2Status::Accepted,
        result: Some(result),
        error: None,
        failure_kind: None,
        item_input_hash: None,
        completion_evidence: vec![],
    }];
    let call = WorkflowV2HostCall {
        id: "verify".into(),
        method: WorkflowV2HostMethod::Parallel,
        write_mode: None,
        options: Default::default(),
    };
    let judgement = excuse_run_base_red_tests(
        &RunBaseRedContext {
            store: &store,
            dispatch: &host,
            universe: Some(&universe),
            repository_root: &root,
            call: &call,
            judged_commit: Some(&tip),
        },
        &outcomes,
        &stamps,
        &BTreeMap::new(),
    )
    .await;
    assert_eq!(judgement["branch"].unknown.len(), 2);
    assert!(judgement["branch"].excused.is_empty());
    enforce_baseline_tests_excusing(&mut outcomes, &stamps, &judgement);
    assert_eq!(outcomes[0].status, WorkflowV2Status::NeedsReview);
    assert!(
        outcomes[0]
            .result
            .as_ref()
            .unwrap()
            .residual_gaps
            .iter()
            .any(|gap| gap.id == super::super::baseline_rule::BASELINE_RED_TEST_GAP_ID)
    );
    assert_eq!(
        outcomes[0].result.as_ref().unwrap().data["baseline_unknown_tests"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // A passing base id omitted by a failing tip was not observed there.
    let mut before = before;
    before
        .get_mut(&commands[0])
        .unwrap()
        .passed_tests
        .push("later::passed".into());
    assert!(
        compare(&commands, &before, &after)
            .iter()
            .any(|f| matches!(f, RegressionFinding::Vanished { .. }))
    );
}

#[test]
fn observed_pass_to_fail_and_unseen_red_ids_block() {
    for (seen, blocks) in [(true, true), (false, true)] {
        let command = "check".to_string();
        let base = HostRunVerdict {
            passed_tests: if seen { vec!["case".into()] } else { vec![] },
            ids_kept: true,
            ..Default::default()
        };
        let tip = HostRunVerdict {
            failing_tests: vec!["case".into()],
            failed_count: Some(1),
            ids_kept: true,
            exit_code: Some(1),
            ..Default::default()
        };
        let findings = compare(
            std::slice::from_ref(&command),
            &BTreeMap::from([(command.clone(), base)]),
            &BTreeMap::from([(command.clone(), tip)]),
        );
        assert_eq!(findings.iter().any(RegressionFinding::blocks), blocks);
    }
}

#[test]
fn anonymous_failures_require_the_same_output_identity() {
    for (identity, expected) in [(Some("old"), "same"), (Some("new"), "new"), (None, "new")] {
        let verdict = |identity: Option<&str>| HostRunVerdict {
            exit_code: Some(1),
            ids_kept: true,
            signatures: identity
                .map(|value| (String::new(), value.to_string()))
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let findings = compare(
            &["check".into()],
            &BTreeMap::from([("check".into(), verdict(Some("old")))]),
            &BTreeMap::from([("check".into(), verdict(identity))]),
        );
        assert_eq!(findings.len(), 1);
        assert!(match expected {
            "same" => matches!(findings[0], RegressionFinding::UnnamedPreExisting { .. }),
            "new" => matches!(findings[0], RegressionFinding::UnnamedNew { .. }),
            _ => matches!(findings[0], RegressionFinding::NotJudgeable { .. }),
        });
    }
}

#[test]
fn skipped_at_base_and_missing_base_identity_never_excuse_tip_red() {
    let tip = HostRunVerdict {
        exit_code: Some(1),
        failing_tests: vec!["x".into()],
        failed_count: Some(1),
        ids_kept: true,
        ..Default::default()
    };
    for base in [
        None,
        Some(HostRunVerdict {
            exit_code: Some(1),
            ids_kept: true,
            ..Default::default()
        }),
        Some(tip.clone()),
    ] {
        let base = base.map(|run| ("check".into(), run)).into_iter().collect();
        let findings = compare(
            &["check".into()],
            &base,
            &BTreeMap::from([("check".into(), tip.clone())]),
        );
        assert!(
            findings.iter().any(RegressionFinding::blocks),
            "{findings:?}"
        );
    }
}

#[test]
fn a_cached_named_failure_preserves_numeric_locations() {
    use crate::v2::write::test_baseline_run_base::{cache, cached};
    let root = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(root.path().join("run/v2"));
    let mut base = HostRunVerdict {
        command: "check".into(),
        commit: "base".into(),
        verbatim: true,
        failing_tests: vec!["case".into()],
        failed_count: Some(1),
        ids_kept: true,
        exit_code: Some(1),
        signatures: [("case".into(), "src/a.rs:12:4 | failure".into())].into(),
        ..Default::default()
    };
    cache(&store, Tree::RunBase, &base);
    base = cached(&store, Tree::RunBase, "base", "check").unwrap();
    let mut tip = base.clone();
    tip.signatures
        .insert("case".into(), "src/a.rs:13:4 | failure".into());
    let findings = compare(
        &["check".into()],
        &[("check".into(), base)].into(),
        &[("check".into(), tip)].into(),
    );
    assert!(matches!(
        findings.as_slice(),
        [RegressionFinding::NewFailure { .. }]
    ));
    assert!(
        findings.iter().all(RegressionFinding::blocks),
        "{findings:?}"
    );
}

#[test]
fn legacy_masked_cache_is_not_reused_as_digit_preserving_evidence() {
    use crate::v2::write::test_baseline_run_base::{cache, cached};
    let root = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(root.path().join("run/v2"));
    let verdict = HostRunVerdict {
        command: "check".into(),
        commit: "base".into(),
        verbatim: true,
        signatures: [(String::new(), "error: case_<value>".into())].into(),
        ..Default::default()
    };
    cache(&store, Tree::RunBase, &verdict);
    assert_eq!(
        cached(&store, Tree::RunBase, "base", "check"),
        Some(verdict)
    );
    let file = std::fs::read_dir(store.root().join("baseline-tests/run-base"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("identity_digits_preserved");
    std::fs::write(file, serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert!(cached(&store, Tree::RunBase, "base", "check").is_none());
}

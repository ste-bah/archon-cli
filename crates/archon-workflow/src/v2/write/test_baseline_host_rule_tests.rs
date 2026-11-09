use super::*;
#[cfg(all(test, unix))]
#[tokio::test]
async fn frozen_shell_text_runs_verbatim_on_a_real_base_tree() {
    let _clock = tests::freeze_runner_clock();
    let temp = tempfile::tempdir().unwrap();
    let (root, base, host, _) = tests::world(temp.path());
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let summary = "printf 'running 1 tests\\ntest result: ok. 1 passed; 0 failed\\n'";
    for command in [
        summary.to_string(),
        format!("test \"$(cat src/old_red)\" = '// x' && {summary}"),
        format!("MODE=value; test \"$MODE\" = value && {summary}"),
    ] {
        let frozen = FrozenCommands::from_commands(std::slice::from_ref(&command));
        let runs = host_verdicts(
            &store,
            &host,
            &root,
            Tree::RunBase,
            &base,
            std::slice::from_ref(&command),
            &frozen,
        )
        .await;
        assert_eq!(runs[&command].exit_code, Some(0));
        assert_eq!(runs[&command].command, command);
        assert!(runs[&command].verbatim);
    }
}
#[test]
fn old_host_verdict_json_round_trips_with_its_existing_fields() {
    let old = serde_json::json!({"command": "check --one", "commit": "abc", "exit_code": 1, "failing_tests": [], "signatures": {}, "failure_files": {}, "failed_count": null, "ids_kept": false});
    let verdict: HostRunVerdict = serde_json::from_value(old.clone()).unwrap();
    assert!(!verdict.verbatim);
    assert!(!verdict.per_test_verdict);
    assert_eq!(serde_json::to_value(verdict).unwrap(), old);
}

#[cfg(all(test, unix))]
#[tokio::test]
async fn selection_and_verbosity_arguments_are_never_rewritten() {
    let _clock = tests::freeze_runner_clock();
    let temp = tempfile::tempdir().unwrap();
    let (root, base, host, counter) = tests::world(temp.path());
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    for text in [
        "cargo test -p app --lib",
        "cargo test -q -p app -v --lib",
        "cargo test -p app -- --exact x",
        "cargo nextest run --no-fail-fast",
    ] {
        let command = text.to_string();
        let frozen = FrozenCommands::from_commands(std::slice::from_ref(&command));
        let runs = host_verdicts(
            &store,
            &host,
            &root,
            Tree::RunBase,
            &base,
            std::slice::from_ref(&command),
            &frozen,
        )
        .await;
        assert_eq!(runs[&command].command, command);
        assert_eq!(runs[&command].exit_code, Some(101));
        assert_eq!(
            std::fs::read_to_string(&counter).unwrap().lines().last(),
            Some(format!("run {}", text.strip_prefix("cargo ").unwrap()).as_str())
        );
    }
}

#[test]
fn cached_results_distinguish_legacy_generic_and_per_test_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    for (verbatim, per_test) in [(false, false), (true, false), (true, true)] {
        let verdict = HostRunVerdict {
            command: "check --one".into(),
            commit: "abc".into(),
            verbatim,
            per_test_verdict: per_test,
            ..Default::default()
        };
        cache(&store, Tree::RunBase, &verdict);
        assert_eq!(
            cached(&store, Tree::RunBase, "abc", "check --one").is_some(),
            verbatim
        );
        assert_eq!(
            cached_per_test(
                &store,
                Tree::RunBase,
                "abc",
                "check --one",
                &FrozenCommands::from_commands(&["check --one".into()]),
            )
            .is_some(),
            verbatim && per_test
        );
    }
}

#[test]
fn panic_paths_use_one_repository_spelling_on_every_host() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("cases")).unwrap();
    std::fs::write(temp.path().join("cases/check.rs"), "content").unwrap();
    for path in [
        "cases/check.rs".to_string(),
        r"cases\check.rs".to_string(),
        temp.path()
            .join("cases/check.rs")
            .to_string_lossy()
            .to_string(),
    ] {
        let output = format!("thread 'case::fails' panicked at {path}:2:3:\nboom\n");
        assert_eq!(
            panic_files(&output, "case::fails", temp.path()),
            ["cases/check.rs"]
        );
    }
}

#[test]
fn only_repository_roots_are_normalized_independently_of_host_os() {
    for (left, left_root, right, right_root) in [
        ("/one/a.rs", "/one", "/two/a.rs", "/two"),
        (r"C:\one\a.rs", r"C:\one", r"D:\two\a.rs", r"D:\two"),
        (
            r"\\one\share\a.rs",
            r"\\one\share",
            r"\\two\share\a.rs",
            r"\\two\share",
        ),
    ] {
        let output = |path: &str| format!("thread 'case::fails' panicked at {path}:2:3:\nboom\n");
        let left_identity = signature_at(&output(left), "case::fails", Path::new(left_root));
        assert_eq!(
            left_identity,
            signature_at(&output(right), "case::fails", Path::new(right_root))
        );
        assert_ne!(
            signature(&output(left), "case::fails"),
            signature(&output(right), "case::fails")
        );
        assert_ne!(
            left_identity,
            signature_at(
                &output(&left.replace("a.rs", "b.rs")),
                "case::fails",
                Path::new(left_root)
            )
        );
    }
}

#[cfg(all(test, unix))]
#[tokio::test]
async fn whitespace_aliases_run_the_literal_frozen_declaration() {
    let _clock = tests::freeze_runner_clock();
    let temp = tempfile::tempdir().unwrap();
    let (root, base, host, _) = tests::world(temp.path());
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    let summary = "printf 'running 1 tests\\ntest result: ok. 1 passed; 0 failed\\n'";
    for command in [
        summary.to_string(),
        format!("MODE=value; {summary}"),
        format!("test -f src/old_red && {summary}"),
    ] {
        let frozen = FrozenCommands::from_commands(std::slice::from_ref(&command));
        let reported = command.replace(' ', "  ");
        let runs = host_verdicts(
            &store,
            &host,
            &root,
            Tree::RunBase,
            &base,
            std::slice::from_ref(&reported),
            &frozen,
        )
        .await;
        assert_eq!(runs[&reported].command, command);
        assert_eq!(runs[&reported].exit_code, Some(0));
        assert_eq!(
            cached_per_test(&store, Tree::RunBase, &base, &reported, &frozen)
                .unwrap()
                .command,
            command
        );
    }
}

#[test]
fn judged_cache_requires_an_isolated_execution_attestation() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("run/v2"));
    for isolated in [false, true] {
        let verdict = HostRunVerdict {
            command: "check".into(),
            commit: "abc".into(),
            verbatim: true,
            per_test_verdict: true,
            isolated,
            ..Default::default()
        };
        cache(&store, Tree::Judged, &verdict);
        assert_eq!(
            cached(&store, Tree::Judged, "abc", "check").is_some(),
            isolated
        );
    }
}

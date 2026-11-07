use super::*;

fn selected(filter: &str, exact: bool) -> Invocation {
    invocations(&format!(
        "cargo test -p package --lib {filter}{}",
        if exact { " -- --exact" } else { "" }
    ))
    .remove(0)
}
fn names() -> Vec<String> {
    [
        "module::tests::alpha",
        "module::tests::beta",
        "module::tests::gamma",
        "a_completely_unrelated_module::different_name",
    ]
    .map(str::to_string)
    .into()
}

#[test]
fn single_dead_filter_preserves_the_exact_finding() {
    let invocation = invocations("cargo test -p package --test target -- missing").remove(0);
    let names = inventory(
        &Runner::Cargo,
        "module::test_alpha: test\n1 test, 0 benchmarks",
    )
    .unwrap();
    assert_eq!(
        mismatch_for_run(Some(0), &invocation, Some(&names)).unwrap(),
        format!(
            "{} ran 0 matching tests: filter `missing` matched no test in its selected base-tree target; if the work adds this test, it does not exist yet at base and the check must fail there when no test runs; otherwise check the full test path against the nearest real test paths: module::test_alpha",
            invocation.line
        )
    );
}

#[test]
fn multi_filter_dead_filter_counts_the_union_of_sibling_matches() {
    for (command, listing) in [
        (
            "cargo test -p package --test target -- missing alpha tests::alpha beta",
            "module::tests::alpha: test\nmodule::tests::beta: test\nmodule::tests::gamma: test",
        ),
        (
            "cargo nextest run -p package --test target -- missing alpha tests::alpha beta",
            r#"{"rust-suites":{"target":{"testcases":{"module::tests::alpha":{},"module::tests::beta":{},"module::tests::gamma":{}}}}}"#,
        ),
    ] {
        let invocations = invocations(command);
        assert_eq!(invocations.len(), 4);
        let names = inventory(&invocations[0].runner, listing).unwrap();
        let findings: Vec<_> = invocations
            .iter()
            .filter_map(|invocation| mismatch_for_run(Some(0), invocation, Some(&names)))
            .collect();
        assert_eq!(findings.len(), 1);
        let finding = &findings[0];
        assert!(finding.contains(&format!(
            "filter `missing` in {} matches no test",
            invocations[0].line
        )));
        assert!(
            finding.contains("other named filters (`alpha`, `tests::alpha`, `beta`) match 2 tests")
        );
        assert!(finding.contains(
            "every named test must exist or the check must fail at base when it does not run"
        ));
        assert!(finding.contains("nearest real test paths: module::tests::"));
        assert!(!finding.contains("ran 0 matching tests"));
    }
}

#[test]
fn multi_filter_all_dead_names_every_filter_and_reports_zero_tests() {
    let invocations = invocations("cargo test -p package --test target -- missing absent");
    assert_eq!(invocations.len(), 2);
    let names = inventory(&Runner::Cargo, "module::test_alpha: test").unwrap();
    for invocation in &invocations {
        let finding = mismatch_for_run(Some(0), invocation, Some(&names)).unwrap();
        assert!(finding.contains(&format!(
            "{} ran 0 matching tests: filters `missing`, `absent` matched no test",
            invocation.line
        )));
        assert!(finding.contains("nearest real test paths: module::test_alpha"));
    }
}

#[test]
fn multi_filter_exact_matching_does_not_count_substring_matches() {
    let invocations = invocations(
        "cargo test -p package --test target -- tests::alpha module::tests::beta --exact",
    );
    assert_eq!(invocations.len(), 2);
    let names = inventory(
        &Runner::Cargo,
        "module::tests::alpha: test\nmodule::tests::beta: test\nmodule::tests::beta_extra: test",
    )
    .unwrap();
    let finding = mismatch_for_run(Some(0), &invocations[0], Some(&names)).unwrap();
    assert!(finding.contains("filter `tests::alpha` in "));
    assert!(finding.contains("other named filters (`module::tests::beta`) match 1 tests"));
    assert!(!finding.contains("ran 0 matching tests"));
    assert!(mismatch_for_run(Some(0), &invocations[1], Some(&names)).is_none());
}

#[test]
fn sibling_filters_stay_with_their_command() {
    let invocations =
        invocations("cargo test --test first -- missing absent; cargo test --test second -- alpha");
    assert_eq!(invocations.len(), 3);
    let names = inventory(&Runner::Cargo, "alpha: test").unwrap();
    let finding = mismatch_for_run(Some(0), &invocations[0], Some(&names)).unwrap();
    assert!(finding.contains("ran 0 matching tests: filters `missing`, `absent`"));
    assert!(mismatch_for_run(Some(0), &invocations[2], Some(&names)).is_none());
}

#[test]
fn pytest_dead_filter_and_node_matching_are_unchanged() {
    let names = inventory(
        &Runner::Pytest,
        "t.py::test_alpha[1]\nt.py::TestC::test_beta",
    )
    .unwrap();
    for command in [
        "pytest -k ALPHA",
        "pytest t.py::TestC",
        "pytest t.py::test_alpha",
    ] {
        let invocation = invocations(command).remove(0);
        assert!(mismatch_for_run(Some(0), &invocation, Some(&names)).is_none());
    }
    let invocation = invocations("pytest -k missing").remove(0);
    let finding = mismatch_for_run(Some(0), &invocation, Some(&names)).unwrap();
    assert!(finding.starts_with(&format!(
        "{} ran 0 matching tests: filter `missing` matched no test",
        invocation.line
    )));
    assert!(finding.contains("does not exist yet at base and the check must fail there"));
}

#[test]
#[allow(non_snake_case)]
fn P3_3_all_zero_names_each_invocation_and_nearest_paths() {
    assert!(archon_workflow::acceptance::output_reports_zero_work(
        "running 0 tests\nrunning 0 tests",
        ""
    ));
    let invocations = invocations(
        "cargo test -p package --lib tests::alpha -- --exact; cargo test -p package --lib tests::beta -- --exact",
    );
    let findings: Vec<_> = invocations
        .iter()
        .filter_map(|invocation| mismatch_for_run(Some(0), invocation, Some(&names())))
        .collect();
    assert_eq!(findings.len(), 2);
    for (index, finding) in findings.iter().enumerate() {
        assert!(finding.contains(&invocations[index].line));
        assert!(finding.contains("nearest real test paths: module::tests::"));
    }
}
#[test]
#[allow(non_snake_case)]
fn P3_4_synthetic_successful_partial_zero_work_rejects_only_missing_exact_filter() {
    // The recorded probes exited 1/101; this successful partial-zero case is synthetic.
    let output = "running 1 test\ntest module::tests::alpha ... ok\nrunning 0 tests\ntest result: ok. 0 passed; 0 failed\n";
    assert!(!archon_workflow::acceptance::output_reports_zero_work(
        output, ""
    ));
    assert!(
        mismatch_for_run(
            Some(0),
            &selected("module::tests::alpha", true),
            Some(&names())
        )
        .is_none()
    );
    let missing =
        mismatch_for_run(Some(0), &selected("tests::beta", true), Some(&names())).unwrap();
    assert!(
        missing.contains("module::tests::beta, module::tests::alpha, module::tests::gamma"),
        "{missing}"
    );
    assert!(!missing.contains("a_completely_unrelated_module::different_name"));
}
#[test]
#[allow(non_snake_case)]
fn P3_5_present_exact_filter_is_not_a_finding() {
    assert!(
        mismatch_for_run(
            Some(0),
            &selected("module::tests::alpha", true),
            Some(&names())
        )
        .is_none()
    );
}
#[test]
#[allow(non_snake_case)]
fn P3_6_matching_substring_is_not_a_finding() {
    assert!(mismatch_for_run(Some(0), &selected("tests::alpha", false), Some(&names())).is_none());
}
#[test]
#[allow(non_snake_case)]
fn P3_7_unmatched_substring_names_nearest_paths() {
    assert!(
        mismatch_for_run(Some(0), &selected("tests::alpa", false), Some(&names()))
            .unwrap()
            .contains("module::tests::alpha")
    );
}
#[test]
#[allow(non_snake_case)]
fn P3_8_unknown_runner_or_failed_inventory_keeps_fallback() {
    assert!(invocations("unknown-runner exact-filter").is_empty());
    assert!(invocations("pytest -k 'alpha or beta'").is_empty());
    assert!(inventory(&Runner::Cargo, "build failed").is_none());
    assert!(mismatch_for_run(Some(0), &selected("absent", true), None).is_none());
    assert!(unavailable_note("check", "runner").contains("inventory unavailable"));
}
#[test]
#[allow(non_snake_case)]
fn P3_9_nonzero_base_run_for_a_new_test_is_not_zero_work() {
    let finding = mismatch_for_run(Some(0), &selected("new_test", true), Some(&names())).unwrap();
    assert!(finding.contains("does not exist yet at base") && finding.contains("must fail there"));
    assert!(!finding.contains("point at an existing test"));
    for code in [1, 101] {
        assert!(
            mismatch_for_run(Some(code), &selected("new_test", true), Some(&names())).is_none()
        );
    }
}

#[tokio::test]
#[allow(non_snake_case)]
async fn R4_inventory_findings_uses_base_cache_for_successful_partial_zero_work() {
    use super::super::{Baseline, HostProbe, hermetic::CheckEvidence};
    let root = tempfile::tempdir().unwrap();
    let probe = HostProbe::at(root.path().into(), root.path().into(), None);
    let base = Baseline {
        origin: crate::command::workflow_task_set::executability::BaselineOrigin::RecordedBase,
        repository: root.path().into(),
        commit: "base".into(),
    };
    let command = "cargo test -p package -- missing module::tests::alpha --exact";
    let contract = serde_json::from_value(serde_json::json!({
        "schema_version":1,"prd":{"path":"requirements.md","digest":"d"},
        "gap_policy":{"permitted_acceptance_ids":[],"forbidden_phrases":[],"required_fields":[]},
        "acceptance":[{"id":"check","criterion":"new behavior","check":{"kind":"command","command":command,"cwd":"repo_root"},"judgment":{"verdict":"accepted","counterexample":"","reason":"","host_call_id":""}}],"supplementary":[]
    })).unwrap();
    let invocation = invocations(command).remove(0);
    let key = format!(
        "base:{:?}:{}",
        Some(archon_workflow::task_set_contract::TrustedCwd::RepoRoot),
        invocation.list
    );
    probe
        .test_inventories
        .lock()
        .unwrap()
        .insert(key, Some(names()));
    let result = archon_workflow::acceptance_scratch::CheckResult {
        acceptance_id: "check".into(),
        exit_code: Some(0),
        stdout: b"running 1 test\nrunning 0 tests".to_vec(),
        stderr: Vec::new(),
        verdict_streams: Default::default(),
        fault: None,
        quota_walk_count: 0,
        operational_error: None,
        classification: None,
        environment_note: None,
    };
    let findings = findings(
        &probe,
        &base,
        &contract,
        &[CheckEvidence {
            result,
            copy_layout: None,
        }],
    )
    .await;
    assert!(findings["check"].contains("module::tests::alpha"));
    assert!(findings["check"].contains("match 1 tests"));
    assert!(findings["check"].contains("must fail at base"));
    assert!(!findings["check"].contains("zero work"));
    assert!(!findings["check"].contains("ran 0 matching tests"));
}

#[test]
fn inventory_runner_flags_and_pytest_node_prefixes_do_not_become_false_filters() {
    let invocation = invocations(
        "cargo test alpha -- --exact -Z unstable-options --shuffle-seed 42 --color never",
    );
    assert_eq!(invocation.len(), 1);
    assert_eq!(invocation[0].filter, "alpha");
    for command in [
        "pytest t.py::test_a",
        "pytest t.py::TestC",
        "pytest -k TEST_A",
    ] {
        let invocation = invocations(command).remove(0);
        assert!(
            mismatch(
                &invocation,
                &["t.py::test_a[1]".into(), "t.py::TestC::test_m".into()]
            )
            .is_none()
        );
    }
    assert!(cargo_multi_names("cargo test -F feature --target-dir cache only_one").is_empty());
}

#[test]
fn test_inventory_located_commands_skip_heredoc_data_and_directory_changes() {
    for reject_guards in [false, true] {
        for text in [
            "cat <<'END'\ncd elsewhere; cargo test -p data || true\nEND\ncargo test -p actual",
            "cat <<-\"END\"\n\tcd elsewhere; cargo test -p data\n\tEND\ncargo test -p actual",
            "cat <<A <<'B'\ncd elsewhere\nA\ncargo test -p data\nB\ncargo test -p actual",
        ] {
            let found = located_commands_with_guards(text, reject_guards);
            let cargo: Vec<_> = found.iter().filter(|(c, _)| c.word == "cargo").collect();
            assert_eq!(cargo.len(), 1, "{text}");
            assert_eq!(cargo[0].0.args, ["test", "-p", "actual"]);
            assert_eq!(cargo[0].1, Some(Vec::new()));
        }
    }
}

use super::*;

#[test]
fn reappearing_input_does_not_preempt_the_mutated_assertion_verdict() {
    let inputs = vec![
        "data/asserted.txt".to_string(),
        "data/reappeared.txt".to_string(),
        "data/third.txt".to_string(),
    ];
    let (evidence, markers, names) = run_real_wrapper_with_inputs(
        "printf recreated > data/reappeared.txt; test -f data/asserted.txt && test -f data/third.txt",
        inputs,
    );
    assert_eq!(markers.moved(&evidence.result, &names).len(), 3);
    assert_eq!(evidence.result.exit_code, Some(1));
    assert!(
        markers
            .recreated(&evidence.result, &names)
            .contains("data/reappeared.txt")
    );
    assert!(
        mutation_host_or_author_finding(
            &evidence,
            &markers,
            &names,
            None,
            "check 'AC-1': it passed",
        )
        .is_none()
    );
}

#[test]
fn repository_alias_diagnostic_keeps_the_named_path_and_site_copy_note_when_moved() {
    let path = "../repository/tests/focused.rs";
    let diagnostics = describe_candidates_with_run_state(
        &[super::mutation::CandidateDiagnostic {
            path: path.into(),
            move_path: Some("/site/repository/tests/focused.rs".into()),
            move_root: Some("/site/repository".into()),
            exclusion: Some(super::mutation::ExclusionReason::RepositoryCopyAlias),
        }],
        &std::collections::BTreeSet::new(),
        &[],
        &std::collections::BTreeSet::from([path.to_string()]),
        &std::collections::BTreeSet::new(),
        &std::collections::BTreeMap::new(),
        &std::collections::BTreeMap::new(),
        true,
    );
    assert!(diagnostics.contains(path));
    assert!(diagnostics.contains("moved in the site copy"));
}

#[test]
fn recreated_input_with_operational_failure_is_host_unproven() {
    let markers = super::mutation::Markers::new();
    let inputs = vec!["data/state.txt".to_string(), "data/other.txt".to_string()];
    let result = CheckResult { verdict_streams: Default::default(),
        fault: None,
        classification: None,
        acceptance_id: "AC-1".into(),
        exit_code: None,
        quota_walk_count: 0,
        stdout: format!(
            "archon-mutation-state-{}: 0:original-recreated\narchon-mutation-state-{}: 1:restored\narchon-mutation-status-{}: nonce:1:0:none\n",
            markers.nonce(), markers.nonce(), markers.nonce()
        )
        .into_bytes(),
        stderr: format!("{}0\n", markers.moved_prefix()).into_bytes(),
        operational_error: Some("the host could not complete the run".into()),
        environment_note: None,
    };
    let evidence = CheckEvidence {
        result,
        copy_layout: None,
    };

    let finding = mutation_host_or_author_finding(
        &evidence,
        &markers,
        &inputs,
        Some("it names `../project/Cargo.toml`, which is absent from the probe copy".into()),
        "check 'AC-1': it passed",
    )
    .expect("operational failure remains host-unproven even with a setup-path finding");
    let MutationFinding::HostUnproven(host) = finding else {
        panic!("host failure with a recreated input must remain host-unproven");
    };
    assert!(host.contains("data/state.txt=original-recreated"));
    assert!(host.contains("data/other.txt=restored"));
    assert!(host.contains("the host could not complete the run"));
}

/// Issue 495 (replaces `a_setup_path_failure_after_environment_repair_is_an_author_finding`):
/// the finding names an environment repair only when the host made one.
#[test]
fn a_setup_path_failure_is_an_author_finding_naming_only_real_repairs() {
    let (result, markers, inputs) = run_real_wrapper("test -f data/state.txt");
    let reason = "it names ../archon-cli/Cargo.toml, absent from the probe copy";
    let repaired = super::setup_reason_with_repairs(
        reason.into(),
        &["re-ran it on a freshly built copy".to_string()],
    );
    let finding = mutation_host_or_author_finding(
        &result,
        &markers,
        &inputs,
        Some(repaired),
        "check 'AC-1': it passed",
    )
    .expect("identified setup failure goes to the check author");
    let MutationFinding::AuthorFinding(author) = finding else {
        panic!("setup path failure must not become a host error");
    };
    assert!(author.contains("../archon-cli/Cargo.toml"), "{author}");
    assert!(
        author.contains("the host had repaired its environment before this run: re-ran it on a freshly built copy"),
        "{author}"
    );
    let finding = mutation_host_or_author_finding(
        &result,
        &markers,
        &inputs,
        Some(super::setup_reason_with_repairs(reason.into(), &[])),
        "check 'AC-1': it passed",
    )
    .expect("identified setup failure goes to the check author");
    let MutationFinding::AuthorFinding(author) = finding else {
        panic!("setup path failure must not become a host error");
    };
    assert!(author.contains("../archon-cli/Cargo.toml"), "{author}");
    assert!(!author.contains("repaired its environment"), "{author}");
}

/// Issue 495: the setup-failure finding carries the output it cites. The
/// check runs for real in the mutation wrapper with its input moved aside.
#[test]
fn mutation_setup_finding_carries_the_output_it_cites() {
    let (result, markers, inputs) =
        run_real_wrapper("test -f data/state.txt || cd ../archon-absent-root/inner");
    let reason = "it names the path `../archon-absent-root/inner` (cd), which the site does not create because it lies outside the tree it ran in; its output reports the path absent: a setup failure of its site, not a failure of its criterion";
    let finding = mutation_host_or_author_finding(
        &result,
        &markers,
        &inputs,
        Some(super::setup_reason_with_repairs(reason.into(), &[])),
        "check 'AC-1': it passed",
    )
    .expect("identified setup failure goes to the check author");
    let MutationFinding::AuthorFinding(author) = finding else {
        panic!("setup path failure must not become a host error");
    };
    let finding = super::observed(
        author,
        &super::mutation_observation(&result.result, &markers, &inputs, &host_redactor()),
    );
    assert!(
        finding
            .contains("with the data it names moved aside, the check stopped at a setup failure"),
        "{finding}"
    );
    assert!(!finding.contains("repaired its environment"), "{finding}");
    let observed = finding
        .split_once("; mutation observation: ")
        .expect("observation")
        .1;
    assert!(
        observed.contains("../archon-absent-root/inner"),
        "the output the reason cites is kept: {observed}"
    );
    assert!(
        !observed.contains("mutated_exit_code=Some(0)"),
        "{observed}"
    );
    assert!(observed.contains("data/state.txt"), "{observed}");
    assert!(!observed.contains("archon-mutation-"), "{observed}");
}

fn run_real_wrapper(command: &str) -> (CheckEvidence, super::mutation::Markers, Vec<String>) {
    run_real_wrapper_with_inputs(command, vec!["data/state.txt".to_string()])
}

fn run_real_wrapper_with_inputs(
    command: &str,
    inputs: Vec<String>,
) -> (CheckEvidence, super::mutation::Markers, Vec<String>) {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("data")).unwrap();
    for input in &inputs {
        let path = project.path().join(input);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "ready").unwrap();
    }
    let markers = super::mutation::Markers::new();
    let command = command.replace("@TAG@", &markers.tag());
    let wrapped = super::mutation::wrap(&command, &inputs, &[], &markers);
    let output = super::super::mutation::run_with_host_mutation(project.path(), &wrapped);
    let evidence = CheckEvidence {
        result: CheckResult {
            verdict_streams: Default::default(),
            fault: None,
            classification: None,
            acceptance_id: "AC-1".into(),
            exit_code: output.status.code(),
            quota_walk_count: 0,
            stdout: output.stdout,
            stderr: output.stderr,
            operational_error: None,
            environment_note: None,
        },
        copy_layout: None,
    };
    (evidence, markers, inputs)
}

#[test]
fn real_wrapper_recreation_is_attributed_to_the_mutated_check() {
    let (result, markers, inputs) = run_real_wrapper("printf recreated > data/state.txt; exit 0");
    let recreated = markers.recreated(&result.result, &inputs);
    assert!(recreated.contains("data/state.txt"));
    assert!(
        author_recreated_finding(
            "check 'AC-1': it passed",
            &recreated.iter().cloned().collect::<Vec<_>>(),
        )
        .contains("it recreated data/state.txt while running")
    );
    assert!(
        mutation_host_or_author_finding(
            &result,
            &markers,
            &inputs,
            None,
            "check 'AC-1': it passed"
        )
        .is_none()
    );
    assert!(!markers.restored_all(&result.result));
}

#[test]
fn a_transient_path_is_not_attributed_when_absent_after_the_check_exits() {
    let (result, markers, inputs) = run_real_wrapper_with_inputs(
        "printf recreated > data/state.txt; rm data/state.txt; exit 0",
        vec![
            "data/state.txt".to_string(),
            "other/untouched.txt".to_string(),
        ],
    );

    assert!(markers.recreated(&result.result, &inputs).is_empty());
    assert!(
        mutation_host_or_author_finding(
            &result,
            &markers,
            &inputs,
            None,
            "check 'AC-1': it passed"
        )
        .is_none()
    );
}

#[test]
fn real_wrapper_restoration_keeps_normal_verdict_route() {
    let (result, markers, inputs) = run_real_wrapper("test -f data/state.txt");

    assert!(markers.restored_all(&result.result));
    assert!(
        mutation_host_or_author_finding(
            &result,
            &markers,
            &inputs,
            None,
            "check 'AC-1': it passed",
        )
        .is_none()
    );
}

#[test]
fn real_wrapper_observed_signal_is_host_unproven() {
    let (result, markers, inputs) = run_real_wrapper("kill -TERM \"$$\"; exit 0");

    let finding = mutation_host_or_author_finding(
        &result,
        &markers,
        &inputs,
        None,
        "check 'AC-1': it passed",
    )
    .expect("observed signal must remain a host failure");
    let MutationFinding::HostUnproven(host) = finding else {
        panic!("a host failure must remain host-unproven");
    };
    assert!(host.contains("wrapper observed signal TERM"));
}

#[test]
fn real_wrapper_missing_aside_is_host_unproven_with_input_state() {
    let (result, markers, inputs) = run_real_wrapper("rm -f data/state.txt.@TAG@; exit 0");

    let finding = mutation_host_or_author_finding(
        &result,
        &markers,
        &inputs,
        None,
        "check 'AC-1': it passed",
    )
    .expect("missing aside must remain host-unproven");
    let MutationFinding::HostUnproven(host) = finding else {
        panic!("a host failure must remain host-unproven");
    };
    assert!(host.contains("data/state.txt=aside-missing"));
}

#[test]
fn incomplete_restore_explains_every_path_and_exit_signal() {
    let states = std::collections::BTreeMap::from([
        (
            "data/state.txt".to_string(),
            "aside-missing: no restoration state was recorded".to_string(),
        ),
        ("data/tree".to_string(), "restored".to_string()),
    ]);
    let reason = format!(
        "{}; check status/signal: {}",
        describe_states(&states),
        describe_status(Some("nonce:143:15:TERM".into()))
    );
    assert!(reason.contains("data/state.txt=aside-missing: no restoration state was recorded"));
    assert!(reason.contains("data/tree=restored"));
    assert!(reason.contains("exit status 143, wrapper-observed signal TERM"));
}

#[test]
fn timeout_diagnostics_keep_known_states_and_report_unavailable_inputs() {
    let markers = super::mutation::Markers::new();
    let inputs = vec!["data/state.txt".to_string(), "data/other.txt".to_string()];
    let result = CheckResult {
        verdict_streams: Default::default(),
        fault: None,
        classification: None,
        acceptance_id: "AC-1".into(),
        exit_code: None,
        quota_walk_count: 0,
        stdout: format!(
            "archon-mutation-status-{}: nonce:143:15:TERM\n",
            markers.nonce()
        )
        .into_bytes(),
        stderr: format!("{}0\n", markers.moved_prefix()).into_bytes(),
        operational_error: Some(archon_workflow::acceptance_scratch::CHECK_TIMED_OUT.into()),
        environment_note: None,
    };

    let states = input_states(&result, &markers, &inputs);
    let report = format!(
        "{}; status/signal: {}",
        describe_states(&states),
        describe_status(markers.status(&result))
    );
    assert!(report.contains("data/state.txt=aside-missing: no restoration state was recorded"));
    assert!(report.contains("data/other.txt=unreported"));
    assert!(report.contains("exit status 143, wrapper-observed signal TERM"));
}

#[test]
fn operational_error_mutation_diagnostics_keep_input_states_and_exit_signal() {
    let markers = super::mutation::Markers::new();
    let inputs = vec!["data/state.txt".to_string(), "data/other.txt".to_string()];
    let result = CheckResult { verdict_streams: Default::default(),
        fault: None,
        classification: None,
        acceptance_id: "AC-1".into(),
        exit_code: None,
        quota_walk_count: 0,
        stdout: format!(
            "archon-mutation-state-{}: 0:aside-missing\narchon-mutation-status-{}: nonce:143:15:TERM\n",
            markers.nonce(),
            markers.nonce()
        )
        .into_bytes(),
        stderr: format!("{}0\n", markers.moved_prefix()).into_bytes(),
        operational_error: Some("the host could not complete the run".into()),
        environment_note: None,
    };

    let report = operational_error_diagnostics(&result, &markers, &inputs);
    assert!(report.contains("data/state.txt=aside-missing"));
    assert!(report.contains("data/other.txt=unreported"));
    assert!(report.contains("exit status 143, wrapper-observed signal TERM"));
}

#[test]
fn recreated_input_is_not_marked_restored_for_cache_reuse() {
    let markers = super::mutation::Markers::new();
    let inputs = vec!["data/state.txt".to_string()];
    let result = CheckResult {
        verdict_streams: Default::default(),
        fault: None,
        classification: None,
        acceptance_id: "AC-1".into(),
        exit_code: Some(1),
        quota_walk_count: 0,
        stdout: format!(
            "archon-mutation-state-{}: 0:original-recreated\narchon-mutation-restored-{}\n",
            markers.nonce(),
            markers.nonce()
        )
        .into_bytes(),
        stderr: format!("{}0\n", markers.moved_prefix()).into_bytes(),
        operational_error: None,
        environment_note: None,
    };

    assert!(!all_moved_inputs_restored(&result, &markers, &inputs));
}

#[test]
fn parent_metadata_recreation_does_not_enter_author_recreated_route() {
    let states =
        std::collections::BTreeMap::from([("data/state.txt".to_string(), "restored".to_string())]);
    assert!(recreated_paths(&states).is_empty());
}

#[test]
fn unrelated_sibling_touch_keeps_the_parent_metadata_signal_on_legacy_path() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("data")).unwrap();
    std::fs::write(project.path().join("data/state.txt"), "ready").unwrap();
    let markers = super::mutation::Markers::new();
    let wrapped = super::mutation::wrap(
        "touch data/unrelated.txt; test -f data/state.txt",
        &["data/state.txt".into()],
        &[],
        &markers,
    );
    let output = super::super::mutation::run_with_host_mutation(project.path(), &wrapped);
    let result = CheckResult {
        verdict_streams: Default::default(),
        fault: None,
        classification: None,
        acceptance_id: "AC-1".into(),
        exit_code: output.status.code(),
        quota_walk_count: 0,
        stdout: output.stdout,
        stderr: output.stderr,
        operational_error: None,
        environment_note: None,
    };
    let names = vec!["data/state.txt".to_string()];
    let states = markers.states(&result, &names);

    assert!(markers.recreated(&result, &names).is_empty());
    assert_eq!(
        states.get("data/state.txt").map(String::as_str),
        Some("original-recreated")
    );
    assert_eq!(recreated_paths(&states), vec!["data/state.txt"]);
    let finding = super::unattributed_recreation_finding("the check passed on the baseline");
    assert!(
        finding.contains("parent directory metadata changed"),
        "{finding}"
    );
    assert!(
        finding.contains("does not identify who wrote the input"),
        "{finding}"
    );
}

fn host_redactor() -> crate::command::workflow_task_set::passability::evidence::Redactor {
    crate::command::workflow_task_set::passability::evidence::Redactor::for_environment(
        std::iter::empty(),
        &[],
    )
}

#[path = "workflow_acceptance_executability_prove_alias_tests_diagnostic_cases.rs"]
mod diagnostic_cases;

use super::*;

#[test]
fn recreated_inputs_route_to_author_action_with_the_named_path() {
    let states = std::collections::BTreeMap::from([
        (
            "data/state.txt".to_string(),
            "original-recreated".to_string(),
        ),
        ("data/other.txt".to_string(), "restored".to_string()),
    ]);
    let paths = recreated_paths(&states);
    let finding = author_recreated_finding("check 'AC-1': it passed", &paths);
    assert!(finding.contains("it recreated data/state.txt while running"));
    assert!(
        finding.contains("use a movable input that exists under the selected root before the run")
    );
}

#[test]
fn baseline_finding_lists_candidates_and_gives_the_root_repair_route() {
    let finding = baseline_pass_finding("check-id", "0123456789ab");
    let candidate = super::mutation::CandidateDiagnostic {
        path: "tests/focused.rs".into(),
        exclusion: Some(super::mutation::ExclusionReason::MissingAtRoot),
    };
    let finding = format!(
        "{finding}; considered inputs: {}",
        describe_candidates(&[candidate])
    );
    assert!(finding.contains("missing at the selected root: tests/focused.rs"));
    assert!(finding.contains("data the check writes itself is never evidence"));
    assert!(
        finding
            .contains("movable inputs must exist under the check's selected root before the run")
    );
    assert!(
        finding.contains(
            "select the root that contains the existing implementation or test dependency"
        )
    );
    assert!(
        finding.contains(
            "the check must still fail on its own assertion when that input is moved aside"
        )
    );
    let recreated = ["output/state.txt".to_string()];
    let diagnostics = describe_candidates_with_run_state(
        &[super::mutation::CandidateDiagnostic {
            path: recreated[0].clone(),
            exclusion: None,
        }],
        &std::collections::BTreeSet::new(),
        &recreated,
        &std::collections::BTreeSet::new(),
        &std::collections::BTreeSet::from(recreated.clone()),
        true,
    );
    assert!(
        diagnostics
            .contains("the check recreated this path while running (observed): output/state.txt")
    );
    assert!(
        super::mutation::ExclusionReason::CheckRecreatedPathObserved
            .label()
            .contains("the check recreated this path while running (observed)")
    );
}

#[test]
fn failed_move_and_recreated_input_get_distinct_grouped_reasons() {
    let recreated = "data/recreated.txt".to_string();
    let move_failed = "data/unmoved.txt".to_string();
    let observed_recreation = super::mutation::ExclusionReason::CheckRecreatedPathObserved.label();
    let diagnostics = describe_candidates_with_run_state(
        &[
            super::mutation::CandidateDiagnostic {
                path: move_failed.clone(),
                exclusion: None,
            },
            super::mutation::CandidateDiagnostic {
                path: recreated.clone(),
                exclusion: None,
            },
        ],
        &std::collections::BTreeSet::from([move_failed.clone(), recreated.clone()]),
        &[recreated.clone()],
        &std::collections::BTreeSet::from([recreated.clone()]),
        &std::collections::BTreeSet::from([recreated.clone()]),
        true,
    );

    assert!(diagnostics.contains(&format!(
        "the host could not move this input aside: {move_failed}"
    )));
    assert!(diagnostics.contains(&format!("{observed_recreation}: {recreated}")));
    assert_eq!(diagnostics.matches(observed_recreation).count(), 1);
    assert!(!diagnostics.contains(&format!("{observed_recreation}: {move_failed}")));
}

#[test]
fn recreated_input_with_operational_failure_is_host_unproven() {
    let markers = super::mutation::Markers::new();
    let inputs = vec!["data/state.txt".to_string(), "data/other.txt".to_string()];
    let result = CheckResult {
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
        None,
        "check 'AC-1': it passed",
    )
    .expect("operational failure remains host-unproven");
    let MutationFinding::HostUnproven(host) = finding else {
        panic!("host failure with a recreated input must remain host-unproven");
    };
    assert!(host.contains("data/state.txt=original-recreated"));
    assert!(host.contains("data/other.txt=restored"));
    assert!(host.contains("the host could not complete the run"));
}

fn run_real_wrapper(command: &str) -> (CheckEvidence, super::mutation::Markers, Vec<String>) {
    run_real_wrapper_with_inputs(command, vec!["data/state.txt".to_string()])
}

fn run_real_wrapper_with_inputs(
    command: &str,
    inputs: Vec<String>,
) -> (CheckEvidence, super::mutation::Markers, Vec<String>) {
    use std::io::Write;

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
    let mut child = std::process::Command::new(archon_shell::resolve_posix_shell())
        .arg("-s")
        .current_dir(project.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(wrapped.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let evidence = CheckEvidence {
        result: CheckResult {
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
fn real_wrapper_recreation_routes_to_author_with_named_path() {
    let (result, markers, inputs) = run_real_wrapper("printf recreated > data/state.txt; exit 0");

    let Some(MutationFinding::AuthorFinding(finding)) = mutation_host_or_author_finding(
        &result,
        &markers,
        &inputs,
        None,
        "check 'AC-1': it passed",
    ) else {
        panic!("real wrapper recreation must route to an author finding");
    };
    assert!(
        finding.contains("it recreated data/state.txt while running"),
        "finding={finding}; states={:?}; markers={:?}; stderr={}",
        markers.states(&result.result, &inputs),
        markers.recreated(&result.result, &inputs),
        String::from_utf8_lossy(&result.result.stderr),
    );
    assert!(!markers.restored_all(&result.result));
}

#[test]
fn real_wrapper_transient_recreation_routes_to_author_with_all_recreated_paths() {
    let (result, markers, inputs) = run_real_wrapper_with_inputs(
        "printf recreated > data/state.txt; rm data/state.txt; exit 0",
        vec![
            "data/state.txt".to_string(),
            "other/untouched.txt".to_string(),
        ],
    );

    let Some(MutationFinding::AuthorFinding(finding)) = mutation_host_or_author_finding(
        &result,
        &markers,
        &inputs,
        None,
        "check 'AC-1': it passed",
    ) else {
        panic!("transient recreation must route to an author finding");
    };
    assert!(
        finding.contains("it recreated data/state.txt while running"),
        "finding={finding}; states={:?}; markers={:?}; stderr={}",
        markers.states(&result.result, &inputs),
        markers.recreated(&result.result, &inputs),
        String::from_utf8_lossy(&result.result.stderr),
    );
    assert!(!finding.contains("other/untouched.txt"));
    assert_eq!(markers.recreated(&result.result, &inputs).len(), 1);
    assert!(
        markers
            .recreated(&result.result, &inputs)
            .contains("data/state.txt")
    );
    assert!(
        !markers
            .recreated(&result.result, &inputs)
            .contains("other/untouched.txt")
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
        panic!("observed signal must route to host-unproven");
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
        panic!("missing aside must route to host-unproven");
    };
    assert!(host.contains("data/state.txt=aside-missing"));
}

#[test]
fn incomplete_restore_explains_every_path_and_exit_signal() {
    let states = std::collections::BTreeMap::from([
        ("data/state.txt".to_string(), "aside-missing".to_string()),
        ("data/tree".to_string(), "restored".to_string()),
    ]);
    let reason = format!(
        "{}; check status/signal: {}",
        describe_states(&states),
        describe_status(Some("nonce:143:15:TERM".into()))
    );
    assert!(reason.contains("data/state.txt=aside-missing"));
    assert!(reason.contains("data/tree=restored"));
    assert!(reason.contains("exit status 143, wrapper-observed signal TERM"));
}

#[test]
fn timeout_diagnostics_keep_known_states_and_report_unavailable_inputs() {
    let markers = super::mutation::Markers::new();
    let inputs = vec!["data/state.txt".to_string(), "data/other.txt".to_string()];
    let result = CheckResult {
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
        operational_error: Some(archon_workflow::acceptance_scratch::CHECK_TIMED_OUT.into()),
        environment_note: None,
    };

    let states = input_states(&result, &markers, &inputs);
    let report = format!(
        "{}; status/signal: {}",
        describe_states(&states),
        describe_status(markers.status(&result))
    );
    assert!(report.contains("data/state.txt=aside-missing"));
    assert!(report.contains("data/other.txt=unreported"));
    assert!(report.contains("exit status 143, wrapper-observed signal TERM"));
}

#[test]
fn operational_error_mutation_diagnostics_keep_input_states_and_exit_signal() {
    let markers = super::mutation::Markers::new();
    let inputs = vec!["data/state.txt".to_string(), "data/other.txt".to_string()];
    let result = CheckResult {
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
    let mut child = std::process::Command::new(archon_shell::resolve_posix_shell())
        .arg("-s")
        .current_dir(project.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(wrapped.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let result = CheckResult {
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

    assert!(
        markers
            .recreated(&result, &names)
            .contains("data/state.txt")
    );
    assert_eq!(
        states.get("data/state.txt").map(String::as_str),
        Some("restored")
    );
    assert!(recreated_paths(&states).is_empty());
}

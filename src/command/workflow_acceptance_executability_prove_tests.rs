use super::*;

#[test]
fn persisted_output_evidence_uses_the_shared_excerpt_limits() {
    let source = include_str!("workflow_acceptance_executability_prove.rs");
    assert!(
        source.contains("crate::command::workflow_task_set::passability::evidence::STDOUT_BYTES")
    );
    assert!(
        source.contains("crate::command::workflow_task_set::passability::evidence::STDERR_BYTES")
    );
    assert!(!source.contains("const STDOUT_BYTES: usize"));
    assert!(!source.contains("const STDERR_BYTES: usize"));
}

#[test]
fn persisted_output_evidence_is_bounded_utf8_safe_and_redacted() {
    let secret = format!("sk-proj-{}", "a".repeat(40));
    let result = archon_workflow::acceptance_scratch::CheckResult {
        acceptance_id: "AC-EVIDENCE".into(),
        exit_code: Some(1),
        quota_walk_count: 0,
        stdout: format!("{}\n{secret}", "界".repeat(196)).into_bytes(),
        stderr: vec![0xff; 3_000],
        environment_note: None,
        operational_error: None,
        classification: None,
    };
    let evidence = output_evidence(&result);
    assert!(!evidence.contains(&secret));
    assert!(evidence.contains("***REDACTED***"));
    assert!(evidence.is_char_boundary(evidence.len()));
    assert!(evidence.len() < 2_700, "{evidence}");
}

#[test]
fn a_long_secret_across_the_excerpt_budget_is_fully_redacted_and_the_tail_kept() {
    // The secret starts inside the stderr budget and runs far past it; the
    // final line after it is what the excerpt must keep.
    let secret = format!("sk-proj-{}", "b".repeat(900));
    let stderr = format!("{}\n{secret}\nfinal assertion failed", "x".repeat(1_900));
    let result = archon_workflow::acceptance_scratch::CheckResult {
        acceptance_id: "AC-EVIDENCE".into(),
        exit_code: Some(1),
        quota_walk_count: 0,
        stdout: Vec::new(),
        stderr: stderr.into_bytes(),
        environment_note: None,
        operational_error: None,
        classification: None,
    };
    let evidence = output_evidence(&result);
    assert!(!evidence.contains("sk-proj-bbbb"), "{evidence}");
    assert!(!evidence.contains(&"b".repeat(64)), "{evidence}");
    assert!(evidence.contains("final assertion failed"), "{evidence}");
}

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
        move_path: None,
        move_root: None,
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
    assert!(finding.contains("the check must fail whenever the deciding test fails"));
    let recreated = ["output/state.txt".to_string()];
    let diagnostics = describe_candidates_with_run_state(
        &[super::mutation::CandidateDiagnostic {
            path: recreated[0].clone(),
            move_path: None,
            move_root: None,
            exclusion: None,
        }],
        &std::collections::BTreeSet::new(),
        &[],
        &std::collections::BTreeSet::new(),
        &std::collections::BTreeSet::from(recreated.clone()),
        &std::collections::BTreeMap::new(),
        true,
    );
    assert!(diagnostics.contains("parent directory metadata changed"));
    assert!(
        super::mutation::ExclusionReason::CheckRecreatedPathObserved
            .label()
            .contains("the check recreated a moved input while it ran")
    );
}

#[test]
fn baseline_passing_check_with_only_unattributed_reappearing_path_is_rejected() {
    let finding =
        super::unattributed_recreation_finding(&baseline_pass_finding("AC-X", "0123456789ab"));
    assert!(finding.contains("parent directory metadata changed"));
    assert!(finding.contains("does not identify who wrote the input"));
    assert!(finding.contains("does not prove that the check depends on the moved input"));
    assert!(!finding.contains("accepted as a regression guard"));
}

#[cfg(unix)]
#[tokio::test]
async fn prove_routes_a_passing_unknown_writer_reappearance_to_an_author_finding() {
    let trees = super::super::probe_tests::trees(&[(
        "AC-UNKNOWN-WRITER",
        "test -f data/state.txt",
        archon_workflow::task_set_contract::TrustedCwd::ProjectRoot,
    )]);
    let contract = trees.contract();
    let probe = super::super::HostProbe::for_task_set(trees.set.project.path(), &trees.set.tasks);
    let digest = super::super::contract_digest(&contract).unwrap();
    let ids = std::collections::BTreeSet::from(["AC-UNKNOWN-WRITER".to_string()]);
    let refs = super::super::refs_for(&contract, &digest, &ids);
    assert_eq!(refs.len(), 1, "accepted command reference is present");
    let baseline = super::Baseline {
        commit: trees.base.clone(),
        repository: trees.repo.clone(),
    };
    let markers = super::mutation::Markers::new();
    let result = archon_workflow::acceptance_scratch::CheckResult {
        classification: None,
        acceptance_id: "AC-UNKNOWN-WRITER".into(),
        exit_code: Some(0),
        quota_walk_count: 0,
        stdout: format!(
            "archon-mutation-state-{}: 0:original-recreated\narchon-mutation-status-{}: nonce:1:0:none\n",
            markers.nonce(), markers.nonce()
        )
        .into_bytes(),
        stderr: format!("{}0\n", markers.moved_prefix()).into_bytes(),
        operational_error: None,
        environment_note: None,
    };
    let evidence = super::hermetic::CheckEvidence {
        result,
        copy_layout: None,
    };
    let observed = super::repairs::TreeRun::known(&[evidence], "already observed");
    assert!(observed.results.contains_key("AC-UNKNOWN-WRITER"));
    probe.inject_prove_run(observed, markers);
    let findings = super::prove(
        &probe,
        &baseline,
        &contract,
        &digest,
        &refs,
        &["AC-UNKNOWN-WRITER".into()],
        &std::collections::BTreeMap::new(),
    )
    .await;
    assert!(
        findings
            .get("AC-UNKNOWN-WRITER")
            .is_some_and(|finding| finding.contains("writer is unknown")),
        "{findings:?}; unproven={:?}",
        super::super::ExecutabilityProbe::take_unproven(&probe)
    );
}

#[test]
fn undiscovered_input_finding_uses_only_generic_path_repair_text() {
    let finding = baseline_pass_finding_without_discovered_input("check-id", "0123456789ab");
    assert!(finding.contains("the command names no existing file path under the selected root"));
    assert!(finding.contains("A test, target or module name is not a file path"));
    assert!(finding.contains("Put the relative path of the file that decides the criterion literally in the command text, so that moving that file aside makes the check fail"));
    assert!(!finding.contains("data the check writes itself is never evidence"));
    for implementation_name in ["cargo", "rust", "python", "node", "javascript", "provider"] {
        assert!(!finding.to_ascii_lowercase().contains(implementation_name));
    }
}

#[test]
fn each_move_state_has_a_distinct_candidate_diagnostic() {
    let recreated = "data/recreated.txt".to_string();
    let absent = "data/absent.txt".to_string();
    let symlink = "data/link/file.txt".to_string();
    let move_failed = "data/unmoved.txt".to_string();
    let observed_recreation = super::mutation::ExclusionReason::CheckRecreatedPathObserved.label();
    let diagnostics = describe_candidates_with_run_state(
        &[
            super::mutation::CandidateDiagnostic {
                path: absent.clone(),
                move_path: None,
                move_root: None,
                exclusion: Some(super::mutation::ExclusionReason::MissingAtRoot),
            },
            super::mutation::CandidateDiagnostic {
                path: symlink.clone(),
                move_path: None,
                move_root: None,
                exclusion: None,
            },
            super::mutation::CandidateDiagnostic {
                path: move_failed.clone(),
                move_path: None,
                move_root: None,
                exclusion: None,
            },
            super::mutation::CandidateDiagnostic {
                path: recreated.clone(),
                move_path: None,
                move_root: None,
                exclusion: None,
            },
        ],
        &std::collections::BTreeSet::from([move_failed.clone(), recreated.clone()]),
        &[],
        &std::collections::BTreeSet::from([recreated.clone()]),
        &std::collections::BTreeSet::from([recreated.clone()]),
        &std::collections::BTreeMap::from([
            (absent.clone(), "absent at the selected root".into()),
            (symlink.clone(), "refused: symlink component".into()),
            (move_failed.clone(), "move failed: permission denied".into()),
        ]),
        true,
    );

    assert!(diagnostics.contains(&format!("absent at the selected root: {absent}")));
    assert!(diagnostics.contains(&format!("refused: symlink component: {symlink}")));
    assert!(diagnostics.contains(&format!("move failed: permission denied: {move_failed}")));
    assert!(diagnostics.contains(&format!(
        "present again after mutation; writer unknown: {recreated}"
    )));
    assert_eq!(diagnostics.matches(observed_recreation).count(), 0);
    assert!(!diagnostics.contains(&format!("{observed_recreation}: {move_failed}")));
}

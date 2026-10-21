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
    assert!(
        finding.contains(
            "the check must still fail on its own assertion when that input is moved aside"
        )
    );
    let recreated = ["output/state.txt".to_string()];
    let diagnostics = describe_candidates_with_run_state(
        &[super::mutation::CandidateDiagnostic {
            path: recreated[0].clone(),
            move_path: None,
            move_root: None,
            exclusion: None,
        }],
        &std::collections::BTreeSet::new(),
        &recreated,
        &std::collections::BTreeSet::new(),
        &std::collections::BTreeSet::from(recreated.clone()),
        &std::collections::BTreeMap::new(),
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
        &[recreated.clone()],
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
    assert!(diagnostics.contains(&format!("{observed_recreation}: {recreated}")));
    assert_eq!(diagnostics.matches(observed_recreation).count(), 1);
    assert!(!diagnostics.contains(&format!("{observed_recreation}: {move_failed}")));
}

use super::*;

#[test]
fn setup_file_that_was_not_moved_has_no_site_copy_note() {
    let candidate = super::mutation::CandidateDiagnostic {
        path: "../project/setup.toml".into(),
        move_path: Some("/site/project/setup.toml".into()),
        move_root: Some("/site/project".into()),
        exclusion: None,
    };
    let diagnostics = describe_candidates_with_run_state(
        &[candidate],
        &std::collections::BTreeSet::new(),
        &[],
        &std::collections::BTreeSet::new(),
        &std::collections::BTreeSet::new(),
        &std::collections::BTreeMap::new(),
        &std::collections::BTreeMap::new(),
        true,
    );
    assert!(diagnostics.contains("../project/setup.toml"));
    assert!(!diagnostics.contains("moved in the site copy"));
}

/// Review of #542: the whole stderr is redacted before its last line is
/// picked, so a credential spanning lines never leaves its tail behind.
#[test]
fn last_stderr_line_masks_a_multi_line_credential_whole() {
    crate::command::workflow_task_set::passability::test_secret(
        "REVIEW_TOKEN",
        "review-prefix\nreview-secret-tail",
    );
    let markers = super::mutation::Markers::new();
    let result = CheckResult {
        verdict_streams: Default::default(),
        fault: None,
        classification: None,
        acceptance_id: "AC-1".into(),
        exit_code: Some(1),
        quota_walk_count: 0,
        stdout: b"review-prefix\nreview-secret-tail\n".to_vec(),
        stderr: b"started\nreview-prefix\nreview-secret-tail\n".to_vec(),
        operational_error: None,
        environment_note: None,
    };
    let line = super::last_stderr_line(&result, &markers, &host_redactor());
    assert!(!line.contains("review-secret-tail"), "{line}");
    assert!(line.contains("REDACTED"), "{line}");
    let observation = super::mutation_observation(&result, &markers, &[], &host_redactor());
    assert!(!observation.contains("review-secret-tail"), "{observation}");
}

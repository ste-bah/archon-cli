#[test]
fn project_artifact_context_has_no_workflow_specific_default_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let v2_root = temp.path().join("project/.archon/workflows/wf-generic/v2");
    std::fs::create_dir_all(&v2_root).expect("v2 root");

    let context = crate::project_artifact_context_from_v2_root(&v2_root);

    assert!(
        context
            .artifact_roots
            .iter()
            .all(|root| !root.contains("trading-lab")),
        "artifact roots must come from workflow context or declared requirements"
    );
    assert_eq!(
        context.branch_evidence_root.as_deref(),
        Some(v2_root.join("branches").to_string_lossy().as_ref())
    );
}

#[test]
fn accepted_branch_proof_is_discoverable_under_explicit_evidence_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let v2_root = temp.path().join("project/.archon/workflows/wf-proof/v2");
    let proof = v2_root
        .join("branches/verification-wave-1")
        .join("verification-wave-1-verify-TASK-003-provider-proof.json");
    std::fs::create_dir_all(proof.parent().expect("proof parent")).expect("proof dir");
    std::fs::write(&proof, r#"{"status":"accepted"}"#).expect("proof");

    let context = crate::project_artifact_context_from_v2_root(&v2_root);
    let evidence_root = std::path::Path::new(
        context
            .branch_evidence_root
            .as_deref()
            .expect("branch evidence root"),
    );

    assert!(
        evidence_root
            .join("verification-wave-1")
            .join("verification-wave-1-verify-TASK-003-provider-proof.json")
            .exists()
    );
}

/// The run's own directory is host bookkeeping, and advertising it as a place
/// to put an artifact is what let a branch write a record of its own among the
/// host's. Only the artifact area inside it is offered.
#[test]
fn the_bare_run_directory_is_not_advertised_as_an_artifact_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let v2_root = temp.path().join("project/.archon/workflows/run-1/v2");
    std::fs::create_dir_all(&v2_root).expect("v2 root");

    let context = crate::project_artifact_context_from_v2_root(&v2_root);

    assert!(
        !context
            .artifact_roots
            .iter()
            .any(|root| root == ".archon/workflows/run-1"),
        "the run directory must not be offered as writable: {:?}",
        context.artifact_roots
    );
    assert!(
        context
            .artifact_roots
            .iter()
            .any(|root| root == ".archon/workflows/run-1/artifacts"),
        "the run's artifact area is still the run-scoped place for a deliverable: {:?}",
        context.artifact_roots
    );
}

/// The advertisement was also what kept a declared path under the run
/// directory out of the repository target set. Withdrawing it must not promote
/// the host's records to code a branch can be told to edit.
#[test]
fn a_declared_path_inside_the_run_store_is_still_not_a_repository_target() {
    let roots = vec![".archon/artifacts".to_string()];
    for path in [
        ".archon/workflows/run-1/state.json",
        ".archon/workflows/run-1/v2/branches/b/rec.json",
        ".archon/workflows/run-1/artifacts/out.json",
    ] {
        assert_eq!(
            crate::v2::contract_code_targets::admissible_repository_path(path, &roots),
            None,
            "{path} is not repository code"
        );
    }
    assert_eq!(
        crate::v2::contract_code_targets::admissible_repository_path(
            "crates/thing/src/lib.rs",
            &roots
        ),
        Some("crates/thing/src/lib.rs".to_string())
    );
}

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

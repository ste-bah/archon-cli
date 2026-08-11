//! Templated artifact paths are excluded from literal evidence checks (D76) and
//! fail closed rather than passing silently (D3, superseding the second half of
//! D76 — see `note_templated_project_artifact`).

use archon_workflow::{
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2ProjectArtifactContext, WorkflowV2Result,
    WorkflowV2Status, normalize_project_artifact_files,
};

fn context_for(root: &std::path::Path) -> WorkflowV2ProjectArtifactContext {
    WorkflowV2ProjectArtifactContext {
        project_root: Some(root.display().to_string()),
        run_id: Some("wf-test".to_string()),
        artifact_roots: vec![".archon/lab-data/".to_string()],
        branch_evidence_root: None,
        policy_version: None,
        ..Default::default()
    }
}

fn accepted_result() -> WorkflowV2Result {
    WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        summary: "test result".to_string(),
        ..Default::default()
    }
}

/// A declared artifact whose path still carries `<...>` names no file. It is
/// kept out of the literal checks (D76) *and* refused as evidence: prior-run
/// finding F4 is exactly an artifact reported present against a wildcard path.
#[test]
fn templated_artifact_path_is_excluded_and_fails_closed() {
    let root = std::env::temp_dir().join("archon-d76-templated");
    std::fs::create_dir_all(&root).unwrap();
    let mut result = accepted_result();
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "refactor complete",
    ));
    result.artifacts.push(archon_workflow::WorkflowV2Artifact {
        id: "artifact-templated".to_string(),
        path: ".archon/lab-data/datasets/<dataset-id>/<version>/manifest.json".to_string(),
        description: Some("declared contract".to_string()),
    });
    normalize_project_artifact_files("item-1", &mut result, &context_for(&root)).unwrap();

    assert!(
        !result
            .residual_gaps
            .iter()
            .any(|gap| gap.id.starts_with("missing_project_artifact_")),
        "a template is not a concrete path that is merely absent: {:?}",
        result.residual_gaps
    );
    let gap = result
        .residual_gaps
        .iter()
        .find(|gap| gap.id.starts_with("unexpanded_artifact_template_"))
        .expect("an unexpanded template raises its own gap");
    assert!(
        gap.description.contains("unexpanded template placeholder"),
        "the gap names what is wrong: {}",
        gap.description
    );
    assert_eq!(
        result.status,
        WorkflowV2Status::NeedsReview,
        "an unexpanded <...> must never leave a result accepted"
    );
    assert!(
        result.evidence.iter().any(|entry| entry
            .summary
            .contains("templated artifact requirement excluded")),
        "expected a typed exclusion note"
    );
    assert!(
        result.artifacts.is_empty(),
        "templated path must not remain as artifact evidence"
    );
}

#[test]
fn literal_missing_artifact_still_fails_closed() {
    let root = std::env::temp_dir().join("archon-d76-missing");
    std::fs::create_dir_all(&root).unwrap();
    let mut result = accepted_result();
    result.artifacts.push(archon_workflow::WorkflowV2Artifact {
        id: "artifact-literal".to_string(),
        path: ".archon/lab-data/datasets/real-id/v1/manifest.json".to_string(),
        description: None,
    });
    normalize_project_artifact_files("item-2", &mut result, &context_for(&root)).unwrap();

    assert!(
        result
            .residual_gaps
            .iter()
            .any(|gap| gap.id.starts_with("missing_project_artifact_")),
        "a literal nonexistent artifact path must still produce a missing gap"
    );
    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
}

#[test]
fn templated_artifact_path_cannot_escape_project_root() {
    let root = std::env::temp_dir().join("archon-d76-templated-traversal");
    std::fs::create_dir_all(&root).unwrap();
    let mut result = accepted_result();
    result.artifacts.push(archon_workflow::WorkflowV2Artifact {
        id: "artifact-traversal".to_string(),
        path: ".archon/lab-data/<dataset-id>/../../../../outside.json".to_string(),
        description: None,
    });

    assert!(
        normalize_project_artifact_files("item-traversal", &mut result, &context_for(&root))
            .is_err(),
        "a placeholder must not bypass target traversal checks"
    );
}

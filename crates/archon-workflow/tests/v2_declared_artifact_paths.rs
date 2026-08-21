//! Issue #168: a value that was never a path must not be used as one, and a
//! path that exists must not be mistaken for evidence.
//!
//! The prose fixtures are the verbatim acceptance criteria that run
//! `wf-67dd2599` turned into directories in the project root.

use archon_workflow::v2::artifact_path_guard::project_root_path_litter;
use archon_workflow::{
    WorkflowV2AgentAdapter, WorkflowV2AgentRequest, WorkflowV2Artifact, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2FileRecord, WorkflowV2HostCall, WorkflowV2HostMethod,
    WorkflowV2HostOptions, WorkflowV2ProjectArtifactContext, WorkflowV2Result, WorkflowV2Status,
    WorkflowV2WriteMode, normalize_project_artifact_files,
};

/// Verbatim from issue #168; the `/` produced the nested tree.
const NESTED_CRITERION: &str = "A gap-audit report or equivalent reducer evidence that cites \
     inspected source files and separates repo implementation gaps from environment/readiness \
     blockers";

const TEMPLATED_ARTIFACT_PATH: &str =
    "${PROJECT_ROOT}/.archon/trading-lab/data/datasets/${DATASET_ID}/${VERSION}/validation.json";

fn context_for(root: &std::path::Path) -> WorkflowV2ProjectArtifactContext {
    WorkflowV2ProjectArtifactContext {
        project_root: Some(root.display().to_string()),
        run_id: Some("wf-168".to_string()),
        artifact_roots: vec![".archon/lab-data".to_string()],
        branch_evidence_root: None,
        policy_version: None,
        ..Default::default()
    }
}

fn accepted_with_declared_artifact(path: &str) -> WorkflowV2Result {
    let mut result = WorkflowV2Result {
        status: WorkflowV2Status::Accepted,
        summary: "declared artifact written".to_string(),
        ..Default::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "wrote the declared artifact",
    ));
    result.artifacts.push(WorkflowV2Artifact {
        id: "declared".to_string(),
        path: path.to_string(),
        description: Some("declared contract".to_string()),
    });
    result
}

/// The hole issue #168 predicted: an existence check that stats a path and
/// finds a directory would pass a contract that nothing satisfies.
#[test]
fn a_directory_does_not_satisfy_a_declared_artifact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let declared = ".archon/lab-data/gap-audit.json";
    std::fs::create_dir_all(root.join(declared)).expect("directory in the artifact's place");
    assert!(root.join(declared).exists(), "the naive check would pass");

    let mut result = accepted_with_declared_artifact(declared);
    normalize_project_artifact_files("item-1", &mut result, &context_for(root)).unwrap();

    assert_eq!(
        result.status,
        WorkflowV2Status::NeedsReview,
        "a directory must never leave a declared artifact accepted"
    );
    assert!(result.artifacts.is_empty(), "{:?}", result.artifacts);
    let gap = result
        .residual_gaps
        .iter()
        .find(|gap| gap.id.starts_with("missing_project_artifact_"))
        .expect("a directory in the artifact's place raises a gap");
    assert!(
        gap.description
            .contains("is a directory, not the declared file"),
        "the gap must name what is actually there: {}",
        gap.description
    );
}

#[test]
fn an_empty_file_does_not_satisfy_a_declared_artifact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let declared = ".archon/lab-data/gap-audit.json";
    std::fs::create_dir_all(root.join(".archon/lab-data")).expect("artifact dir");
    std::fs::write(root.join(declared), "").expect("empty artifact");

    let mut result = accepted_with_declared_artifact(declared);
    normalize_project_artifact_files("item-1", &mut result, &context_for(root)).unwrap();

    assert_eq!(result.status, WorkflowV2Status::NeedsReview);
    let gap = result
        .residual_gaps
        .iter()
        .find(|gap| gap.id.starts_with("missing_project_artifact_"))
        .expect("an empty file in the artifact's place raises a gap");
    assert!(
        gap.description.contains("is an empty file"),
        "{}",
        gap.description
    );
}

#[test]
fn a_written_artifact_still_satisfies_the_contract() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path();
    let declared = ".archon/lab-data/gap-audit.json";
    std::fs::create_dir_all(root.join(".archon/lab-data")).expect("artifact dir");
    std::fs::write(root.join(declared), "{\"gaps\":[]}").expect("artifact");

    let mut result = accepted_with_declared_artifact(declared);
    normalize_project_artifact_files("item-1", &mut result, &context_for(root)).unwrap();

    assert_eq!(result.status, WorkflowV2Status::Accepted);
    assert_eq!(result.artifacts[0].path, declared);
    assert!(
        result.residual_gaps.is_empty(),
        "{:?}",
        result.residual_gaps
    );
}

/// The declared-artifact contract handed to an agent: a criterion must never
/// reach the "Resolved Project Artifact Paths" section, and must never be
/// accepted as satisfied.
#[test]
fn an_acceptance_criterion_declared_as_a_required_artifact_is_refused() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("project");
    let repo = temp.path().join("repo");
    let v2_root = project.join(".archon/workflows/wf-168/v2");
    std::fs::create_dir_all(repo.join("src")).expect("repo");
    std::fs::create_dir_all(&v2_root).expect("v2");

    let mut options = WorkflowV2HostOptions::default();
    options
        .required_artifacts
        .push(archon_workflow::WorkflowV2ArtifactRequirement::new(
            NESTED_CRITERION,
        ));
    let request = WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "impl-criterion-as-path".to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options,
        },
        role: "coder".to_string(),
        task: "write the declared artifact".to_string(),
        constraints: Vec::new(),
        input: serde_json::Value::Null,
        repository_root: Some(repo.display().to_string()),
        project_artifacts: archon_workflow::project_artifact_context_from_v2_root(&v2_root),
        target_files: vec!["src/lib.rs".to_string()],
        target_ownership_scopes: Vec::new(),
    };
    let mut result = WorkflowV2Result::accepted("implementation complete");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "updated implementation source",
    ));
    result
        .files_changed
        .push(WorkflowV2FileRecord::new("src/lib.rs"));

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &serde_json::to_string(&result).expect("json"))
        .expect("a refused declared path is a failed result value, not a hard error");

    assert_eq!(
        parsed.status,
        WorkflowV2Status::Failed,
        "a criterion declared as a deliverable must fail the call"
    );
    assert!(
        parsed
            .residual_gaps
            .iter()
            .any(|gap| gap.description.contains("refused")
                && gap.description.contains("reads as prose")),
        "the gap must say the value is prose: {:?}",
        parsed.residual_gaps
    );
    assert!(
        parsed
            .artifacts
            .iter()
            .all(|artifact| !artifact.path.contains("gap-audit report")),
        "the criterion must never be recorded as an artifact path: {:?}",
        parsed.artifacts
    );
    // Refusal, not creation: nothing on disk was derived from the sentence.
    assert!(
        project_root_path_litter(&project).is_empty(),
        "validating a declared path must not create anything"
    );
}

/// `${PROJECT_ROOT}` with nothing to bind it is an error. An empty expansion
/// would silently turn an absolute path into a relative one.
#[test]
fn an_unset_project_root_template_is_an_error_not_an_empty_expansion() {
    use archon_workflow::v2::artifact_path_guard::{
        ArtifactPathRejection, expand_project_root_template,
    };

    assert_eq!(
        expand_project_root_template(TEMPLATED_ARTIFACT_PATH, None),
        Err(ArtifactPathRejection::UnboundTemplateVariable {
            name: "PROJECT_ROOT".to_string()
        })
    );
    // Even bound, the remaining variables are named rather than left literal in
    // a path someone would then create.
    assert_eq!(
        expand_project_root_template(TEMPLATED_ARTIFACT_PATH, Some("/repo")),
        Err(ArtifactPathRejection::UnboundTemplateVariable {
            name: "DATASET_ID".to_string()
        })
    );
}

/// A deliverable contract carrying `${...}` cannot be verified by anything in
/// this engine, so its verification command must fail closed naming the token
/// rather than probe a path built from an unexpanded template.
#[test]
fn a_shell_templated_deliverable_contract_fails_closed() {
    let contract = serde_json::json!({
        "artifact_path": TEMPLATED_ARTIFACT_PATH,
        "min_instances": 2,
    });

    let command =
        archon_workflow::v2::deliverable_contract::verification_command("/repo", &contract);

    assert!(
        command.starts_with("printf"),
        "must be the fail-closed form: {command}"
    );
    assert!(command.contains("exit 1"), "{command}");
    assert!(
        command.contains("${PROJECT_ROOT}"),
        "the refusal must name the token: {command}"
    );
    assert!(
        archon_workflow::v2::deliverable_contract::typed_verification_command("/repo", &contract)
            .is_none(),
        "a typed verifier must not be handed a templated path"
    );
}

/// The script option that let a sentence become a path in the first place.
#[test]
fn a_prose_required_artifact_is_rejected_at_script_parse() {
    let options = serde_json::json!({ "requiredArtifacts": [NESTED_CRITERION] });

    let error = archon_workflow::v2::script::parse_script_options(&options)
        .expect_err("a criterion declared as requiredArtifacts is a script defect");

    let message = error.to_string();
    assert!(
        message.contains("requiredArtifacts entry is not a path"),
        "{message}"
    );
    assert!(message.contains("reads as prose"), "{message}");
}

#[test]
fn real_required_artifacts_still_parse() {
    let options = serde_json::json!({
        "requiredArtifacts": [
            ".archon/artifacts/gap-audit.json",
            { "path": "artifacts/report.md", "kind": "report" }
        ]
    });

    let (parsed, _) =
        archon_workflow::v2::script::parse_script_options(&options).expect("real paths parse");

    assert_eq!(parsed.required_artifacts.len(), 2);
    assert_eq!(
        parsed.required_artifacts[0].path,
        ".archon/artifacts/gap-audit.json"
    );
    assert_eq!(parsed.required_artifacts[1].kind.as_deref(), Some("report"));
}
